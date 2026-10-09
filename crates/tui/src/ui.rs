//! Drawing, in the app's look (ADR 0025): the Welcome screen; a space (its
//! tab bar, the sidebar, the open tab's panes, the status bar); the list of
//! keys; and the boxes that ask. Every cell is painted with the app's palette
//! (`theme.rs`), so it looks the same in every terminal.

use ratatui::Frame;
use ratatui::layout::{Alignment, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::agents::Isolated;
use crate::app::{App, Dialog, Mode, Question, Screen};
use crate::dialog::{FieldKind, Form, Picker};
use crate::layout::Axis;
use crate::listing::{Item, Tone};
use crate::space::{Focus, Label, PanelLine, PanelRow, Sidebar, SpaceLayout, SpaceView, clip};
use crate::theme::Theme;

/// The Welcome's column is at most this wide, as the app's.
const WELCOME_WIDTH: u16 = 84;

/// The smallest window anything is drawn in.
const MIN_SIZE: (u16, u16) = (24, 6);

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let theme = app.theme;
    // The app's background and text, under everything.
    frame.render_widget(Block::new().style(base(theme)), area);
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

/// Text on the app's background.
fn base(theme: Theme) -> Style {
    Style::new().bg(theme.bg()).fg(theme.text())
}

fn muted(theme: Theme) -> Style {
    Style::new().fg(theme.muted())
}

fn faint(theme: Theme) -> Style {
    Style::new().fg(theme.faint())
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

/// A key shown as a key: bold, on a raised chip, as the app's `kbd`.
fn chip(theme: Theme, key: &str) -> Span<'static> {
    Span::styled(
        format!(" {key} "),
        Style::new()
            .bg(theme.active())
            .fg(theme.text())
            .add_modifier(Modifier::BOLD),
    )
}

/// The button that agrees: white on the indigo fill.
fn primary(theme: Theme, key: &str, text: &str) -> Vec<Span<'static>> {
    let fill = Style::new().bg(theme.accent_fill()).fg(theme.on_fill());
    vec![
        Span::styled(format!(" {key}"), fill.add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {text} "), fill),
    ]
}

/// A button that does not.
fn secondary(theme: Theme, key: &str, text: &str) -> Vec<Span<'static>> {
    let plain = Style::new().bg(theme.active()).fg(theme.text());
    vec![
        Span::styled(format!(" {key}"), plain.add_modifier(Modifier::BOLD)),
        Span::styled(format!(" {text} "), plain),
    ]
}

/// Fills `area` with `style` (a background), before drawing on it.
fn fill(frame: &mut Frame<'_>, area: Rect, style: Style) {
    frame.render_widget(Block::new().style(style), area);
}

/// Wide-set capitals: `Aashirvad` as `A A S H I R V A D`.
fn spread(text: &str) -> String {
    let mut out = String::new();
    for (i, c) in text.to_uppercase().chars().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

// The Welcome screen

/// The letters of "WELCOME, SIR", three rows each, drawn with lines: light
/// for WELCOME and heavy for SIR, as the app sets them light and bold.
fn glyph(c: char) -> [&'static str; 3] {
    match c {
        'W' => ["╷   ╷", "│ ╷ │", "└─┴─┘"],
        'E' => ["┌──╴", "├─╴ ", "└──╴"],
        'L' => ["╷   ", "│   ", "└──╴"],
        'C' => ["┌──╴", "│   ", "└──╴"],
        'O' => ["┌──┐", "│  │", "└──┘"],
        'M' => ["┌─┬─┐", "│ │ │", "╵ ╵ ╵"],
        ',' => ["  ", "  ", " ╯"],
        'S' => ["┏━━╸", "┗━━┓", "╺━━┛"],
        'I' => ["╻", "┃", "╹"],
        'R' => ["┏━━┓", "┣━┳┛", "╹ ┗╸"],
        _ => ["", "", ""],
    }
}

/// `word`'s three rows, its letters two columns apart.
fn big(word: &str, row: usize) -> String {
    let mut out = String::new();
    for (i, c) in word.chars().enumerate() {
        if i > 0 && c != ',' {
            out.push_str("  ");
        }
        out.push_str(glyph(c)[row]);
    }
    out
}

/// How wide the big greeting is, its cursor included.
fn big_width() -> u16 {
    let width = big("WELCOME,", 0).width() + 3 + big("SIR", 0).width() + 4;
    u16::try_from(width).unwrap_or(u16::MAX)
}

/// "WELCOME, SIR" and a raspberry block cursor after it: three rows of lines
/// where there is room, else one row of wide-set capitals.
fn greeting(theme: Theme, large: bool) -> Vec<Line<'static>> {
    let welcome = muted(theme);
    let sir = Style::new()
        .fg(theme.highlight())
        .add_modifier(Modifier::BOLD);
    let cursor = Style::new()
        .fg(theme.highlight())
        .add_modifier(Modifier::SLOW_BLINK);
    if !large {
        return vec![Line::from(vec![
            Span::styled(format!("{} ", spread("WELCOME,")), welcome),
            Span::styled(format!(" {} ", spread("SIR")), sir),
            Span::styled("█", cursor),
        ])];
    }
    (0..3)
        .map(|row| {
            Line::from(vec![
                Span::styled(big("WELCOME,", row), welcome),
                Span::raw("   "),
                Span::styled(big("SIR", row), sir),
                Span::raw("  "),
                Span::styled("██", cursor),
            ])
        })
        .collect()
}

