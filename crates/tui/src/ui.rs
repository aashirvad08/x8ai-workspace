//! Drawing: the Welcome screen, a space (its header, its shell, the status
//! bar), and the question asked before quitting.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Mode, QuitQuestion, Screen};
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
    if let Some(question) = &app.quit_question {
        quit_question(frame, app, question);
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
        .map(|l| u16::try_from(l.width().div_ceil(usize::from(width.max(1))).max(1)).unwrap_or(1))
        .sum();
    // A little above the middle, where the eye starts.
    let top = area.y + area.height.saturating_sub(rows) / 3;
    let column = Rect::new(x, top, width, area.height.saturating_sub(top - area.y));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), column);

    if app.quit_question.is_none() {
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
    let area = frame.area();
    let header = Rect::new(area.x, area.y, area.width, 1);
    let status = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    let body = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(2),
    );

    frame.render_widget(space_header(app, area.width), header);
    let Some(pane) = app.current_pane() else {
        return;
    };
    let cursor = pane.render(body, frame.buffer_mut(), app.theme);
    if let Some(exit) = pane.exit() {
        let how = match &exit.signal {
            Some(signal) => format!("ended by {signal}"),
            None => format!("exited with {}", exit.code),
        };
        let bar = Line::from(vec![
            Span::styled(
                format!(" The shell {how}. "),
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::raw("Enter starts a new one · ctrl-g h Welcome · ctrl-g q quit "),
        ]);
        let row = Rect::new(body.x, body.bottom().saturating_sub(1), body.width, 1);
        frame.render_widget(Clear, row);
        frame.render_widget(
            Paragraph::new(bar).style(Style::new().add_modifier(Modifier::REVERSED)),
            row,
        );
    }
    frame.render_widget(status_bar(app, pane.scrolled_back()), status);
    if let Some(position) = cursor
        && app.quit_question.is_none()
        && app.mode != Mode::Scroll
    {
        frame.set_cursor_position(position);
    }
}

fn space_header(app: &App, width: u16) -> Paragraph<'static> {
    let theme = app.theme;
    let space = &app.current;
    let mut left = vec![
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
    ];
    let place = space
        .root
        .as_deref()
        .map_or_else(|| "~".to_owned(), |root| tilde(root, app.home()));
    left.push(Span::styled(format!("  {place}"), muted()));
    if let Some(id) = &space.id {
        left.push(Span::styled(format!("  {id}"), muted()));
    }
    if let Some(title) = app.current_pane().and_then(|p| p.title()) {
        let title: String = title.chars().filter(|c| !c.is_control()).take(60).collect();
        left.push(Span::styled(
            format!("  — {title}"),
            muted().add_modifier(Modifier::ITALIC),
        ));
    }
    let (trust, color) = match (&space.root, space.trusted) {
        (None, _) => ("", Color::Reset),
        (Some(_), true) => ("trusted ", theme.ok()),
        (Some(_), false) => ("not trusted ", Color::Reset),
    };
    let used: usize = left.iter().map(|s| s.width()).sum();
    let pad = usize::from(width).saturating_sub(used + trust.width());
    left.push(Span::raw(" ".repeat(pad)));
    let trust_style = if space.trusted {
        Style::new().fg(color)
    } else {
        muted()
    };
    left.push(Span::styled(trust, trust_style));
    Paragraph::new(Line::from(left))
}

fn status_bar(app: &App, scrolled: usize) -> Paragraph<'static> {
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
    let line = match app.mode {
        Mode::Normal => Line::from(vec![
            text(" "),
            key("ctrl-g"),
            text(" then  "),
            key("h"),
            text(" Welcome  "),
            key("s"),
            text(" scroll back  "),
            key("q"),
            text(" quit"),
        ]),
        Mode::Prefix => Line::from(vec![
            Span::styled(
                " ctrl-g ",
                Style::new()
                    .fg(theme.highlight())
                    .add_modifier(Modifier::BOLD),
            ),
            key("h"),
            Span::raw(" Welcome   "),
            key("s"),
            Span::raw(" scroll back   "),
            key("q"),
            Span::raw(" quit   "),
            key("ctrl-g"),
            Span::raw(" send ctrl-g   "),
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
            text(" back to the shell"),
        ]),
    };
    Paragraph::new(line)
}

// Quitting

fn quit_question(frame: &mut Frame<'_>, app: &App, question: &QuitQuestion) {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(64);
    let names = question.busy.join(", ");
    let lines = vec![
        Line::styled("Quit x8ai?", Style::new().add_modifier(Modifier::BOLD)),
        Line::raw(""),
        Line::raw(format!(
            "A program is still running in {names}. Quitting stops it."
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                "y",
                Style::new()
                    .fg(app.theme.highlight())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" quit   "),
            Span::styled(
                "n",
                Style::new()
                    .fg(app.theme.accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" stay"),
        ]),
    ];
    let inner = usize::from(width.saturating_sub(4)).max(1);
    let rows: u16 = lines
        .iter()
        .map(|l| u16::try_from(l.width().div_ceil(inner).max(1)).unwrap_or(1))
        .sum::<u16>()
        + 2;
    let height = rows.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_style(Style::new().fg(app.theme.highlight()))
        .padding(ratatui::widgets::Padding::horizontal(1));
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        rect,
    );
}
