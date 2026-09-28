use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError, mpsc};
use std::thread;

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder};
use x8ai_core::workspace::{SearchMatch, SearchQuery, SearchSummary};

use crate::files::{relative, walker};
use crate::{Error, Workspace};

/// Result limits. Keep a search's output bounded, however large the workspace.
#[derive(Debug, Clone, Copy)]
pub struct SearchLimits {
    pub max_matches: u32,
    pub max_matches_per_file: u32,
    pub max_file_bytes: u64,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_matches: 5_000,
            max_matches_per_file: 200,
            max_file_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Longest preview sent for one line, in characters. Longer lines are shortened
/// around the first match.
const MAX_PREVIEW_CHARS: usize = 240;
/// Characters of context kept before the first match when a line is shortened.
const PREVIEW_LEAD_CHARS: usize = 60;

impl Workspace {
    /// Searches every file in the workspace for `query.text`, matched literally.
    ///
    /// Walks the same files as quick open: `.gitignore` is respected, `.git` and
    /// well-known dependency and build directories are skipped, symlinks are not
    /// followed. Each file is opened through the workspace's `cap-std` handle, so a
    /// file replaced by a symlink during the walk still cannot lead outside. Binary
    /// files (a NUL byte) and files that are not UTF-8 are skipped.
    ///
    /// Files are searched on several threads, because opening and reading many
    /// small files is dominated by system calls. `on_file` is called on the
    /// calling thread with each file's matches as they are found, in no particular
    /// order. `cancel` stops the search at the next file or line.
    pub fn search(
        &self,
        query: &SearchQuery,
        limits: SearchLimits,
        cancel: &AtomicBool,
        mut on_file: impl FnMut(String, Vec<SearchMatch>),
    ) -> Result<SearchSummary, Error> {
        let mut summary = SearchSummary::default();
        if query.text.is_empty() {
            return Ok(summary);
        }
        let matcher = RegexMatcherBuilder::new()
            .fixed_strings(true)
            .case_insensitive(!query.case_sensitive)
            .build(&query.text)
            .map_err(|e| Error::InvalidQuery(e.to_string()))?;

        // Set when the search is cancelled or has found enough.
        let stop = AtomicBool::new(false);
        let stopped = || stop.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed);
        let (paths_tx, paths_rx) = mpsc::sync_channel::<String>(PATH_QUEUE);
        let paths_rx = Mutex::new(paths_rx);
        let (found_tx, found_rx) = mpsc::channel::<(String, Vec<SearchMatch>)>();