fn welcome(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let theme = app.theme;
    let status = Rect {
        y: area.bottom().saturating_sub(1),
        height: 1,
        ..area
    };
    status_bar(frame, app, status, Line::raw(""));
    let body = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let width = body.width.saturating_sub(4).min(WELCOME_WIDTH);
    let x = body.x + (body.width - width) / 2;
    let text_width = usize::from(width.saturating_sub(6)).max(1);

    let large = width >= big_width() && body.height >= 24;
    let roomy = body.height >= 20;
    let title = greeting(theme, large);
    let name = app.name().map(|n| {
        Line::styled(
            spread(n),
            Style::new()
                .fg(theme.highlight())
                .add_modifier(Modifier::BOLD),
        )
    });
    let typed = !app.line.is_empty();
    let below: usize = if typed { app.suggestions.len() } else { 0 };
    let message = app.message.as_ref().map(|m| {
        let style = if m.error {
            Style::new().fg(theme.highlight())
        } else {
            muted(theme)
        };
        Line::styled(m.text.clone(), style)
    });
    let message_rows = message.as_ref().map_or(0, |m| wrapped_rows(m, text_width));
    let hints = hints(app, width);
    let recent = if typed { 0 } else { app.suggestions.len() };

    let box_rows: u16 = if roomy { 4 } else { 2 };
    let gap = |rows: u16| if roomy { rows } else { rows.min(1) };
    let mut total = u16::try_from(title.len()).unwrap_or(3) + gap(2);
    if name.is_some() {
        total += 1 + gap(2);
    }
    total += box_rows + u16::try_from(below).unwrap_or(0);
    total += 1 + message_rows + u16::try_from(hints.len()).unwrap_or(1);
    if recent > 0 {
        total += gap(2) + 1 + u16::try_from(recent).unwrap_or(0);
    }
    let mut y = body.y + body.height.saturating_sub(total) / 2;
    let fits = |y: u16, rows: u16| y + rows <= body.bottom();

    // The greeting, and the name under it.
    for line in title {
        if fits(y, 1) {
            frame.render_widget(
                Paragraph::new(line).alignment(Alignment::Center),
                Rect::new(x, y, width, 1),
            );
        }
        y += 1;
    }
    y += gap(1);
    if let Some(name) = name {
        if fits(y, 1) {
            frame.render_widget(
                Paragraph::new(name).alignment(Alignment::Center),
                Rect::new(x, y, width, 1),
            );
        }
        y += 1 + gap(2);
    } else {
        y += gap(1);
    }

    // The command line, in a raised box with the indigo bar.
    let raised = Style::new().bg(theme.raised());
    let boxed = Rect::new(x, y, width, box_rows.min(body.bottom().saturating_sub(y)));
    fill(frame, boxed, raised);
    for row in boxed.top()..boxed.bottom() {
        if let Some(cell) = frame.buffer_mut().cell_mut((x, row)) {
            cell.set_symbol("▎").set_fg(theme.accent_fill());
        }
    }
    let prompt_y = y + u16::from(roomy);
    let caret = Span::styled("  ›  ", Style::new().fg(theme.highlight()));
    let prompt = if typed {
        Line::from(vec![caret, Span::raw(app.line.text().to_owned())])
    } else {
        Line::from(vec![
            caret,
            Span::styled("Type a command…  \"/cd ~/projects/app\"", faint(theme)),
        ])
    };
    if fits(prompt_y, 1) {
        frame.render_widget(
            Paragraph::new(prompt),
            Rect::new(x + 1, prompt_y, width - 1, 1),
        );
    }
    if fits(prompt_y + 1, 1) {
        frame.render_widget(
            Paragraph::new(context(app)),
            Rect::new(x + 6, prompt_y + 1, width.saturating_sub(6), 1),
        );
    }
    y += box_rows;

    // What fits what is typed, under the box.
    if typed {
        for (i, suggestion) in app.suggestions.iter().enumerate() {
            if !fits(y, 1) {
                break;
            }
            let row = Rect::new(x, y, width, 1);
            let style = if app.selected == Some(i) {
                Style::new().bg(theme.active())
            } else {
                raised
            };
            fill(frame, row, style);
            if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                cell.set_symbol("▎").set_fg(theme.border_strong());
            }
            let line = Line::from(vec![
                Span::raw(suggestion.label.clone()),
                Span::raw("  "),
                Span::styled(suggestion.detail.clone(), muted(theme)),
            ]);
            frame.render_widget(
                Paragraph::new(line),
                Rect::new(x + 6, y, width.saturating_sub(6), 1),
            );
            y += 1;
        }
    }
    y += 1;

    if let Some(message) = message {
        let rows = message_rows.min(body.bottom().saturating_sub(y));
        frame.render_widget(
            Paragraph::new(message).wrap(Wrap { trim: true }),
            Rect::new(x + 6, y, width.saturating_sub(6), rows),
        );
        y += message_rows;
    }
    for line in hints {
        if fits(y, 1) {
            frame.render_widget(
                Paragraph::new(line).alignment(Alignment::Right),
                Rect::new(x, y, width, 1),
            );
        }
        y += 1;
    }

    // The other recent spaces: ↑↓ and Enter open one.
    if recent > 0 {
        y += gap(2);
        if fits(y, 1) {
            frame.render_widget(
                Paragraph::new(Line::styled("RECENT SPACES", faint(theme))),
                Rect::new(x, y, width, 1),
            );
        }
        y += 1;
        for (i, suggestion) in app.suggestions.iter().enumerate() {
            if !fits(y, 1) {
                break;
            }
            let selected = app.selected == Some(i);
            if selected {
                fill(
                    frame,
                    Rect::new(x, y, width, 1),
                    Style::new().bg(theme.active()),
                );
            }
            let mut name = Style::new().fg(theme.accent());
            if selected {
                name = name.add_modifier(Modifier::BOLD);
            }
            let line = Line::from(vec![
                Span::styled(suggestion.label.clone(), name),
                Span::raw("  "),
                Span::styled(suggestion.detail.clone(), faint(theme)),
            ]);
            frame.render_widget(
                Paragraph::new(line),
                Rect::new(x + 1, y, width.saturating_sub(1), 1),
            );
            y += 1;
        }
    }

    if app.question.is_none() && app.dialog.is_none() {
        let caret = u16::try_from(app.line.caret_column() + 6).unwrap_or(u16::MAX);
        if caret < width && fits(prompt_y, 1) {
            frame.set_cursor_position(Position::new(x + caret, prompt_y));
        }
    }
}

