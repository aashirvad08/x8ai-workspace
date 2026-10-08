//! A terminal pane: a program on a PTY (`x8ai-pty`) and the emulator that keeps
//! its screen (`alacritty_terminal`), drawn into the TUI's own screen.
//!
//! What the program prints never reaches the user's terminal as it is: it is
//! parsed into a grid here and drawn as plain cells. So a program in a pane
//! cannot set the outer terminal's title, clipboard or colors, or answer for it.
//! The emulator answers the program's queries (cursor position, device
//! attributes) itself; clipboard access (OSC 52) is off.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Config, Osc52, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as TermColor, CursorShape, NamedColor, Processor};
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};
use x8ai_core::terminal::{SessionId, TerminalExit, TerminalSize};
use x8ai_pty::{Program, Session, SessionEvents, Sessions};

use crate::app::Msg;
use crate::mouse::{self, Action, Button};
use crate::theme::Theme;

/// Names a pane in messages from its session's threads. A new pane, even for
/// the same space, gets a new id, so messages from one that was replaced are
/// recognized and dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneId(pub u32);

pub struct Pane {
    pub id: PaneId,
    session: Arc<Session>,
    term: Term<Listener>,
    parser: Processor,
    events: Listener,
    size: TerminalSize,
    title: Option<String>,
    exit: Option<TerminalExit>,
    /// Where a selection with the mouse started.
    anchor: Option<Point>,
}

