//! Boxes that ask for something: forms (a few fields of text, a secret, or a
//! choice) and pickers (one of a list, or several). What a dialog is for is
//! the app's business (`app/panels.rs`); this is how it is typed into.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use x8ai_core::id::IntegrationId;
use x8ai_core::model::ModelSelection;

use crate::welcome::Line;

/// What a form is for: what happens when it is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormFor {
    /// A provider's API key.
    ProviderKey(String),
    /// A model id the user knows a provider serves.
    AddModel(String),
    McpAdd,
    McpEdit(String),
    /// The value of an MCP server's secret variable.
    McpSecret {
        server: String,
        variable: String,
    },
    SkillAdd,
    SkillEdit(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    /// Typed but never shown.
    Secret,
    /// One of these, changed with ←→.
    Choice(Vec<&'static str>, usize),
}

#[derive(Debug)]
pub struct Field {
    pub label: &'static str,
    pub kind: FieldKind,
    pub line: Line,
    /// Shown under the field while it has the keys.
    pub hint: &'static str,
}

impl Field {
    pub fn text(label: &'static str, value: &str, hint: &'static str) -> Self {
        let mut line = Line::default();
        line.set(value);
        Self {
            label,
            kind: FieldKind::Text,
            line,
            hint,
        }
    }

    pub fn secret(label: &'static str, hint: &'static str) -> Self {
        Self {
            label,
            kind: FieldKind::Secret,
            line: Line::default(),
            hint,
        }
    }

    pub fn choice(
        label: &'static str,
        options: Vec<&'static str>,
        chosen: usize,
        hint: &'static str,
    ) -> Self {
        Self {
            label,
            kind: FieldKind::Choice(options, chosen),
            line: Line::default(),
            hint,
        }
    }

    /// The text typed, or the option chosen.
    pub fn value(&self) -> &str {
        match &self.kind {
            FieldKind::Choice(options, chosen) => options[*chosen],
            _ => self.line.text(),
        }
    }

    pub fn chosen(&self) -> usize {
        match &self.kind {
            FieldKind::Choice(_, chosen) => *chosen,
            _ => 0,
        }
    }
}

#[derive(Debug)]
pub struct Form {
    pub title: String,
    /// Said above the fields.
    pub note: Option<String>,
    pub fields: Vec<Field>,
    pub focus: usize,
    /// Why the last send was refused; the form stays open.
    pub error: Option<String>,
    pub purpose: FormFor,
}

/// What a key did to a dialog.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Cancelled,
    Sent,
}

impl Form {
    pub fn new(title: impl Into<String>, purpose: FormFor, fields: Vec<Field>) -> Self {
        Self {
            title: title.into(),
            note: None,
            fields,
            focus: 0,
            error: None,
            purpose,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let last = self.fields.len().saturating_sub(1);
        let field = &mut self.fields[self.focus];
        match key.code {
            KeyCode::Esc => return Outcome::Cancelled,
            KeyCode::Char('c') if ctrl => return Outcome::Cancelled,
            KeyCode::Enter => return Outcome::Sent,
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1).min(last),
            KeyCode::BackTab | KeyCode::Up => self.focus = self.focus.saturating_sub(1),
            KeyCode::Left | KeyCode::Right if matches!(field.kind, FieldKind::Choice(..)) => {
                if let FieldKind::Choice(options, chosen) = &mut field.kind {
                    let count = options.len();
                    *chosen = if key.code == KeyCode::Right {
                        (*chosen + 1) % count
                    } else {
                        (*chosen + count - 1) % count
                    };
                }
            }
            _ if matches!(field.kind, FieldKind::Choice(..)) => {}
            KeyCode::Left => field.line.left(),
            KeyCode::Right => field.line.right(),
            KeyCode::Home => field.line.home(),
            KeyCode::End => field.line.end(),
            KeyCode::Char('a') if ctrl => field.line.home(),
            KeyCode::Char('e') if ctrl => field.line.end(),
            KeyCode::Char('u') if ctrl => {
                field.line.take();
            }
            KeyCode::Char('w') if ctrl => field.line.delete_word(),
            KeyCode::Backspace => field.line.backspace(),
            KeyCode::Delete => field.line.delete(),
            KeyCode::Char(c) if !ctrl => field.line.insert(c.encode_utf8(&mut [0; 4])),
            _ => {}
        }
        self.error = None;
        Outcome::Open
    }

