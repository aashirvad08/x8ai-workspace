//! The Welcome screen's command line: what is typed, the commands, and what
//! fits what is typed so far. The same commands and matching as the app's
//! (`src/home/home.ts`), except that a folder is typed, not chosen in a picker.

use std::path::{Path, PathBuf};

use unicode_width::UnicodeWidthStr;
use x8ai_core::workspace::RecentWorkspace;

/// Longest name the welcome greets.
pub const MAX_NAME_LENGTH: usize = 40;

/// Most suggestions shown at once.
const MAX_SUGGESTIONS: usize = 8;

/// Most entries read from one folder while completing a path.
const MAX_LISTED: usize = 2_000;

pub struct CommandInfo {
    pub name: &'static str,
    pub usage: &'static str,
    pub description: &'static str,
}

pub const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "/cd",
        usage: "/cd <folder>",
        description: "open a folder as your space",
    },
    CommandInfo {
        name: "/new",
        usage: "/new <name>",
        description: "a new, empty space in ~/Workspaces",
    },
    CommandInfo {
        name: "/home",
        usage: "/home",
        description: "the workspace with no folder open",
    },
    CommandInfo {
        name: "/name",
        usage: "/name <your name>",
        description: "how the welcome greets you",
    },
    CommandInfo {
        name: "/quit",
        usage: "/quit",
        description: "leave x8ai",
    },
];

/// The app's commands that the terminal version does not have yet.
const APP_ONLY: &[&str] = &["/share", "/get", "/give"];

