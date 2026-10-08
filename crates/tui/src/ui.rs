//! Drawing: the Welcome screen; a space (its header and tabs, the file list,
//! the open tab's panes, the status bar); the list of keys; and the question
//! asked before ending a running program.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::agents::Isolated;
use crate::app::{App, Dialog, Mode, Question, Screen};
use crate::dialog::{FieldKind, Form, Picker};
use crate::layout::Axis;
use crate::listing::{Item, Tone};
use crate::space::{Focus, Label, PanelLine, PanelRow, Sidebar, SpaceLayout, SpaceView, clip};
use crate::welcome::tilde;

/// The Welcome's column is at most this wide.
const WELCOME_WIDTH: u16 = 76;

/// The smallest window anything is drawn in.
const MIN_SIZE: (u16, u16) = (24, 6);

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    if area.width < MIN_SIZE.0 || area.height < MIN_SIZE.1 {
        let text = Paragraph::new("Make the window larger to use x8ai.").wrap(Wrap { trim: true });
        frame.render_widget(text, area);
        return;
    }
    match app.screen {
        Screen::Welcome => welcome(frame, app),
        Screen::Space => space(frame, app),
    }
    if app.mode == Mode::Help && app.screen == Screen::Space {
        help(frame, app);
    }
    if let Some(dialog) = &app.dialog {
        match dialog {
            Dialog::Form(form) => form_box(frame, app, form),
            Dialog::Picker(picker) => picker_box(frame, app, picker),
        }
    }
    if let Some(question) = &app.question {
        ask(frame, app, question);
    }
}

fn muted() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

// The Welcome screen

fn welcome(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let theme = app.theme;
    let width = area.width.saturating_sub(4).min(WELCOME_WIDTH);
    let x = area.x + (area.width - width) / 2;

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(greeting(app, usize::from(width)));
    if let Some(name) = app.name() {
        lines.push(Line::styled(
            name.to_owned(),
            Style::new().fg(theme.highlight()),
        ));
    }
    lines.push(Line::raw(""));
    let prompt_row = lines.len();
    let prompt = if app.line.is_empty() {
        Line::from(vec![
            Span::styled("› ", Style::new().fg(theme.highlight())),
            Span::styled("Type a command…  \"/cd ~/projects/app\"", muted()),
        ])
    } else {
        Line::from(vec![
            Span::styled("› ", Style::new().fg(theme.highlight())),
            Span::raw(app.line.text().to_owned()),
        ])
    };
    lines.push(prompt);
    lines.push(context(app));
    lines.push(Line::raw(""));

    if !app.suggestions.is_empty() {
        if app.line.is_empty() {
            lines.push(Line::styled("Recent spaces", muted()));
        }
        let label_width = app
            .suggestions
            .iter()
            .map(|s| s.label.width())
            .max()
            .unwrap_or(0)
            .min(28);
        for (i, suggestion) in app.suggestions.iter().enumerate() {
            let selected = app.selected == Some(i);
            let marker = if selected { "› " } else { "  " };
            let label_style = if selected {
                Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            let pad = label_width.saturating_sub(suggestion.label.width());
            lines.push(Line::from(vec![
                Span::styled(marker, Style::new().fg(theme.accent())),
                Span::styled(suggestion.label.clone(), label_style),
                Span::raw(" ".repeat(pad + 2)),
                Span::styled(suggestion.detail.clone(), muted()),
            ]));
        }
        lines.push(Line::raw(""));
    }

    if let Some(message) = &app.message {
        let style = if message.error {
            Style::new().fg(theme.error())
        } else {
            Style::new()
        };
        lines.push(Line::styled(message.text.clone(), style));
        lines.push(Line::raw(""));
    }
    lines.extend(hints(app, width));

    // Wrapped lines take more rows than lines; leave room for them.
    let rows: u16 = lines
        .iter()
        .map(|l| wrapped_rows(l, usize::from(width)))
        .sum();
    // A little above the middle, where the eye starts.
    let top = area.y + area.height.saturating_sub(rows) / 3;
    let column = Rect::new(x, top, width, area.height.saturating_sub(top - area.y));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), column);

    if app.question.is_none() {
        let caret = u16::try_from(app.line.caret_column() + 2).unwrap_or(u16::MAX);
        let row = top + u16::try_from(prompt_row).unwrap_or(0);
        if caret < width && row < area.bottom() {
            frame.set_cursor_position(Position::new(x + caret, row));
        }
    }
}