impl Pane {
    /// Starts `program` on a new PTY of `size`. Its output and exit arrive on
    /// `tx` as messages for this pane.
    pub fn start(
        id: PaneId,
        sessions: &Sessions,
        program: &Program,
        size: TerminalSize,
        tx: Sender<Msg>,
    ) -> Result<Self, String> {
        let events = Listener::default();
        let config = Config {
            osc52: Osc52::Disabled,
            kitty_keyboard: false,
            ..Config::default()
        };
        let term = Term::new(config, &Grid(size), events.clone());
        let session = sessions
            .spawn(program, size, Arc::new(Forward { pane: id, tx }))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            id,
            session,
            term,
            parser: Processor::new(),
            events,
            size,
            title: None,
            exit: None,
            anchor: None,
        })
    }

    /// The program's output: parsed into the screen, and acknowledged, so the
    /// session keeps reading (flow control, ADR 0006).
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        self.answer();
        self.session
            .ack(u32::try_from(bytes.len()).unwrap_or(u32::MAX));
    }

    /// What the emulator asked for while parsing.
    fn answer(&mut self) {
        let events = std::mem::take(&mut *self.events.0.borrow_mut());
        for event in events {
            match event {
                // Replies to the program's queries: cursor position, device
                // attributes, modes.
                TermEvent::PtyWrite(text) => self.write(text.into_bytes()),
                TermEvent::Title(title) => self.title = Some(title),
                TermEvent::ResetTitle => self.title = None,
                // The colors and pixel size of the user's terminal are not
                // known here, so those queries go unanswered, as some terminals
                // leave them; programs then use their defaults. Clipboard
                // events do not occur with OSC 52 off.
                _ => {}
            }
        }
    }

    /// Input for the program. Dropped once it has exited.
    pub fn write(&self, bytes: Vec<u8>) {
        if self.exit.is_none() {
            // Fails only once the process is gone; its exit is on its way.
            let _ = self.session.write(bytes);
        }
    }

    pub fn resize(&mut self, size: TerminalSize) {
        if size == self.size || !size.is_valid() {
            return;
        }
        self.size = size;
        self.term.resize(Grid(size));
        if self.exit.is_none() {
            let _ = self.session.resize(size);
        }
    }

    pub fn exited(&mut self, exit: TerminalExit) {
        self.exit = Some(exit);
    }

    pub fn exit(&self) -> Option<&TerminalExit> {
        self.exit.as_ref()
    }

    /// The program's path, as started (the user's shell, for a shell).
    pub fn program(&self) -> &str {
        self.session.program()
    }

    /// The title the program set (OSC 0/2), if any.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Whether a program other than the shell is in the foreground, so closing
    /// the pane would end it.
    pub fn is_busy(&self) -> bool {
        self.exit.is_none() && self.session.has_foreground_job()
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// Scrolls the view through the scrollback (the program is not told).
    pub fn scroll(&mut self, scroll: Scroll) {
        self.term.scroll_display(scroll);
        self.answer();
    }

    /// Scrolls the view by `lines` (up when positive), as the mouse wheel does.
    pub fn scroll_lines(&mut self, lines: i32) {
        self.scroll(Scroll::Delta(lines));
    }

    /// Whether the program asked for the mouse (vim, htop, less with `--mouse`).
    pub fn wants_mouse(&self) -> bool {
        mouse::wanted(self.mode())
    }

    /// Reports a mouse event at `col`, `row` of the pane to the program, if
    /// it asked for this kind.
    pub fn report_mouse(
        &self,
        action: Action,
        button: Button,
        (col, row): (u16, u16),
        modifiers: KeyModifiers,
    ) {
        if let Some(bytes) = mouse::report(action, button, col, row, modifiers, self.mode()) {
            self.write(bytes);
        }
    }

    /// The point of the grid shown at `col`, `row` of the pane.
    fn point(&self, col: u16, row: u16) -> Point {
        let offset = i32::try_from(self.scrolled_back()).unwrap_or(i32::MAX);
        let last_line = i32::try_from(self.term.screen_lines()).unwrap_or(1) - 1;
        let last_column = self.term.columns().saturating_sub(1);
        Point::new(
            Line(i32::from(row).min(last_line) - offset),
            Column(usize::from(col).min(last_column)),
        )
    }

    /// Starts selecting text at `col`, `row`.
    pub fn start_selection(&mut self, col: u16, row: u16) {
        self.anchor = Some(self.point(col, row));
        self.term.selection = None;
    }

    /// Selects from where it started to `col`, `row`, both cells included.
    pub fn extend_selection(&mut self, col: u16, row: u16) {
        let Some(anchor) = self.anchor else {
            return;
        };
        let to = self.point(col, row);
        let (from_side, to_side) = if to >= anchor {
            (Side::Left, Side::Right)
        } else {
            (Side::Right, Side::Left)
        };
        let mut selection = Selection::new(SelectionType::Simple, anchor, from_side);
        selection.update(to, to_side);
        self.term.selection = Some(selection);
    }

    /// Ends the selection: its text, if anything was selected. It stays
    /// highlighted until cleared.
    pub fn end_selection(&mut self) -> Option<String> {
        self.anchor = None;
        let text = self.term.selection_to_string().filter(|t| !t.is_empty());
        if text.is_none() {
            self.term.selection = None;
        }
        text
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
        self.term.selection = None;
    }

    /// Lines scrolled back from the bottom.
    pub fn scrolled_back(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// When a synchronized update (DEC 2026) the program began must be shown
    /// even though it has not ended it.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    pub fn end_sync(&mut self) {
        self.parser.stop_sync(&mut self.term);
        self.answer();
    }

    /// The session's id in the registry the pane was started from.
    pub fn session_id(&self) -> SessionId {
        self.session.id()
    }

    /// The cursor's shape and whether it blinks, `None` when hidden or when the
    /// view is scrolled back.
    pub fn cursor_style(&self) -> Option<(CursorShape, bool)> {
        let style = self.term.cursor_style();
        let shown = self.mode().contains(TermMode::SHOW_CURSOR) && self.scrolled_back() == 0;
        (shown && self.exit.is_none()).then_some((style.shape, style.blinking))
    }

    /// Draws the screen into `area`. Returns where the cursor is, if shown.
    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: Theme) -> Option<Position> {
        let content = self.term.renderable_content();
        let selection = content.selection;
        let offset = i32::try_from(content.display_offset).unwrap_or(i32::MAX);
        for indexed in content.display_iter {
            let (Ok(row), Ok(col)) = (
                u16::try_from(indexed.point.line.0 + offset),
                u16::try_from(indexed.point.column.0),
            ) else {
                continue;
            };
            if row >= area.height || col >= area.width {
                continue;
            }
            let Some(out) = buf.cell_mut((area.x + col, area.y + row)) else {
                continue;
            };
            out.reset();
            let cell = indexed.cell;
            // The second half of a wide character: the screen skips it.
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let c = if cell.flags.contains(Flags::HIDDEN) || cell.c.is_control() {
                ' '
            } else {
                cell.c
            };
            match cell.zerowidth() {
                Some(marks) if !marks.is_empty() => {
                    let symbol: String = std::iter::once(c).chain(marks.iter().copied()).collect();
                    out.set_symbol(&symbol);
                }
                _ => {
                    out.set_char(c);
                }
            }
            out.fg = color(cell.fg, content.colors, theme);
            out.bg = color(cell.bg, content.colors, theme);
            out.modifier = modifier(cell.flags);
            if selection.is_some_and(|s| s.contains(indexed.point)) {
                out.modifier.toggle(Modifier::REVERSED);
            }
        }
        let point = content.cursor.point;
        let shown = content.cursor.shape != CursorShape::Hidden
            && content.display_offset == 0
            && self.exit.is_none();
        let (Ok(row), Ok(col)) = (u16::try_from(point.line.0), u16::try_from(point.column.0))
        else {
            return None;
        };
        (shown && row < area.height && col < area.width)
            .then(|| Position::new(area.x + col, area.y + row))
    }
}