/// Under the command line: the space Esc goes back to.
fn context(app: &App) -> Line<'static> {
    let theme = app.theme;
    let space = &app.current;
    let kind = Style::new().fg(theme.accent());
    match &space.root {
        Some(_) => {
            let mut detail = space.id.clone().unwrap_or_default();
            if !detail.is_empty() {
                detail.push_str(" · ");
            }
            detail.push_str(if space.trusted {
                "trusted"
            } else {
                "not trusted"
            });
            Line::from(vec![
                Span::styled("Space", kind),
                Span::raw("  "),
                Span::raw(space.name.clone()),
                Span::raw("  "),
                Span::styled(detail, faint(theme)),
            ])
        }
        None => {
            let mut spans = vec![
                Span::styled("Home", kind),
                Span::raw("  "),
                Span::raw("no folder open"),
            ];
            if let Some(id) = &space.id {
                spans.push(Span::styled(format!("  {id}"), faint(theme)));
            }
            Line::from(spans)
        }
    }
}

/// The commands to know, as keys with what they do: one row, or rows of
/// about as many each where one is too narrow.
fn hints(app: &App, width: u16) -> Vec<Line<'static>> {
    const GAP: &str = "   ";
    let theme = app.theme;
    let back = format!("back to {}", app.current.name);
    let hints = [
        ("/cd", "open a space"),
        ("/new", "new space"),
        ("/share", "share add-ons"),
        ("/home", "no folder"),
        ("esc", back.as_str()),
    ];
    let widths: Vec<usize> = hints
        .iter()
        .map(|(key, text)| key.width() + 2 + 1 + text.width())
        .collect();
    let fits = |count: usize| -> bool {
        widths.chunks(hints.len().div_ceil(count)).all(|row| {
            row.iter().sum::<usize>() + GAP.len() * (row.len() - 1) <= usize::from(width)
        })
    };
    let rows = (1..=hints.len()).find(|&n| fits(n)).unwrap_or(hints.len());
    hints
        .chunks(hints.len().div_ceil(rows))
        .map(|row| {
            let mut spans = Vec::new();
            for (i, (key, text)) in row.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw(GAP));
                }
                spans.push(chip(theme, key));
                spans.push(Span::styled(format!(" {text}"), faint(theme)));
            }
            Line::from(spans)
        })
        .collect()
}