/// "WELCOME, SIR" in wide-set capitals, SIR in raspberry, and a block cursor,
/// as the app's head has it.
fn greeting(app: &App, width: usize) -> Line<'static> {
    let theme = app.theme;
    let spread = |word: &str| word.chars().map(|c| format!("{c} ")).collect::<String>();
    let (welcome, sir) = if width >= 28 {
        (spread("WELCOME,"), spread("SIR"))
    } else {
        ("WELCOME, ".to_owned(), "SIR ".to_owned())
    };
    Line::from(vec![
        Span::raw(welcome),
        Span::raw(" "),
        Span::styled(
            sir,
            Style::new()
                .fg(theme.highlight())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "█",
            Style::new()
                .fg(theme.highlight())
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ])
}

/// Under the prompt: the space Esc goes back to.
fn context(app: &App) -> Line<'static> {
    let space = &app.current;
    let mut spans = vec![Span::styled("  ", muted())];
    match &space.root {
        Some(root) => {
            spans.push(Span::styled("Space ", muted()));
            spans.push(Span::styled(
                space.name.clone(),
                Style::new().add_modifier(Modifier::BOLD),
            ));
            let mut detail = format!("  {}", tilde(root, app.home()));
            if let Some(id) = &space.id {
                detail.push_str(&format!(" · {id}"));
            }
            detail.push_str(if space.trusted {
                " · trusted"
            } else {
                " · not trusted"
            });
            spans.push(Span::styled(detail, muted()));
        }
        None => {
            spans.push(Span::styled("Home ", muted()));
            spans.push(Span::styled(
                "no folder open",
                Style::new().add_modifier(Modifier::BOLD),
            ));
            if let Some(id) = &space.id {
                spans.push(Span::styled(format!("  {id}"), muted()));
            }
        }
    }
    Line::from(spans)
}

/// The keys to know, wrapped between hints, never inside one.
fn hints(app: &App, width: u16) -> Vec<Line<'static>> {
    const GAP: &str = "  ·  ";
    let back = format!("back to {}", app.current.name);
    let hints = [
        ("/cd", "open a space"),
        ("/new", "new space"),
        ("/home", "no folder"),
        ("esc", back.as_str()),
        ("ctrl-c", "quit"),
    ];
    let mut lines = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for (key, text) in hints {
        let hint = key.width() + 1 + text.width();
        if !spans.is_empty() && used + GAP.width() + hint > usize::from(width) {
            lines.push(Line::from(std::mem::take(&mut spans)));
            used = 0;
        }
        if !spans.is_empty() {
            spans.push(Span::styled(GAP, muted()));
            used += GAP.width();
        }
        spans.push(Span::styled(
            key.to_owned(),
            Style::new().fg(app.theme.accent()),
        ));
        spans.push(Span::styled(format!(" {text}"), muted()));
        used += hint;
    }
    lines.push(Line::from(spans));
    lines
}

// A space

