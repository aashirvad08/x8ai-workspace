use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher as _};
use x8ai_core::workspace::WorkspaceEvent;

use crate::files::relative;
use crate::{Error, Workspace};

/// Changes are collected until the filesystem has been quiet this long.
const QUIET: Duration = Duration::from_millis(100);
/// A continuous stream of changes is still reported at least this often.
const MAX_DELAY: Duration = Duration::from_millis(500);
/// Past this many paths in one batch, report a rescan instead of every path.
const MAX_PATHS: usize = 1000;

/// Watches the workspace for changes until dropped.
pub struct Watcher {
    _watcher: notify::RecommendedWatcher,
}

impl Workspace {
    /// Reports changes under the root through `on_event`, batched so that bursts
    /// (a `git checkout`, a build) arrive as a few events. Uses the platform's
    /// notification API (FSEvents on macOS, inotify on Linux); nothing is polled.
    pub fn watch(
        &self,
        on_event: impl Fn(WorkspaceEvent) + Send + 'static,
    ) -> Result<Watcher, Error> {
        let root = self.root().to_owned();
        let shown = root.display().to_string();
        let watch_error = |e: notify::Error| Error::Io {
            path: shown.clone(),
            detail: format!("cannot watch for changes: {e}"),
        };
        let (events, received) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(events).map_err(watch_error)?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(watch_error)?;
        thread::Builder::new()
            .name("workspace-watch".into())
            .spawn(move || batch(&received, &root, &on_event))
            .map_err(|e| Error::io(&shown, e))?;
        Ok(Watcher { _watcher: watcher })
    }
}

/// Runs until the watcher is dropped, which closes the channel.
fn batch(
    received: &mpsc::Receiver<notify::Result<notify::Event>>,
    root: &Path,
    on_event: &dyn Fn(WorkspaceEvent),
) {
    while let Ok(first) = received.recv() {
        let mut changes = Changes::default();
        changes.add(first, root);
        let deadline = Instant::now() + MAX_DELAY;
        let mut closed = false;
        while Instant::now() < deadline {
            match received.recv_timeout(QUIET) {
                Ok(next) => changes.add(next, root),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        if let Some(event) = changes.into_event() {
            on_event(event);
        }
        if closed {
            return;
        }
    }
}

#[derive(Default)]
struct Changes {
    paths: BTreeSet<String>,
    rescan: bool,
}

impl Changes {
    fn add(&mut self, event: notify::Result<notify::Event>, root: &Path) {
        match event {
            Ok(event) => {
                self.rescan |= event.need_rescan();
                self.paths
                    .extend(event.paths.iter().filter_map(|p| relative(root, p)));
            }
            // An error from the watcher means events may have been lost.
            Err(_) => self.rescan = true,
        }
    }

    fn into_event(self) -> Option<WorkspaceEvent> {
        if self.rescan || self.paths.len() > MAX_PATHS {
            Some(WorkspaceEvent::Rescan)
        } else if self.paths.is_empty() {
            None
        } else {
            Some(WorkspaceEvent::Changed {
                paths: self.paths.into_iter().collect(),
            })
        }
    }
}