/// The bar at the bottom, as the app's: home and the dot of a running
/// x8ai, `middle` (keys, or a message), and the space with its trust.
fn status_bar(frame: &mut Frame<'_>, app: &App, area: Rect, middle: Line<'static>) {
    let theme = app.theme;
    fill(
        frame,
        area,
        Style::new().bg(theme.raised()).fg(theme.muted()),
    );
    let left = Line::from(vec![
        Span::styled(" ⌂ ", muted(theme)),
        Span::styled("●", Style::new().fg(theme.ok())),
        Span::raw("  "),
    ]);
    let left_width = u16::try_from(left.width()).unwrap_or(0);
    frame.render_widget(Paragraph::new(left), area);
    let space = &app.current;
    let right = space.root.as_ref().map(|_| {
        let (trust, style) = if space.trusted {
            ("Trusted", Style::new().fg(theme.ok()))
        } else {
            ("Untrusted", faint(theme))
        };
        Line::from(vec![
            Span::styled(space.name.clone(), muted(theme)),
            Span::raw("  "),
            Span::styled(trust, style),
            Span::raw(" "),
        ])
    });
    let middle_width = u16::try_from(middle.width()).unwrap_or(u16::MAX);
    let right_width = right
        .as_ref()
        .map_or(0, |r| u16::try_from(r.width()).unwrap_or(u16::MAX));
    let mut room = area.width.saturating_sub(left_width);
    // The space and its trust stay; keys give way to them, but a message
    // that needs the room takes it.
    let message = app.message.is_some();
    if let Some(right) = right
        && (!message || middle_width + right_width + 2 <= room)
        && right_width + 2 <= room
    {
        let at = area.right().saturating_sub(right_width);
        frame.render_widget(
            Paragraph::new(right),
            Rect {
                x: at,
                width: right_width,
                ..area
            },
        );
        room -= right_width + 2;
    }
    // Keys that do not fit are left out whole, never cut.
    let middle = if message {
        middle
    } else {
        whole_spans(middle, usize::from(room))
    };
    frame.render_widget(
        Paragraph::new(middle),
        Rect {
            x: area.x + left_width,
            width: room,
            ..area
        },
    );
}