fn space(frame: &mut Frame<'_>, app: &App) {
    let Some(view) = app.view() else {
        return;
    };
    let layout = view.layout(frame.area());
    header(frame, app, view, &layout);
    if let Some(area) = layout.sidebar {
        match view.sidebar {
            Sidebar::Files => file_list(frame, app, view, area),
            Sidebar::Agents => agents_panel(frame, app, view, area),
            sidebar if sidebar.is_list() => list_panel(frame, app, view, area),
            _ => {}
        }
    }

    let theme = app.theme;
    let mut cursor = None;
    for area in &layout.panes.panes {
        let Some(slot) = view.slot(area.id) else {
            continue;
        };
        let focused = view.focused() == Some(area.id) && view.focus == Focus::Panes;
        if let Some(title) = area.title {
            // The title row: the pane's name on a line, which also separates it
            // from the pane above.
            let name_style = if focused {
                Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD)
            } else {
                muted()
            };
            let name = clip(&slot.title(), usize::from(title.width.saturating_sub(4)));
            let fill = usize::from(title.width).saturating_sub(name.width() + 3);
            let line = Line::from(vec![
                Span::styled("─ ", muted()),
                Span::styled(name, name_style),
                Span::styled(format!(" {}", "─".repeat(fill)), muted()),
            ]);
            frame.render_widget(Paragraph::new(line), title);
        }
        let at = slot.pane.render(area.body, frame.buffer_mut(), theme);
        if focused {
            cursor = at;
        }
        if let Some(exit) = slot.pane.exit() {
            let how = match &exit.signal {
                Some(signal) => format!("was ended by {signal}"),
                None => format!("exited with {}", exit.code),
            };
            let what = match slot.kind {
                crate::space::Kind::Shell => "Enter starts a new shell",
                crate::space::Kind::Agent(..) => "Enter runs it again",
                _ => "Enter closes it",
            };
            let bar = Line::from(vec![
                Span::styled(
                    format!(" {} {how}. ", slot.title()),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!("{what} · ctrl-g x closes the pane ")),
            ]);
            let row = Rect::new(
                area.body.x,
                area.body.bottom().saturating_sub(1),
                area.body.width,
                1,
            );
            frame.render_widget(Clear, row);
            frame.render_widget(
                Paragraph::new(bar).style(Style::new().add_modifier(Modifier::REVERSED)),
                row,
            );
        }
    }
    // The lines between panes side by side.
    for divider in &layout.panes.dividers {
        if divider.axis == Axis::Right {
            for y in divider.line.top()..divider.line.bottom() {
                if let Some(cell) = frame.buffer_mut().cell_mut((divider.line.x, y)) {
                    cell.set_symbol("│").set_style(muted());
                }
            }
        }
    }
    frame.render_widget(status_bar(app, view), layout.status);
    if let Some(position) = cursor
        && app.question.is_none()
        && matches!(app.mode, Mode::Normal | Mode::Prefix)
    {
        frame.set_cursor_position(position);
    }
}