#[derive(Debug, PartialEq, Eq)]
pub enum Command<'a> {
    Empty,
    Cd(&'a str),
    New(&'a str),
    Home,
    Name(&'a str),
    Quit,
    AppOnly(&'a str),
    Unknown(&'a str),
}

pub fn parse(text: &str) -> Command<'_> {
    let text = text.trim();
    if text.is_empty() {
        return Command::Empty;
    }
    let (word, arg) = match text.find(char::is_whitespace) {
        Some(at) => (&text[..at], text[at..].trim()),
        None => (text, ""),
    };
    match word {
        "/cd" => Command::Cd(arg),
        "/new" => Command::New(arg),
        "/home" => Command::Home,
        "/name" => Command::Name(arg),
        "/quit" | "/exit" => Command::Quit,
        _ if APP_ONLY.contains(&word) => Command::AppOnly(word),
        _ => Command::Unknown(word),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    /// What the line becomes when the suggestion is taken.
    pub completion: String,
}

/// Where the suggestions come from: the recent spaces, the open one, and where
/// relative folders and `~` are.
pub struct Context<'a> {
    pub recent: &'a [RecentWorkspace],
    pub open: Option<&'a Path>,
    pub cwd: &'a Path,
    pub home: &'a Path,
}

/// What fits what is typed so far: the other recent spaces on an empty line,
/// commands after `/`, and recent spaces and folders after `/cd `.
pub fn suggestions(text: &str, context: &Context<'_>) -> Vec<Suggestion> {
    if text.is_empty() {
        return context
            .recent
            .iter()
            .filter(|r| r.available && Some(Path::new(&r.root)) != context.open)
            .take(MAX_SUGGESTIONS)
            .map(|r| recent_suggestion(r, context.home))
            .collect();
    }
    if !text.starts_with('/') {
        return Vec::new();
    }
    let Some(at) = text.find(char::is_whitespace) else {
        return COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(text))
            .map(|c| Suggestion {
                label: c.usage.to_owned(),
                detail: c.description.to_owned(),
                completion: if c.usage.contains('<') {
                    format!("{} ", c.name)
                } else {
                    c.name.to_owned()
                },
            })
            .collect();
    };
    if &text[..at] != "/cd" {
        return Vec::new();
    }
    let query = text[at..].trim_start();
    let wanted = query.to_lowercase();
    let wanted = wanted
        .strip_prefix("~/")
        .map_or(wanted.clone(), |rest| format!("/{rest}"));
    let mut found: Vec<Suggestion> = context
        .recent
        .iter()
        .filter(|r| r.available && (wanted.is_empty() || r.root.to_lowercase().contains(&wanted)))
        .take(MAX_SUGGESTIONS)
        .map(|r| recent_suggestion(r, context.home))
        .collect();
    if looks_like_a_path(query) {
        let listed = |path: &Path| context.recent.iter().any(|r| Path::new(&r.root) == path);
        for (path, folder) in folders(query, context) {
            if found.len() == MAX_SUGGESTIONS {
                break;
            }
            if !listed(&path) {
                found.push(folder);
            }
        }
    }
    found
}

fn recent_suggestion(recent: &RecentWorkspace, home: &Path) -> Suggestion {
    Suggestion {
        label: recent.name.clone(),
        detail: tilde(Path::new(&recent.root), home),
        completion: format!("/cd {}", recent.root),
    }
}

/// `~/code`, `./app`, `/Users/me`, `src/`: a path, not a space's name.
fn looks_like_a_path(query: &str) -> bool {
    query.starts_with(['/', '~', '.']) || query.contains('/')
}

/// The folders whose path starts with `query`, as `/cd` completions that keep
/// what was typed (`~/co` → `~/code/`), each with its path.
fn folders(query: &str, context: &Context<'_>) -> Vec<(PathBuf, Suggestion)> {
    let (typed_dir, prefix) = match query.rfind('/') {
        Some(at) => (&query[..=at], &query[at + 1..]),
        None => ("", query),
    };
    let dir = if typed_dir.is_empty() {
        // `~` alone: inside the home folder.
        if query == "~" {
            let suggestion = Suggestion {
                label: "~/".to_owned(),
                detail: context.home.display().to_string(),
                completion: "/cd ~/".to_owned(),
            };
            return vec![(context.home.to_owned(), suggestion)];
        }
        context.cwd.to_owned()
    } else {
        resolve(typed_dir, context.cwd, context.home)
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .take(MAX_LISTED)
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| {
            name.starts_with(prefix) && (prefix.starts_with('.') || !name.starts_with('.'))
        })
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|name| {
            let path = dir.join(&name);
            let suggestion = Suggestion {
                label: format!("{name}/"),
                detail: tilde(&path, context.home),
                completion: format!("/cd {typed_dir}{name}/"),
            };
            (path, suggestion)
        })
        .collect()
}

/// The recent spaces `/cd <arg>` means: the exact folder, or every one whose
/// path ends with `arg` (`~/` taken as a prefix of the home folder), or else
/// every one whose name starts with it. `/cd gymRL` and `/cd gym` match
/// `/Users/me/gymRL`.
pub fn match_recent<'a>(arg: &str, recent: &'a [RecentWorkspace]) -> Vec<&'a RecentWorkspace> {
    let wanted = arg.trim().trim_end_matches('/');
    if wanted.is_empty() || wanted == "~" {
        return Vec::new();
    }
    let available = recent.iter().filter(|r| r.available);
    if wanted.starts_with('/') {
        return available.filter(|r| r.root == wanted).collect();
    }
    let relative = wanted.strip_prefix("~/").unwrap_or(wanted);
    let suffix = format!("/{relative}");
    let by_path: Vec<&RecentWorkspace> = available
        .clone()
        .filter(|r| r.root.ends_with(&suffix))
        .collect();
    if !by_path.is_empty() || relative.contains('/') {
        return by_path;
    }
    let prefix = relative.to_lowercase();
    available
        .filter(|r| r.name.to_lowercase().starts_with(&prefix))
        .collect()
}

/// A typed folder: `~` is the home folder, and a relative path is relative to
/// where `x8ai` was started, as `cd` would take it.
pub fn resolve(typed: &str, cwd: &Path, home: &Path) -> PathBuf {
    match typed.strip_prefix('~') {
        Some("") => home.to_owned(),
        Some(rest) if rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ if typed.starts_with('/') => PathBuf::from(typed),
        _ => cwd.join(typed),
    }
}