/// The spans of `line` that fit in `width`, without a key left alone at
/// the end (keys are bold, what they do is not).
fn whole_spans(line: Line<'static>, width: usize) -> Line<'static> {
    let mut kept: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in line.spans {
        let w = span.content.width();
        if used + w > width {
            break;
        }
        used += w;
        kept.push(span);
    }
    while kept
        .last()
        .is_some_and(|s| s.style.add_modifier.contains(Modifier::BOLD))
    {
        kept.pop();
    }
    Line::from(kept)
}

// A space

fn space(frame: &mut Frame<'_>, app: &App) {
    let Some(view) = app.view() else {
        return;
    };
    let layout = view.layout(frame.area());
    header(frame, app, view, &layout);
    if let Some(area) = layout.sidebar {
        sidebar_frame(frame, app.theme, area);
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
                muted(theme)
            };
            let line_style = Style::new().fg(theme.border_strong());
            let name = clip(&slot.title(), usize::from(title.width.saturating_sub(4)));
            let rest = usize::from(title.width).saturating_sub(name.width() + 3);
            let line = Line::from(vec![
                Span::styled("─ ", line_style),
                Span::styled(name, name_style),
                Span::styled(format!(" {}", "─".repeat(rest)), line_style),
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
                Span::styled(format!(" {} {how}. ", slot.title()), bold()),
                Span::styled(format!("{what} · ctrl-g x closes the pane "), muted(theme)),
            ]);
            let row = Rect::new(
                area.body.x,
                area.body.bottom().saturating_sub(1),
                area.body.width,
                1,
            );
            frame.render_widget(Clear, row);
            fill(frame, row, Style::new().bg(theme.raised()).fg(theme.text()));
            frame.render_widget(Paragraph::new(bar), row);
        }
    }
    // The lines between panes side by side.
    for divider in &layout.panes.dividers {
        if divider.axis == Axis::Right {
            for y in divider.line.top()..divider.line.bottom() {
                if let Some(cell) = frame.buffer_mut().cell_mut((divider.line.x, y)) {
                    cell.set_symbol("│").set_fg(theme.border_strong());
                }
            }
        }
    }
    status_bar(frame, app, layout.status, space_keys(app, view));
    if let Some(position) = cursor
        && app.question.is_none()
        && matches!(app.mode, Mode::Normal | Mode::Prefix)
    {
        frame.set_cursor_position(position);
    }
}