/// " x8ai <name>  1 zsh  2 main.rs  +            not trusted"
fn header(frame: &mut Frame<'_>, app: &App, view: &SpaceView, layout: &SpaceLayout) {
    let theme = app.theme;
    let space = &view.info;
    let prefix = Line::from(vec![
        Span::styled(
            " x8ai ",
            Style::new()
                .fg(theme.highlight())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            space.name.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(prefix), layout.header);
    for &(label, rect) in &layout.labels {
        let (text, style) = match label {
            Label::Tab(index) if index == view.active => (
                view.label(index),
                Style::new()
                    .fg(theme.accent())
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            ),
            Label::Tab(index) => (view.label(index), muted()),
            Label::New => (" + ".to_owned(), muted()),
        };
        frame.render_widget(Paragraph::new(Span::styled(text, style)), rect);
    }
    let text = view.right_text(usize::from(layout.header.width / 3));
    let width = u16::try_from(text.width()).unwrap_or(u16::MAX);
    let style = if space.trusted {
        Style::new().fg(theme.ok())
    } else {
        muted()
    };
    let right = Rect {
        x: layout.header.right().saturating_sub(width),
        width,
        ..layout.header
    };
    frame.render_widget(Paragraph::new(Span::styled(text, style)), right);
}

/// The space's folder as a tree, beside the panes.
fn file_list(frame: &mut Frame<'_>, app: &App, view: &SpaceView, area: Rect) {
    let theme = app.theme;
    let Some(files) = &view.files else {
        return;
    };
    let focused = view.focus == Focus::Sidebar;
    let width = usize::from(area.width.saturating_sub(1));
    let name = view.info.name.to_uppercase();
    let title_style = if focused {
        Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let mut lines = vec![Line::styled(clip(&format!(" {name}"), width), title_style)];
    if let Some(error) = files.error() {
        lines.push(Line::styled(
            clip(&format!(" {error}"), width),
            Style::new().fg(theme.error()),
        ));
    } else if files.rows().is_empty() {
        lines.push(Line::styled(" (empty)", muted()));
    }
    let height = usize::from(area.height.saturating_sub(1));
    for (index, row) in files
        .rows()
        .iter()
        .enumerate()
        .skip(files.offset())
        .take(height)
    {
        let marker = match (row.folder, row.open) {
            (true, true) => "▾ ",
            (true, false) => "▸ ",
            (false, _) => "  ",
        };
        let text = format!(" {}{marker}{}", "  ".repeat(row.depth), row.name);
        let mut style = if row.name.starts_with('.') {
            muted()
        } else {
            Style::new()
        };
        if row.folder {
            style = style.add_modifier(Modifier::BOLD);
        }
        if index == files.selected() {
            style = if focused {
                Style::new()
                    .fg(theme.accent())
                    .add_modifier(Modifier::REVERSED)
            } else {
                style.fg(theme.accent())
            };
        }
        let text = clip(&text, width);
        let pad = width.saturating_sub(text.width());
        lines.push(Line::styled(format!("{text}{}", " ".repeat(pad)), style));
    }
    let inner = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };
    frame.render_widget(Paragraph::new(lines), inner);
    for y in area.top()..area.bottom() {
        if let Some(cell) = frame
            .buffer_mut()
            .cell_mut((area.right().saturating_sub(1), y))
        {
            cell.set_symbol("│").set_style(muted());
        }
    }
}

/// The Agents panel: the folder's trust and isolation, the agents, and the
/// space's sessions.
fn agents_panel(frame: &mut Frame<'_>, app: &App, view: &SpaceView, area: Rect) {
    let theme = app.theme;
    let panel = &view.panel;
    let focused = view.focus == Focus::Sidebar;
    let width = usize::from(area.width.saturating_sub(1));
    let title_style = if focused {
        Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let mut lines = vec![Line::styled(" AGENTS", title_style)];
    let height = usize::from(area.height.saturating_sub(1));
    for line in panel.lines().into_iter().skip(panel.offset).take(height) {
        let shown = match line {
            PanelLine::Trust => {
                if view.info.trusted {
                    Line::styled(
                        clip(" ● trusted folder", width),
                        Style::new().fg(theme.ok()),
                    )
                } else {
                    Line::styled(
                        clip(" ○ not trusted: t to trust it", width),
                        Style::new().fg(theme.highlight()),
                    )
                }
            }
            PanelLine::Isolation => {
                let text = match &panel.isolated {
                    Some(Isolated::Worktrees { branch }) => format!(
                        " each session: a worktree from {}",
                        branch.as_deref().unwrap_or("HEAD")
                    ),
                    Some(Isolated::Shared(reason)) => format!(" {reason}: in your files"),
                    None => " …".to_owned(),
                };
                Line::styled(clip(&text, width), muted())
            }
            PanelLine::Blank => Line::raw(""),
            PanelLine::Heading(text) => {
                Line::styled(format!(" {text}"), muted().add_modifier(Modifier::BOLD))
            }
            PanelLine::Empty(text) => Line::styled(clip(&format!("   {text}"), width), muted()),
            PanelLine::Draft(index) => {
                let summary = match &panel.rows[index] {
                    PanelRow::Agent(agent) => panel
                        .drafts
                        .get(agent.definition.id.as_str())
                        .cloned()
                        .unwrap_or_default(),
                    PanelRow::Session(_) => String::new(),
                };
                Line::styled(
                    clip(&format!("   → {summary}"), width),
                    Style::new().fg(theme.accent()),
                )
            }
            PanelLine::Row(index) => {
                let (name, detail, dim) = match &panel.rows[index] {
                    PanelRow::Agent(agent) => {
                        let detail = match (&agent.program, agent.approved) {
                            (Err(reason), _) => reason.clone(),
                            (Ok(_), true) => "allowed".to_owned(),
                            (Ok(_), false) => "installed".to_owned(),
                        };

                        (
                            agent.definition.name.clone(),
                            detail,
                            agent.program.is_err(),
                        )
                    }
                    PanelRow::Session(session) => (
                        session.name.clone(),
                        session_detail(session),
                        session.state != x8ai_agents::SessionState::Running,
                    ),
                };
                let text = format!("   {name}  {detail}");
                let text = clip(&text, width);
                let pad = width.saturating_sub(text.width());
                let mut style = if dim { muted() } else { Style::new() };
                if index == panel.selected {
                    style = if focused {
                        Style::new()
                            .fg(theme.accent())
                            .add_modifier(Modifier::REVERSED)
                    } else {
                        style.fg(theme.accent())
                    };
                }
                Line::styled(format!("{text}{}", " ".repeat(pad)), style)
            }
        };
        lines.push(shown);
    }
    let inner = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };
    frame.render_widget(Paragraph::new(lines), inner);
    for y in area.top()..area.bottom() {
        if let Some(cell) = frame
            .buffer_mut()
            .cell_mut((area.right().saturating_sub(1), y))
        {
            cell.set_symbol("│").set_style(muted());
        }
    }
}

/// A session's state and when it started (its branch's time), short.
fn session_detail(session: &x8ai_agents::AgentSession) -> String {
    use x8ai_agents::SessionState;
    let state = match &session.state {
        SessionState::Running => "running".to_owned(),
        SessionState::NotRunning => "not running".to_owned(),
        SessionState::Exited(exit) if exit.signal.is_some() => "stopped".to_owned(),
        SessionState::Exited(exit) if exit.code == 0 => "ended".to_owned(),
        SessionState::Exited(exit) => format!("exited {}", exit.code),
        SessionState::Failed(_) => "failed".to_owned(),
    };
    let when = session
        .worktree
        .as_ref()
        .and_then(|w| w.branch.rsplit('/').next())
        .and_then(|token| {
            // YYYYMMDD-HHMMSS-xxxxxx, in UTC.
            let time = token.get(9..15)?;
            Some(format!(" · {}:{} UTC", &time[..2], &time[2..4]))
        })
        .unwrap_or_else(|| " · in your folder".to_owned());
    let model = session
        .model
        .as_ref()
        .map_or_else(String::new, |m| format!(" · {}", m.model));
    format!("{state}{when}{model}")
}

fn status_bar(app: &App, view: &SpaceView) -> Paragraph<'static> {
    let theme = app.theme;
    let key = |k: &str| {
        Span::styled(
            k.to_owned(),
            Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD),
        )
    };
    let text = |t: &str| Span::styled(t.to_owned(), muted());
    if let Some(message) = &app.message {
        let style = if message.error {
            Style::new().fg(theme.error())
        } else {
            Style::new()
        };
        return Paragraph::new(Line::styled(format!(" {}", message.text), style));
    }
    let scrolled = view.focused_slot().map_or(0, |s| s.pane.scrolled_back());
    let line = match app.mode {
        Mode::Prefix => Line::from(vec![
            Span::styled(
                " ctrl-g ",
                Style::new()
                    .fg(theme.highlight())
                    .add_modifier(Modifier::BOLD),
            ),
            key("t"),
            Span::raw(" tab  "),
            key("| -"),
            Span::raw(" split  "),
            key("x"),
            Span::raw(" close  "),
            key("←→↑↓"),
            Span::raw(" move  "),
            key("f"),
            Span::raw(" files  "),
            key("h"),
            Span::raw(" Welcome  "),
            key("?"),
            Span::raw(" all keys  "),
            key("esc"),
            Span::raw(" cancel"),
        ]),
        Mode::Scroll => Line::from(vec![
            Span::styled(
                format!(" Scrolled back {scrolled} lines  "),
                Style::new()
                    .fg(theme.highlight())
                    .add_modifier(Modifier::BOLD),
            ),
            key("↑↓"),
            text(" line  "),
            key("PgUp PgDn"),
            text(" page  "),
            key("g G"),
            text(" top, bottom  "),
            key("esc"),
            text(" back"),
        ]),
        _ if view.focus == Focus::Sidebar
            && view.sidebar == Sidebar::Agents
            && matches!(view.panel.row(), Some(PanelRow::Agent(_))) =>
        {
            Line::from(vec![
                text(" "),
                key("enter"),
                text(" launch  "),
                key("m"),
                text(" model  "),
                key("u"),
                text(" MCP  "),
                key("l"),
                text(" skills  "),
                key("t"),
                text(" trust  "),
                key("r"),
                text(" revoke  "),
                key("esc"),
                text(" back"),
            ])
        }
        _ if view.focus == Focus::Sidebar && view.sidebar.is_list() => {
            list_hints(view, &key, &text)
        }
        _ if view.focus == Focus::Sidebar && view.sidebar == Sidebar::Agents => Line::from(vec![
            text(" "),
            key("enter"),
            text(" open, run again  "),
            key("c"),
            text(" changes  "),
            key("o"),
            text(" shell there  "),
            key("s"),
            text(" stop  "),
            key("d"),
            text(" remove  "),
            key("t"),
            text(" trust  "),
            key("esc"),
            text(" back"),
        ]),
        _ if view.focus == Focus::Sidebar => Line::from(vec![
            text(" "),
            key("↑↓"),
            text(" move  "),
            key("enter"),
            text(" open in your editor  "),
            key("←→"),
            text(" close, open folders  "),
            key("esc"),
            text(" back to the terminal"),
        ]),
        _ if scrolled > 0 => Line::from(vec![
            Span::styled(
                format!(" Scrolled back {scrolled} lines"),
                Style::new()
                    .fg(theme.highlight())
                    .add_modifier(Modifier::BOLD),
            ),
            text("  ·  scroll down, or type to go back"),
        ]),
        _ => Line::from(vec![
            text(" "),
            key("ctrl-g"),
            text(" then  "),
            key("t"),
            text(" tab  "),
            key("| -"),
            text(" split  "),
            key("x"),
            text(" close  "),
            key("f"),
            text(" files  "),
            key("h"),
            text(" Welcome  "),
            key("?"),
            text(" all keys"),
        ]),
    };
    Paragraph::new(line)
}