/// A pane that goes away hangs up its program, as closing a terminal window does.
impl Drop for Pane {
    fn drop(&mut self) {
        self.session.close();
    }
}

/// A pane's size as the emulator's grid sees it.
struct Grid(TerminalSize);

impl Dimensions for Grid {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.0.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.0.cols)
    }
}

/// Keeps what the emulator asks for while parsing, to handle after.
#[derive(Clone, Default)]
struct Listener(Rc<RefCell<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        self.0.borrow_mut().push(event);
    }
}

/// Passes a session's output and exit to the main loop, from its threads.
struct Forward {
    pane: PaneId,
    tx: Sender<Msg>,
}

impl SessionEvents for Forward {
    // Sending fails only when the main loop has ended, and the pane with it.
    fn output(&self, bytes: Vec<u8>) {
        let _ = self.tx.send(Msg::Output(self.pane, bytes));
    }

    fn error(&self, message: String) {
        let _ = self.tx.send(Msg::PaneError(self.pane, message));
    }

    fn exited(&self, exit: TerminalExit) {
        let _ = self.tx.send(Msg::Exited(self.pane, exit));
    }
}

/// A cell's color: what the program set, through the palette it may have
/// changed (OSC 4), else the user's terminal's own.
fn color(color: TermColor, colors: &Colors, theme: Theme) -> Color {
    let custom = |index: usize| colors[index].map(|rgb| theme.rgb(rgb.r, rgb.g, rgb.b));
    match color {
        TermColor::Spec(rgb) => theme.rgb(rgb.r, rgb.g, rgb.b),
        TermColor::Indexed(i) => custom(usize::from(i)).unwrap_or(Color::Indexed(i)),
        TermColor::Named(name) => custom(name as usize).unwrap_or_else(|| named(name)),
    }
}