/// The tab bar: " x8ai <name>  1 zsh  2 main.rs  +            ~/code/app",
/// raised, the open tab on the app's background.
fn header(frame: &mut Frame<'_>, app: &App, view: &SpaceView, layout: &SpaceLayout) {
    let theme = app.theme;
    let space = &view.info;
    fill(
        frame,
        layout.header,
        Style::new().bg(theme.raised()).fg(theme.text()),
    );
    let prefix = Line::from(vec![
        Span::styled(
            " x8ai ",
            Style::new()
                .fg(theme.highlight())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(space.name.clone(), bold()),
    ]);
    frame.render_widget(Paragraph::new(prefix), layout.header);
    for &(label, rect) in &layout.labels {
        let (text, style) = match label {
            Label::Tab(index) if index == view.active => (
                view.label(index),
                Style::new()
                    .bg(theme.bg())
                    .fg(theme.accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Label::Tab(index) => (view.label(index), muted(theme)),
            Label::New => (" + ".to_owned(), faint(theme)),
        };
        frame.render_widget(Paragraph::new(Span::styled(text, style)), rect);
    }
    let text = view.right_text(usize::from(layout.header.width / 3));
    let width = u16::try_from(text.width()).unwrap_or(u16::MAX);
    let right = Rect {
        x: layout.header.right().saturating_sub(width),
        width,
        ..layout.header
    };
    frame.render_widget(Paragraph::new(Span::styled(text, muted(theme))), right);
}

/// The sidebar's raised background, and the line between it and the panes.
fn sidebar_frame(frame: &mut Frame<'_>, theme: Theme, area: Rect) {
    fill(
        frame,
        area,
        Style::new().bg(theme.raised()).fg(theme.text()),
    );
    for y in area.top()..area.bottom() {
        if let Some(cell) = frame
            .buffer_mut()
            .cell_mut((area.right().saturating_sub(1), y))
        {
            cell.set_symbol("│")
                .set_fg(theme.border_strong())
                .set_bg(theme.bg());
        }
    }
}

/// A sidebar's title: bold, indigo while it has the keys.
fn title_style(theme: Theme, focused: bool) -> Style {
    if focused {
        Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD)
    } else {
        bold()
    }
}

/// A selected row: on the active background where it has the keys, and in
/// indigo either way.
fn selected_style(theme: Theme, focused: bool) -> Style {
    if focused {
        Style::new()
            .bg(theme.active())
            .fg(theme.accent())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().bg(theme.hover()).fg(theme.accent())
    }
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
    let mut lines = vec![Line::styled(
        clip(&format!(" {name}"), width),
        title_style(theme, focused),
    )];
    if let Some(error) = files.error() {
        lines.push(Line::styled(
            clip(&format!(" {error}"), width),
            Style::new().fg(theme.error()),
        ));
    } else if files.rows().is_empty() {
        lines.push(Line::styled(" (empty)", faint(theme)));
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
            faint(theme)
        } else {
            Style::new()
        };
        if row.folder {
            style = style.add_modifier(Modifier::BOLD);
        }
        if index == files.selected() {
            style = selected_style(theme, focused);
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
}

/// The Agents panel: the folder's trust and isolation, the agents, and the
/// space's sessions.
fn agents_panel(frame: &mut Frame<'_>, app: &App, view: &SpaceView, area: Rect) {
    let theme = app.theme;
    let panel = &view.panel;
    let focused = view.focus == Focus::Sidebar;
    let width = usize::from(area.width.saturating_sub(1));
    let mut lines = vec![Line::styled(" AGENTS", title_style(theme, focused))];
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
                Line::styled(clip(&text, width), muted(theme))
            }
            PanelLine::Blank => Line::raw(""),
            PanelLine::Heading(text) => Line::styled(format!(" {text}"), faint(theme)),
            PanelLine::Empty(text) => {
                Line::styled(clip(&format!("   {text}"), width), faint(theme))
            }
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
                let mut style = if dim { muted(theme) } else { Style::new() };
                if index == panel.selected {
                    style = selected_style(theme, focused);
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

/// The middle of a space's status bar: what was said, else the keys that
/// work now.
fn space_keys(app: &App, view: &SpaceView) -> Line<'static> {
    let theme = app.theme;
    let key = |k: &str| {
        Span::styled(
            k.to_owned(),
            Style::new().fg(theme.text()).add_modifier(Modifier::BOLD),
        )
    };
    let text = |t: &str| Span::styled(t.to_owned(), faint(theme));
    if let Some(message) = &app.message {
        let style = if message.error {
            Style::new().fg(theme.highlight())
        } else {
            Style::new().fg(theme.text())
        };
        return Line::styled(message.text.clone(), style);
    }
    let scrolled = view.focused_slot().map_or(0, |s| s.pane.scrolled_back());
    match app.mode {
        Mode::Prefix => Line::from(vec![
            Span::styled(
                " ctrl-g ",
                Style::new()
                    .bg(theme.accent_fill())
                    .fg(theme.on_fill())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            key("t"),
            text(" tab  "),
            key("| -"),
            text(" split  "),
            key("x"),
            text(" close  "),
            key("←→↑↓"),
            text(" move  "),
            key("f"),
            text(" files  "),
            key("a"),
            text(" agents  "),
            key("d"),
            text(" detach  "),
            key("?"),
            text(" all keys  "),
            key("esc"),
            text(" cancel"),
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
    }
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
        ("d", "detach: x8ai keeps running in the background"),
        ("q", "quit, ending every shell and agent"),
        ("ctrl-g", "send ctrl-g to the program"),
    ];
    let theme = app.theme;
    let mut lines = vec![Line::styled("Keys, after ctrl-g", bold()), Line::raw("")];
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
        muted(theme),
    ));
    lines.push(Line::raw(""));
    lines.push(Line::styled("Any key closes this.", faint(theme)));
    dialog(frame, theme, lines, 66);
}

// Questions

/// A question, as the app's dialogs ask: the title, what it means, and the
/// buttons on the right, the one that agrees in indigo.
fn ask(frame: &mut Frame<'_>, app: &App, question: &Question) {
    let theme = app.theme;
    let mut lines = vec![Line::styled(question.title.clone(), bold()), Line::raw("")];
    for line in &question.lines {
        lines.push(Line::styled(line.clone(), muted(theme)));
    }
    lines.push(Line::raw(""));
    lines.push(buttons(
        theme,
        &[("y", question.yes, true), ("n", "cancel", false)],
    ));
    dialog(frame, theme, lines, 72);
}

/// Buttons in a row, on the right: `(key, what, agrees)`.
fn buttons(theme: Theme, list: &[(&str, &str, bool)]) -> Line<'static> {
    let mut spans = Vec::new();
    // The one that agrees comes last, as in the app's dialogs.
    let mut ordered: Vec<_> = list.iter().collect();
    ordered.sort_by_key(|(_, _, agrees)| *agrees);
    for (i, (key, what, agrees)) in ordered.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        if *agrees {
            spans.extend(primary(theme, key, what));
        } else {
            spans.extend(secondary(theme, key, what));
        }
    }
    Line::from(spans).alignment(Alignment::Right)
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
    let mut lines = vec![Line::styled(
        clip(&title, width),
        title_style(theme, focused),
    )];
    let height = usize::from(area.height.saturating_sub(1));
    let mut row = list.items[..list.offset.min(list.items.len())]
        .iter()
        .filter(|i| matches!(i, Item::Row { .. }))
        .count();
    for item in list.items.iter().skip(list.offset).take(height) {
        lines.push(match item {
            Item::Heading(text) => Line::styled(format!(" {text}"), faint(theme)),
            Item::Note(text) => Line::styled(clip(&format!(" {text}"), width), muted(theme)),
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
                    Tone::Muted => faint(theme),
                    Tone::Plain => muted(theme),
                };
                if selected && focused {
                    let text = clip(&format!("{label}  {detail}{}", " ".repeat(pad)), width);
                    Line::styled(text, selected_style(theme, true))
                } else {
                    let label_style = if selected {
                        Style::new().fg(theme.accent())
                    } else if *nested {
                        muted(theme)
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

/// A form: its fields, each in an input on the app's background, the
/// focused one's hint, why it was refused, and its buttons.
fn form_box(frame: &mut Frame<'_>, app: &App, form: &Form) {
    let theme = app.theme;
    // A blank row after each field, where the window is tall enough.
    let rows_spaced = 9 + form.fields.len() * 3 + usize::from(form.note.is_some()) * 3;
    let spaced = usize::from(frame.area().height) >= rows_spaced;
    let mut lines = vec![Line::styled(form.title.clone(), bold()), Line::raw("")];
    if let Some(note) = &form.note {
        lines.push(Line::styled(note.clone(), muted(theme)));
        lines.push(Line::raw(""));
    }
    let input = Style::new().bg(theme.bg()).fg(theme.text());
    let mut caret = None;
    let mut inputs = Vec::new();
    for (index, field) in form.fields.iter().enumerate() {
        let focused = index == form.focus;
        let label_style = if focused {
            Style::new().fg(theme.accent()).add_modifier(Modifier::BOLD)
        } else {
            muted(theme)
        };
        lines.push(Line::styled(field.label.to_owned(), label_style));
        let bar = Span::styled(
            "▎ ",
            Style::new().bg(theme.bg()).fg(if focused {
                theme.accent_fill()
            } else {
                theme.border_strong()
            }),
        );
        let shown = match &field.kind {
            FieldKind::Choice(options, chosen) => {
                let mut spans = vec![bar];
                for (i, option) in options.iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::styled(" ", input));
                    }
                    let style = if i == *chosen {
                        Style::new()
                            .bg(theme.accent_fill())
                            .fg(theme.on_fill())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::new().bg(theme.active()).fg(theme.muted())
                    };
                    spans.push(Span::styled(format!(" {option} "), style));
                }
                Line::from(spans)
            }
            FieldKind::Secret => Line::from(vec![
                bar,
                Span::styled("•".repeat(field.line.text().chars().count()), input),
            ]),
            FieldKind::Text => {
                Line::from(vec![bar, Span::styled(field.line.text().to_owned(), input)])
            }
        };
        if focused && !matches!(field.kind, FieldKind::Choice(..)) {
            let column = match field.kind {
                FieldKind::Secret => field
                    .line
                    .text()
                    .chars()
                    .count()
                    .min(field.line.caret_column()),
                _ => field.line.caret_column(),
            };
            caret = Some((lines.len(), column + 2));
        }
        if !matches!(field.kind, FieldKind::Choice(..)) {
            inputs.push(lines.len());
        }
        lines.push(shown);
        if focused && !field.hint.is_empty() {
            lines.push(Line::styled(field.hint.to_owned(), faint(theme)));
        }
        if spaced {
            lines.push(Line::raw(""));
        }
    }
    if !spaced {
        lines.push(Line::raw(""));
    }
    if let Some(error) = &form.error {
        lines.push(Line::styled(
            error.clone(),
            Style::new().fg(theme.highlight()),
        ));
        lines.push(Line::raw(""));
    }
    let mut footer = vec![
        Span::styled(
            "tab",
            Style::new().fg(theme.text()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" next field   ", faint(theme)),
    ];
    footer.extend(secondary(theme, "esc", "cancel"));
    footer.push(Span::raw("  "));
    footer.extend(primary(theme, "enter", "save"));
    lines.push(Line::from(footer).alignment(Alignment::Right));
    let inner = dialog(frame, theme, lines.clone(), 72);
    let width = usize::from(inner.width).max(1);
    let top_of =
        |line: usize| -> u16 { lines[..line].iter().map(|l| wrapped_rows(l, width)).sum() };
    // Each input fills its row, as a text field does.
    for line in inputs {
        let y = inner.y + top_of(line);
        if y < inner.bottom() {
            let row = Rect::new(inner.x, y, inner.width, 1);
            frame.render_widget(Paragraph::new(lines[line].clone()).style(input), row);
        }
    }
    if let Some((line, column)) = caret {
        let x = inner.x + u16::try_from(column).unwrap_or(0);
        let y = inner.y + top_of(line);
        if x < inner.right() && y < inner.bottom() {
            frame.set_cursor_position(Position::new(x, y));
        }
    }
}

/// A picker: one of a list, or several ticked.
fn picker_box(frame: &mut Frame<'_>, app: &App, picker: &Picker) {
    let theme = app.theme;
    let mut lines = vec![Line::styled(picker.title.clone(), bold()), Line::raw("")];
    for (index, option) in picker.options.iter().enumerate() {
        let mark = match (picker.multi, option.chosen) {
            (true, true) => "● ",
            (true, false) => "○ ",
            (false, _) => "",
        };
        let selected = index == picker.selected;
        let style = if selected {
            Style::new()
                .bg(theme.active())
                .fg(theme.accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        let mut spans = vec![Span::styled(format!(" {mark}{} ", option.label), style)];
        if !option.detail.is_empty() {
            spans.push(Span::styled(format!(" {}", option.detail), muted(theme)));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    let mut footer = Vec::new();
    if picker.multi {
        footer.push(Span::styled(
            "space",
            Style::new().fg(theme.text()).add_modifier(Modifier::BOLD),
        ));
        footer.push(Span::styled(" ticks   ", faint(theme)));
        footer.extend(secondary(theme, "esc", "cancel"));
        footer.push(Span::raw("  "));
        footer.extend(primary(theme, "enter", "done"));
    } else {
        footer.push(Span::styled(
            "↑↓",
            Style::new().fg(theme.text()).add_modifier(Modifier::BOLD),
        ));
        footer.push(Span::styled(" move   ", faint(theme)));
        footer.extend(secondary(theme, "esc", "cancel"));
        footer.push(Span::raw("  "));
        footer.extend(primary(theme, "enter", "choose"));
    }
    lines.push(Line::from(footer).alignment(Alignment::Right));
    dialog(frame, theme, lines, 72);
}

/// A box in the middle of the screen, as the app's dialogs: raised, with a
/// rounded border. Returns where its text goes.
fn dialog(frame: &mut Frame<'_>, theme: Theme, lines: Vec<Line<'static>>, most: u16) -> Rect {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(most);
    let inner = usize::from(width.saturating_sub(6)).max(1);
    let rows: u16 = lines.iter().map(|l| wrapped_rows(l, inner)).sum::<u16>() + 4;
    let height = rows.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme.border_strong()))
        .style(Style::new().bg(theme.raised()).fg(theme.text()))
        .padding(ratatui::widgets::Padding::symmetric(2, 1));
    let inner = block.inner(rect);
    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: true }),
        rect,
    );
    inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_big_greeting_has_rows_of_one_width() {
        let theme = Theme::new(true, true);
        let rows = greeting(theme, true);
        assert_eq!(rows.len(), 3);
        let widths: Vec<usize> = rows.iter().map(Line::width).collect();
        assert!(widths.iter().all(|&w| w == widths[0]), "{widths:?}");
        assert_eq!(widths[0], usize::from(big_width()));
        // Where there is no room, one row of wide-set capitals.
        let small = greeting(theme, false);
        assert_eq!(small.len(), 1);
        assert!(small[0].to_string().starts_with("W E L C O M E ,"));
    }

    #[test]
    fn keys_that_do_not_fit_are_left_out_whole() {
        let theme = Theme::new(true, true);
        let key = |k: &str| {
            Span::styled(
                k.to_owned(),
                Style::new().fg(theme.text()).add_modifier(Modifier::BOLD),
            )
        };
        let line = Line::from(vec![
            key("t"),
            Span::raw(" tab  "),
            key("f"),
            Span::raw(" files  "),
            key("h"),
            Span::raw(" Welcome"),
        ]);
        assert_eq!(
            whole_spans(line.clone(), 100).to_string(),
            "t tab  f files  h Welcome"
        );
        // Room for the key h but not what it does: neither is shown.
        assert_eq!(whole_spans(line, 18).to_string(), "t tab  f files  ");
    }

    #[test]
    fn names_are_spread_as_the_app_spaces_them() {
        assert_eq!(spread("Aashirvad"), "A A S H I R V A D");
    }
}