/// Ctrl-g ?: every key, in a box.
fn help(frame: &mut Frame<'_>, app: &App) {
    const KEYS: &[(&str, &str)] = &[
        ("t", "a new tab, with a shell"),
        ("n  p  1-9", "the next, the previous, or that tab"),
        ("|", "split: a new shell to the right"),
        ("-", "split: a new shell below"),
        ("←→↑↓  o", "the pane beside, or the next one"),
        ("z", "the pane alone, or back with the others"),
        ("x", "close the pane"),
        ("f", "the file list: Enter opens a file in $EDITOR"),
        ("a", "agents: launch, review, stop and remove sessions"),
        ("m", "models: API keys, model ids, local models"),
        ("u", "MCP servers: add, change, secrets, on and off"),
        ("k", "the catalog: everything, and your own skills"),
        ("e", "add-ons for this space's terminals"),
        ("s", "scroll back"),
        ("h", "the Welcome screen (the space keeps running)"),
        ("q", "quit"),
        ("ctrl-g", "send ctrl-g to the program"),
    ];
    let theme = app.theme;
    let mut lines = vec![
        Line::styled(
            "Keys, after ctrl-g",
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];
    for (keys, what) in KEYS {
        lines.push(Line::from(vec![
            Span::styled(
                format!("{keys:<11}"),
                Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD),
            ),
            Span::raw((*what).to_owned()),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "The mouse: click a pane or a tab, drag a line between panes, wheel to scroll, drag to select and copy (hold Shift in programs that use the mouse).",
        muted(),
    ));
    lines.push(Line::raw(""));
    lines.push(Line::styled("Any key closes this.", muted()));
    dialog(frame, lines, theme.accent(), 66);
}

// Questions

fn ask(frame: &mut Frame<'_>, app: &App, question: &Question) {
    let mut lines = vec![
        Line::styled(
            question.title.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];
    for line in &question.lines {
        lines.push(Line::raw(line.clone()));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(
            "y",
            Style::new()
                .fg(app.theme.highlight())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {}   ", question.yes)),
        Span::styled(
            "n",
            Style::new()
                .fg(app.theme.accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" cancel"),
    ]));
    dialog(frame, lines, app.theme.highlight(), 72);
}

/// The rows `line` takes when wrapped at word boundaries in `width` columns,
/// as `Paragraph` wraps it: words move to the next row whole, and a word
/// longer than a row is broken.
fn wrapped_rows(line: &Line<'_>, width: usize) -> u16 {
    let width = width.max(1);
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let mut rows = 1;
    let mut used = 0;
    for word in text.split(' ') {
        let len = word.width();
        let needed = if used == 0 { len } else { used + 1 + len };
        if needed <= width {
            used = needed;
        } else if len <= width {
            rows += 1;
            used = len;
        } else {
            // Broken across rows, starting on a new one when this one has text.
            if used > 0 {
                rows += 1;
            }
            rows += (len - 1) / width;
            used = len % width;
            if used == 0 {
                used = width;
            }
        }
    }
    u16::try_from(rows).unwrap_or(u16::MAX)
}

/// The Models, MCP, Catalog or Add-ons panel: a list.
fn list_panel(frame: &mut Frame<'_>, app: &App, view: &SpaceView, area: Rect) {
    let theme = app.theme;
    let list = &view.list;
    let focused = view.focus == Focus::Sidebar;
    let width = usize::from(area.width.saturating_sub(1));
    let title = match view.sidebar {
        Sidebar::Models => " MODELS".to_owned(),
        Sidebar::Mcp => " MCP SERVERS".to_owned(),
        Sidebar::Catalog if list.filtering => format!(" CATALOG  /{}▏", list.filter),
        Sidebar::Catalog => " CATALOG".to_owned(),
        Sidebar::Addons => format!(" ADD-ONS · {}", view.info.name),
        _ => String::new(),
    };
    let title_style = if focused {
        Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    };
    let mut lines = vec![Line::styled(clip(&title, width), title_style)];
    let height = usize::from(area.height.saturating_sub(1));
    let mut row = list.items[..list.offset.min(list.items.len())]
        .iter()
        .filter(|i| matches!(i, Item::Row { .. }))
        .count();
    for item in list.items.iter().skip(list.offset).take(height) {
        lines.push(match item {
            Item::Heading(text) => {
                Line::styled(format!(" {text}"), muted().add_modifier(Modifier::BOLD))
            }
            Item::Note(text) => Line::styled(clip(&format!(" {text}"), width), muted()),
            Item::Blank => Line::raw(""),
            Item::Row {
                label,
                detail,
                tone,
                nested,
                ..
            } => {
                let selected = row == list.selected;
                row += 1;
                let indent = if *nested { "     " } else { "   " };
                let label = format!("{indent}{label}");
                let label_width = label.width();
                let detail_room = width.saturating_sub(label_width + 2);
                let detail = clip(detail, detail_room);
                let pad = width.saturating_sub(label_width + 2 + detail.width());
                let detail_style = match tone {
                    Tone::Good => Style::new().fg(theme.ok()),
                    Tone::Wanting => Style::new().fg(theme.highlight()),
                    Tone::Muted => muted(),
                    Tone::Plain => Style::new(),
                };
                if selected && focused {
                    let text = clip(&format!("{label}  {detail}{}", " ".repeat(pad)), width);
                    Line::styled(
                        text,
                        Style::new()
                            .fg(theme.accent())
                            .add_modifier(Modifier::REVERSED),
                    )
                } else {
                    let label_style = if selected {
                        Style::new().fg(theme.accent())
                    } else if *nested {
                        muted()
                    } else {
                        Style::new()
                    };
                    Line::from(vec![
                        Span::styled(clip(&label, width), label_style),
                        Span::raw("  "),
                        Span::styled(detail, detail_style),
                    ])
                }
            }
        });
    }
    let inner = Rect {
        width: area.width.saturating_sub(1),
        ..area
    };
    frame.render_widget(Paragraph::new(lines), inner);
    for y in area.top()..area.bottom() {
        if let Some(cell) = frame
            .buffer_mut()
            .cell_mut((area.right().saturating_sub(1), y))
        {
            cell.set_symbol("│").set_style(muted());
        }
    }
}

/// The keys of the list panel that has them, for the status bar.
fn list_hints(
    view: &SpaceView,
    key: &dyn Fn(&str) -> Span<'static>,
    text: &dyn Fn(&str) -> Span<'static>,
) -> Line<'static> {
    let pairs: &[(&str, &str)] = match view.sidebar {
        Sidebar::Models => &[
            ("enter", "use a model"),
            ("s", "save a key"),
            ("d", "delete"),
            ("a", "add a model id"),
            ("r", "look for local"),
        ],
        Sidebar::Mcp => &[
            ("n", "new"),
            ("enter", "change"),
            ("space", "on/off"),
            ("s", "secret"),
            ("l", "for the next launch"),
            ("d", "remove"),
        ],
        Sidebar::Catalog if view.list.filtering => {
            &[("enter", "keep the filter"), ("esc", "clear it")]
        }
        Sidebar::Catalog => &[
            ("enter", "use, open"),
            ("/", "filter"),
            ("n", "new skill"),
            ("e", "change"),
            ("d", "remove"),
        ],
        Sidebar::Addons => &[
            ("enter", "add, install"),
            ("d", "remove"),
            ("s", "share with a space"),
            ("r", "check again"),
        ],
        _ => &[],
    };
    let mut spans = vec![text(" ")];
    for (k, t) in pairs {
        spans.push(key(k));
        spans.push(text(&format!(" {t}  ")));
    }
    spans.push(key("esc"));
    spans.push(text(" back"));
    Line::from(spans)
}

/// A form: its fields, the focused one's hint, and why it was refused.
fn form_box(frame: &mut Frame<'_>, app: &App, form: &Form) {
    let theme = app.theme;
    let mut lines = vec![
        Line::styled(
            form.title.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];
    if let Some(note) = &form.note {
        lines.push(Line::styled(note.clone(), muted()));
        lines.push(Line::raw(""));
    }
    let mut caret = None;
    for (index, field) in form.fields.iter().enumerate() {
        let focused = index == form.focus;
        let label_style = if focused {
            Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD)
        } else {
            muted()
        };
        lines.push(Line::styled(field.label.to_owned(), label_style));
        let shown = match &field.kind {
            FieldKind::Text => field.line.text().to_owned(),
            FieldKind::Secret => "•".repeat(field.line.text().chars().count()),
            FieldKind::Choice(options, chosen) => options
                .iter()
                .enumerate()
                .map(|(i, o)| {
                    if i == *chosen {
                        format!("[{o}]")
                    } else {
                        o.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("  "),
        };
        if focused && !matches!(field.kind, FieldKind::Choice(..)) {
            let column = match field.kind {
                FieldKind::Secret => field.line.text()[..]
                    .chars()
                    .count()
                    .min(field.line.caret_column()),
                _ => field.line.caret_column(),
            };
            caret = Some((lines.len(), column + 2));
        }
        lines.push(Line::from(vec![
            Span::styled(
                "› ",
                Style::new().fg(if focused {
                    theme.highlight()
                } else {
                    Color::Reset
                }),
            ),
            Span::raw(shown),
        ]));
        if focused && !field.hint.is_empty() {
            lines.push(Line::styled(field.hint.to_owned(), muted()));
        }
    }
    if let Some(error) = &form.error {
        lines.push(Line::raw(""));
        lines.push(Line::styled(error.clone(), Style::new().fg(theme.error())));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(
            "enter",
            Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" save   "),
        Span::styled(
            "tab",
            Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" next field   "),
        Span::styled(
            "esc",
            Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" cancel"),
    ]));
    let inner = dialog(frame, lines.clone(), theme.accent(), 72);
    if let Some((line, column)) = caret {
        let width = usize::from(inner.width).max(1);
        let row: u16 = lines[..line].iter().map(|l| wrapped_rows(l, width)).sum();
        let x = inner.x + u16::try_from(column).unwrap_or(0);
        let y = inner.y + row;
        if x < inner.right() && y < inner.bottom() {
            frame.set_cursor_position(Position::new(x, y));
        }
    }
}

/// A picker: one of a list, or several ticked.
fn picker_box(frame: &mut Frame<'_>, app: &App, picker: &Picker) {
    let theme = app.theme;
    let mut lines = vec![
        Line::styled(
            picker.title.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];
    for (index, option) in picker.options.iter().enumerate() {
        let mark = match (picker.multi, option.chosen) {
            (true, true) => "[x] ",
            (true, false) => "[ ] ",
            (false, _) => "",
        };
        let selected = index == picker.selected;
        let style = if selected {
            Style::new()
                .fg(theme.accent())
                .add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        let mut spans = vec![Span::styled(format!(" {mark}{}", option.label), style)];
        if !option.detail.is_empty() {
            spans.push(Span::styled(format!("  {}", option.detail), muted()));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    let keys = if picker.multi {
        "space ticks   enter done   esc cancel"
    } else {
        "↑↓ move   enter choose   esc cancel"
    };
    lines.push(Line::styled(keys, muted()));
    dialog(frame, lines, theme.accent(), 72);
}

/// A box in the middle of the screen.
fn dialog(frame: &mut Frame<'_>, lines: Vec<Line<'static>>, border: Color, most: u16) -> Rect {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(most);
    let inner = usize::from(width.saturating_sub(4)).max(1);
    let rows: u16 = lines.iter().map(|l| wrapped_rows(l, inner)).sum::<u16>() + 2;
    let height = rows.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_style(Style::new().fg(border))
        .padding(ratatui::widgets::Padding::horizontal(1));
    let inner = block.inner(rect);
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        rect,
    );
    inner
}