fn named(name: NamedColor) -> Color {
    use NamedColor as N;
    match name {
        N::Black | N::DimBlack => Color::Black,
        N::Red | N::DimRed => Color::Red,
        N::Green | N::DimGreen => Color::Green,
        N::Yellow | N::DimYellow => Color::Yellow,
        N::Blue | N::DimBlue => Color::Blue,
        N::Magenta | N::DimMagenta => Color::Magenta,
        N::Cyan | N::DimCyan => Color::Cyan,
        N::White | N::DimWhite => Color::Gray,
        N::BrightBlack => Color::DarkGray,
        N::BrightRed => Color::LightRed,
        N::BrightGreen => Color::LightGreen,
        N::BrightYellow => Color::LightYellow,
        N::BrightBlue => Color::LightBlue,
        N::BrightMagenta => Color::LightMagenta,
        N::BrightCyan => Color::LightCyan,
        N::BrightWhite => Color::White,
        N::Foreground | N::Background | N::Cursor | N::BrightForeground | N::DimForeground => {
            Color::Reset
        }
    }
}

fn modifier(flags: Flags) -> Modifier {
    let mut modifier = Modifier::empty();
    for (flag, style) in [
        (Flags::BOLD, Modifier::BOLD),
        (Flags::DIM, Modifier::DIM),
        (Flags::ITALIC, Modifier::ITALIC),
        (Flags::STRIKEOUT, Modifier::CROSSED_OUT),
        (Flags::INVERSE, Modifier::REVERSED),
    ] {
        if flags.contains(flag) {
            modifier |= style;
        }
    }
    if flags.intersects(Flags::ALL_UNDERLINES) {
        modifier |= Modifier::UNDERLINED;
    }
    modifier
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    /// A pane running `script` in `sh`, fed until `done` shows, drawn.
    fn run(script: &str, done: &str) -> (Pane, Buffer) {
        let (tx, rx) = mpsc::channel();
        let sessions = Sessions::default();
        let size = TerminalSize { cols: 40, rows: 6 };
        let program = Program::Exec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: None,
            env: x8ai_pty::Environment::Inherit,
        };
        let mut pane = Pane::start(PaneId(1), &sessions, &program, size, tx).unwrap();
        let area = Rect::new(0, 0, size.cols, size.rows);
        let mut buf = Buffer::empty(area);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(Msg::Output(_, bytes)) => pane.feed(&bytes),
                Ok(Msg::Exited(_, exit)) => pane.exited(exit),
                Ok(_) => {}
                Err(_) => panic!("no {done:?} on the screen"),
            }
            pane.render(area, &mut buf, Theme::detect());
            if screen(&buf).contains(done) {
                return (pane, buf);
            }
        }
    }

    fn screen(buf: &Buffer) -> String {
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn output_is_drawn_with_its_colors() {
        let (_pane, buf) = run(
            r"printf 'plain \033[1;31mred\033[0m\nnext:%s' $((40 + 2))",
            "next:42",
        );
        let text = screen(&buf);
        assert!(text.starts_with("plain red"), "{text}");
        let red = &buf[(6, 0)];
        assert_eq!(red.symbol(), "r");
        assert_eq!(red.fg, Color::Red);
        assert!(red.modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(0, 0)].fg, Color::Reset);
    }

    #[test]
    fn the_program_gets_answers_to_its_queries() {
        // Asks where the cursor is (DSR 6) and prints the answer it reads back.
        let script = r#"stty raw -echo; printf 'abc\033[6n'; reply=$(dd bs=1 count=6 2>/dev/null | od -An -c | tr -d ' \n'); stty sane; printf '\nreply:%s:end' "$reply""#;
        let (_pane, buf) = run(script, ":end");
        let text = screen(&buf);
        // ESC [ 1 ; 4 R: row 1, column 4.
        assert!(text.contains(r"reply:033[1;4R:end"), "{text}");
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let (_pane, buf) = run("printf '日本x'; printf '\\ndone'", "done");
        assert_eq!(buf[(0, 0)].symbol(), "日");
        assert_eq!(buf[(2, 0)].symbol(), "本");
        assert_eq!(buf[(4, 0)].symbol(), "x");
    }
}