    /// Pasted text goes into the focused text field.
    pub fn paste(&mut self, text: &str) {
        let field = &mut self.fields[self.focus];
        if !matches!(field.kind, FieldKind::Choice(..)) {
            field.line.insert(text);
        }
    }

    pub fn value(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map_or("", Field::value)
    }

    pub fn chosen(&self, label: &str) -> usize {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map_or(0, Field::chosen)
    }
}

/// What the next launch of an agent gets, chosen in a picker or from the
/// Models, MCP and Catalog panels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Use {
    Model(ModelSelection),
    Mcp(IntegrationId),
    Skill(IntegrationId),
}

/// What a picker is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickFor {
    /// The model of an agent's next launch (one).
    Model(String),
    /// The session MCP servers of an agent's next launch (several).
    Mcp(String),
    /// The session skills of an agent's next launch (several).
    Skills(String),
    /// Which agent's next launch gets this.
    Agent(Use),
    /// Which secret variable of a server to save.
    Secret(String),
    /// Which space gets this space's add-ons.
    Share,
}

#[derive(Debug, Clone)]
pub struct PickOption {
    pub value: String,
    pub label: String,
    pub detail: String,
    pub chosen: bool,
}

#[derive(Debug)]
pub struct Picker {
    pub title: String,
    pub options: Vec<PickOption>,
    /// Several can be chosen (space ticks), or one (Enter takes it).
    pub multi: bool,
    pub selected: usize,
    pub purpose: PickFor,
}

impl Picker {
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let count = self.options.len();
        match key.code {
            KeyCode::Esc => return Outcome::Cancelled,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Outcome::Cancelled;
            }
            KeyCode::Enter if count > 0 => return Outcome::Sent,
            KeyCode::Up | KeyCode::Char('k') if count > 0 => {
                self.selected = (self.selected + count - 1) % count;
            }
            KeyCode::Down | KeyCode::Char('j') if count > 0 => {
                self.selected = (self.selected + 1) % count;
            }
            KeyCode::Char(' ') if self.multi && count > 0 => {
                let option = &mut self.options[self.selected];
                option.chosen = !option.chosen;
            }
            _ => {}
        }
        Outcome::Open
    }

    /// The value taken: the selected one (one), or every one ticked (several).
    pub fn values(&self) -> Vec<String> {
        if self.multi {
            self.options
                .iter()
                .filter(|o| o.chosen)
                .map(|o| o.value.clone())
                .collect()
        } else {
            self.options
                .get(self.selected)
                .map(|o| vec![o.value.clone()])
                .unwrap_or_default()
        }
    }
}

/// Arguments as typed: split at spaces, with `"…"` or `'…'` keeping spaces
/// in one argument. Never run by a shell.
pub fn split_args(text: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in text.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => {
                current.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err("A quote is not closed.".to_owned());
    }
    if started {
        args.push(current);
    }
    Ok(args)
}

/// Arguments as they are typed back into a form.
pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.is_empty() || a.contains(char::is_whitespace) {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_split_at_spaces_outside_quotes() {
        assert_eq!(split_args("-y  @scope/pkg").unwrap(), ["-y", "@scope/pkg"]);
        assert_eq!(
            split_args(r#"--dir "/My Folder" 'a b' """#).unwrap(),
            ["--dir", "/My Folder", "a b", ""]
        );
        assert!(split_args("\"open").is_err());
        let args = vec!["--dir".to_owned(), "/My Folder".to_owned()];
        assert_eq!(split_args(&join_args(&args)).unwrap(), args);
    }

    #[test]
    fn a_form_moves_between_fields_and_cycles_choices() {
        let mut form = Form::new(
            "t",
            FormFor::McpAdd,
            vec![
                Field::text("Name", "", ""),
                Field::choice("Transport", vec!["stdio", "http"], 0, ""),
            ],
        );
        for c in "gh".chars() {
            form.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        form.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        form.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(form.value("Name"), "gh");
        assert_eq!(form.value("Transport"), "http");
        assert_eq!(
            form.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Outcome::Sent
        );
    }
}
