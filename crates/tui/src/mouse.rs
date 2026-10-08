//! Mouse reports for a program in a pane that asked for them (xterm's modes
//! 1000, 1002 and 1003), in SGR form (1006) or the original one. Positions are
//! the pane's own, so the program never learns where it is on the screen.

use alacritty_terminal::term::TermMode;
use ratatui::crossterm::event::KeyModifiers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Press,
    Release,
    Drag,
}

/// Whether the program asked for mouse reports at all.
pub fn wanted(mode: TermMode) -> bool {
    mode.intersects(TermMode::MOUSE_MODE)
}

/// The report for a mouse event at `col`, `row` of the pane (from 0), or
/// `None` when the program did not ask for this kind of event.
pub fn report(
    action: Action,
    button: Button,
    col: u16,
    row: u16,
    modifiers: KeyModifiers,
    mode: TermMode,
) -> Option<Vec<u8>> {
    let asked = match action {
        Action::Press | Action::Release => wanted(mode),
        Action::Drag => mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION),
    };
    let wheel = matches!(button, Button::WheelUp | Button::WheelDown);
    if !asked || (wheel && action != Action::Press) {
        return None;
    }
    let mut held = 0;
    if modifiers.contains(KeyModifiers::SHIFT) {
        held += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        held += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        held += 16;
    }
    let button_code: u16 = match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
        Button::WheelUp => 64,
        Button::WheelDown => 65,
    };
    let motion = if action == Action::Drag { 32 } else { 0 };
    let (x, y) = (u32::from(col) + 1, u32::from(row) + 1);
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if action == Action::Release { 'm' } else { 'M' };
        let code = button_code + motion + held;
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    // The original form: a release does not say which button, and positions
    // past 223 cannot be written.
    let code = if action == Action::Release {
        3
    } else {
        button_code
    } + motion
        + held;
    if x > 223 || y > 223 {
        return None;
    }
    Some(vec![
        0x1b,
        b'[',
        b'M',
        32 + code as u8,
        32 + x as u8,
        32 + y as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyModifiers = KeyModifiers::NONE;

    #[test]
    fn nothing_is_reported_unless_asked() {
        let mode = TermMode::default();
        assert_eq!(report(Action::Press, Button::Left, 0, 0, NONE, mode), None);
        assert_eq!(
            report(Action::Press, Button::WheelUp, 0, 0, NONE, mode),
            None
        );
        // Clicks only: no drags.
        let clicks = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert!(report(Action::Press, Button::Left, 0, 0, NONE, clicks).is_some());
        assert_eq!(report(Action::Drag, Button::Left, 0, 0, NONE, clicks), None);
    }

    #[test]
    fn sgr_reports_say_where_and_what() {
        let mode = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
        let at = |action, button, modifiers| {
            String::from_utf8(report(action, button, 9, 4, modifiers, mode).unwrap()).unwrap()
        };
        assert_eq!(at(Action::Press, Button::Left, NONE), "\x1b[<0;10;5M");
        assert_eq!(at(Action::Release, Button::Left, NONE), "\x1b[<0;10;5m");
        assert_eq!(at(Action::Drag, Button::Left, NONE), "\x1b[<32;10;5M");
        assert_eq!(
            at(Action::Press, Button::Right, KeyModifiers::CONTROL),
            "\x1b[<18;10;5M"
        );
        assert_eq!(at(Action::Press, Button::WheelDown, NONE), "\x1b[<65;10;5M");
        assert_eq!(
            report(Action::Release, Button::WheelDown, 9, 4, NONE, mode),
            None
        );
    }

    #[test]
    fn the_original_form_has_no_release_button_and_a_limit() {
        let mode = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(
            report(Action::Press, Button::Left, 9, 4, NONE, mode).unwrap(),
            [0x1b, b'[', b'M', 32, 42, 37]
        );
        assert_eq!(
            report(Action::Release, Button::Left, 9, 4, NONE, mode).unwrap(),
            [0x1b, b'[', b'M', 35, 42, 37]
        );
        assert_eq!(
            report(Action::Press, Button::Left, 300, 4, NONE, mode),
            None
        );
    }
}