/// `path` with the home folder shown as `~`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// The command line: its text and where the caret is.
#[derive(Debug, Default)]
pub struct Line {
    text: String,
    /// A byte offset at a character boundary.
    caret: usize,
}

impl Line {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Display columns before the caret.
    pub fn caret_column(&self) -> usize {
        self.text[..self.caret].width()
    }

    pub fn set(&mut self, text: &str) {
        self.text = text.to_owned();
        self.caret = self.text.len();
    }

    pub fn take(&mut self) -> String {
        self.caret = 0;
        std::mem::take(&mut self.text)
    }

    /// Inserts typed or pasted text; line breaks and control characters are
    /// dropped, since the line is one command.
    pub fn insert(&mut self, text: &str) {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        self.text.insert_str(self.caret, &clean);
        self.caret += clean.len();
    }

    pub fn backspace(&mut self) {
        if let Some(c) = self.text[..self.caret].chars().next_back() {
            self.caret -= c.len_utf8();
            self.text.remove(self.caret);
        }
    }

    pub fn delete(&mut self) {
        if self.caret < self.text.len() {
            self.text.remove(self.caret);
        }
    }

    /// Ctrl+W: the word before the caret, and the spaces after it.
    pub fn delete_word(&mut self) {
        let before = &self.text[..self.caret];
        let trimmed = before.trim_end();
        let start = trimmed
            .rfind(|c: char| c.is_whitespace() || c == '/')
            .map_or(0, |at| at + 1);
        let start = if start == trimmed.len() && start > 0 {
            start - 1
        } else {
            start
        };
        self.text.replace_range(start..self.caret, "");
        self.caret = start;
    }

    pub fn left(&mut self) {
        if let Some(c) = self.text[..self.caret].chars().next_back() {
            self.caret -= c.len_utf8();
        }
    }

    pub fn right(&mut self) {
        if let Some(c) = self.text[self.caret..].chars().next() {
            self.caret += c.len_utf8();
        }
    }

    pub fn home(&mut self) {
        self.caret = 0;
    }