        thread::scope(|scope| {
            scope.spawn(|| {
                for entry in walker(self.root(), Some(limits.max_file_bytes))
                    .build()
                    .flatten()
                {
                    if stopped() {
                        break;
                    }
                    if !entry.file_type().is_some_and(|t| t.is_file()) {
                        continue;
                    }
                    let Some(path) = relative(self.root(), entry.path()) else {
                        continue;
                    };
                    if paths_tx.send(path).is_err() {
                        break;
                    }
                }
                // Dropping the sender tells the workers the walk is over.
                drop(paths_tx);
            });
            for _ in 0..workers() {
                let (matcher, found_tx, paths_rx, stopped) =
                    (matcher.clone(), found_tx.clone(), &paths_rx, &stopped);
                scope.spawn(move || {
                    let mut searcher = SearcherBuilder::new()
                        .binary_detection(BinaryDetection::quit(b'\0'))
                        .line_number(true)
                        .build();
                    loop {
                        let next = paths_rx
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .recv();
                        let Ok(path) = next else { break };
                        // Once stopped, keep taking paths so the walker never blocks.
                        if stopped() {
                            continue;
                        }
                        let matches =
                            self.search_file(&mut searcher, &matcher, &path, limits, stopped);
                        if !matches.is_empty() && found_tx.send((path, matches)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(found_tx);

            for (path, mut matches) in found_rx {
                if stopped() {
                    continue;
                }
                let room = (limits.max_matches - summary.matches) as usize;
                if matches.len() >= room {
                    matches.truncate(room);
                    summary.truncated = true;
                    stop.store(true, Ordering::Relaxed);
                    if matches.is_empty() {
                        continue;
                    }
                }
                if matches.len() as u32 >= limits.max_matches_per_file {
                    summary.truncated = true;
                }
                summary.files += 1;
                summary.matches += matches.len() as u32;
                on_file(path, matches);
            }
        });
        summary.cancelled = cancel.load(Ordering::Relaxed);
        Ok(summary)
    }

    /// One file's matches, at most `max_matches_per_file`. Unreadable and
    /// non-UTF-8 files have none, as in ripgrep.
    fn search_file(
        &self,
        searcher: &mut Searcher,
        matcher: &RegexMatcher,
        path: &str,
        limits: SearchLimits,
        stopped: &dyn Fn() -> bool,
    ) -> Vec<SearchMatch> {
        // Opened beneath the root, never by the absolute path the walker saw.
        let Ok(file) = self.dir.open(path) else {
            return Vec::new();
        };
        let mut matches = Vec::new();
        let result = searcher.search_file(
            matcher,
            &file.into_std(),
            UTF8(|line_number, line| {
                if stopped() {
                    return Ok(false);
                }
                let line = line.trim_end_matches(['\n', '\r']);
                let mut spans = Vec::new();
                matcher.find_iter(line.as_bytes(), |m| {
                    spans.push((m.start(), m.end()));
                    true
                })?;
                if let Some(found) = to_match(line_number, line, &spans) {
                    matches.push(found);
                }
                Ok((matches.len() as u32) < limits.max_matches_per_file)
            }),
        );
        if result.is_err() { Vec::new() } else { matches }
    }
}

/// Paths queued for the searching threads. Keeps the walk only slightly ahead.
const PATH_QUEUE: usize = 1024;

/// Searching threads: enough to overlap system calls, not so many that a search
/// takes over the machine.
fn workers() -> usize {
    thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8)
}

/// A line's match, with columns and preview ranges in UTF-16 units.
fn to_match(line_number: u64, line: &str, spans: &[(usize, usize)]) -> Option<SearchMatch> {
    let &(first_start, first_end) = spans.first()?;
    let (preview, offset) = shorten(line, first_start);
    let utf16 = |text: &str| text.encode_utf16().count() as u32;
    let ranges = spans
        .iter()
        .filter(|&&(start, end)| start >= offset && end <= offset + preview.len())
        .map(|&(start, end)| (utf16(&line[offset..start]), utf16(&line[offset..end])))
        .collect();
    Some(SearchMatch {
        line: u32::try_from(line_number).unwrap_or(u32::MAX),
        column: utf16(&line[..first_start]),
        length: utf16(&line[first_start..first_end]),
        preview: preview.to_owned(),
        ranges,
    })
}

/// A preview of `line` that shows the match at byte `start`, and the byte offset
/// the preview begins at. Cuts only on character boundaries.
fn shorten(line: &str, start: usize) -> (&str, usize) {
    if line.chars().count() <= MAX_PREVIEW_CHARS {
        return (line, 0);
    }
    let lead_start = line[..start]
        .char_indices()
        .rev()
        .nth(PREVIEW_LEAD_CHARS - 1)
        .map_or(0, |(i, _)| i);
    let rest = &line[lead_start..];
    let end = rest
        .char_indices()
        .nth(MAX_PREVIEW_CHARS)
        .map_or(rest.len(), |(i, _)| i);
    (&rest[..end], lead_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_are_utf16() {
        let line = "é😀 needle";
        let start = line.find("needle").unwrap();
        let found = to_match(3, line, &[(start, start + 6)]).unwrap();
        // "é" is 1 UTF-16 unit, "😀" is 2, then a space.
        assert_eq!((found.line, found.column, found.length), (3, 4, 6));
        assert_eq!(found.ranges, vec![(4, 10)]);
    }

    #[test]
    fn long_lines_are_shortened_around_the_match() {
        let line = format!("{}needle{}", "a".repeat(1000), "b".repeat(1000));
        let start = 1000;
        let found = to_match(1, &line, &[(start, start + 6)]).unwrap();
        assert!(found.preview.chars().count() <= MAX_PREVIEW_CHARS);
        let (a, b) = found.ranges[0];
        assert_eq!(&found.preview[a as usize..b as usize], "needle");
        assert_eq!(found.column, 1000);
    }
}