    pub fn end(&mut self) {
        self.caret = self.text.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recent(root: &str) -> RecentWorkspace {
        RecentWorkspace {
            id: None,
            root: root.to_owned(),
            name: root.rsplit('/').next().unwrap().to_owned(),
            available: true,
        }
    }

    #[test]
    fn commands_are_parsed_with_their_argument() {
        assert_eq!(parse("  "), Command::Empty);
        assert_eq!(parse("/cd  ~/code/app "), Command::Cd("~/code/app"));
        assert_eq!(parse("/cd"), Command::Cd(""));
        assert_eq!(parse("/new demo app"), Command::New("demo app"));
        assert_eq!(parse("/home"), Command::Home);
        assert_eq!(parse("/name Ada"), Command::Name("Ada"));
        assert_eq!(parse("/quit"), Command::Quit);
        assert_eq!(parse("/share ws-abcdef"), Command::AppOnly("/share"));
        assert_eq!(parse("ls -la"), Command::Unknown("ls"));
    }

    #[test]
    fn recent_spaces_match_by_path_or_name() {
        let list = [
            recent("/Users/me/gymRL"),
            recent("/Users/me/code/app"),
            recent("/Users/me/other/app"),
        ];
        let roots = |arg| {
            match_recent(arg, &list)
                .into_iter()
                .map(|r| r.root.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(roots("gymRL"), ["/Users/me/gymRL"]);
        assert_eq!(roots("gym"), ["/Users/me/gymRL"]);
        assert_eq!(roots("~/gymRL/"), ["/Users/me/gymRL"]);
        assert_eq!(roots("/Users/me/gymRL"), ["/Users/me/gymRL"]);
        assert_eq!(roots("code/app"), ["/Users/me/code/app"]);
        assert_eq!(roots("app"), ["/Users/me/code/app", "/Users/me/other/app"]);
        // A path that matches no recent space is not matched by name.
        assert!(roots("nothing/gym").is_empty());
        assert!(roots("~").is_empty());
        assert!(roots("").is_empty());
    }

    #[test]
    fn missing_folders_are_not_matched() {
        let mut gone = recent("/Users/me/gone");
        gone.available = false;
        assert!(match_recent("gone", &[gone]).is_empty());
    }

    #[test]
    fn typed_folders_resolve_like_cd() {
        let (cwd, home) = (Path::new("/work"), Path::new("/Users/me"));
        assert_eq!(resolve("~", cwd, home), Path::new("/Users/me"));
        assert_eq!(resolve("~/code", cwd, home), Path::new("/Users/me/code"));
        assert_eq!(resolve("/tmp", cwd, home), Path::new("/tmp"));
        assert_eq!(resolve("app", cwd, home), Path::new("/work/app"));
        assert_eq!(resolve("./app", cwd, home), Path::new("/work/./app"));
    }

    #[test]
    fn home_is_shown_as_tilde() {
        let home = Path::new("/Users/me");
        assert_eq!(tilde(Path::new("/Users/me"), home), "~");
        assert_eq!(tilde(Path::new("/Users/me/code"), home), "~/code");
        assert_eq!(tilde(Path::new("/tmp"), home), "/tmp");
    }

    #[test]
    fn suggestions_fit_what_is_typed() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        std::fs::create_dir_all(home.join("code/app")).unwrap();
        std::fs::create_dir_all(home.join("code/api")).unwrap();
        std::fs::create_dir_all(home.join("code/.git")).unwrap();
        std::fs::write(home.join("code/apple.txt"), "").unwrap();
        let open = home.join("code/app");
        let list = [
            recent(&open.display().to_string()),
            recent("/elsewhere/gymRL"),
        ];
        let context = Context {
            recent: &list,
            open: Some(&open),
            cwd: home,
            home,
        };
        let completions = |text| {
            suggestions(text, &context)
                .into_iter()
                .map(|s| s.completion)
                .collect::<Vec<_>>()
        };
        // An empty line: the other recent spaces.
        assert_eq!(completions(""), ["/cd /elsewhere/gymRL"]);
        // Commands, with a space after those that take an argument.
        assert_eq!(
            completions("/"),
            ["/cd ", "/new ", "/home", "/name ", "/quit"]
        );
        assert_eq!(completions("/h"), ["/home"]);
        // Recent spaces by path, then folders for a path, not files or hidden
        // ones, and not a folder already listed as a recent space.
        let app = format!("/cd {}", open.display());
        assert_eq!(completions("/cd gym"), ["/cd /elsewhere/gymRL"]);
        assert_eq!(
            completions("/cd ~/code/ap"),
            [app.as_str(), "/cd ~/code/api/"]
        );
        assert_eq!(completions("/cd code/"), [app.as_str(), "/cd code/api/"]);
        assert_eq!(completions("/cd ~/code/."), ["/cd ~/code/.git/"]);
        assert_eq!(completions("ls"), Vec::<String>::new());
    }

    #[test]
    fn the_line_edits_by_character() {
        let mut line = Line::default();
        line.insert("/cd ~/cöde");
        line.backspace();
        line.insert("e\nx");
        assert_eq!(line.text(), "/cd ~/cödex");
        line.left();
        line.left();
        assert_eq!(line.caret_column(), 9);
        line.delete();
        assert_eq!(line.text(), "/cd ~/cödx");
        line.end();
        line.delete_word();
        assert_eq!(line.text(), "/cd ~/");
        line.delete_word();
        assert_eq!(line.text(), "/cd ~");
        line.delete_word();
        assert_eq!(line.text(), "/cd ");
        line.delete_word();
        assert_eq!(line.text(), "/");
        assert_eq!(line.take(), "/");
        assert!(line.is_empty());
    }
}
