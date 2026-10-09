//! `x8ai` end to end, step 2: split panes and tabs, the file list, the user's
//! editor, and the mouse.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use common::{CTRL_G, X8ai};

const LEFT: &str = "\x1b[D";

fn script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn tabs_panes_files_and_the_editor() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let proj = home.join("proj");
    fs::create_dir_all(proj.join("src")).unwrap();
    fs::write(proj.join("notes.txt"), "hello\n").unwrap();
    fs::write(proj.join("src/main.rs"), "fn main() {}\n").unwrap();
    // A stand-in editor that says what it opened and waits for Enter, and a
    // stand-in pbcopy, so the user's clipboard is left alone.
    let bin = home.join("bin");
    fs::create_dir(&bin).unwrap();
    script(
        &bin.join("edit"),
        "#!/bin/sh\nprintf 'EDITING:%s\\n' \"$(basename \"$1\")\"\nIFS= read -r line\n",
    );
    script(
        &bin.join("pbcopy"),
        "#!/bin/sh\ncat > \"$HOME/copied.txt\"\n",
    );
    let mut x8ai = X8ai::start_with(
        &home,
        &["proj"],
        &[
            ("EDITOR", bin.join("edit").display().to_string()),
            (
                "PATH",
                format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
            ),
        ],
    );
    x8ai.wait_for_space("proj  Untrusted");

    // Ctrl-g |: a shell to the right, which gets the keys.
    x8ai.keys(&format!("{CTRL_G}|"));
    x8ai.wait_until("two panes", |s| s.matches("─ sh ").count() == 2);
    x8ai.keys("echo \"right:$((1 + 1))\"\r");
    x8ai.wait_for("right:2");
    assert!(x8ai.find("right:2").unwrap().0 > 50);

    // Ctrl-g ←: the pane to the left.
    x8ai.keys(&format!("{CTRL_G}{LEFT}"));
    x8ai.keys("echo \"left:$((2 + 2))\"\r");
    x8ai.wait_for("left:4");
    assert!(x8ai.find("left:4").unwrap().0 < 50);

    // A click gives a pane the keys.
    x8ai.click(80, 15);
    x8ai.keys("echo \"clicked:$((3 + 3))\"\r");
    x8ai.wait_for("clicked:6");
    assert!(x8ai.find("clicked:6").unwrap().0 > 50);

    // Dragging the line between them resizes both: the left one is now 30
    // columns wide, under its title row.
    x8ai.drag((50, 15), (30, 15));
    x8ai.click(10, 15);
    std::thread::sleep(Duration::from_millis(300));
    x8ai.keys("stty size\r");
    x8ai.wait_for("27 30");

    // Ctrl-g t: a new tab; Ctrl-g 1: back to the first.
    x8ai.keys(&format!("{CTRL_G}t"));
    x8ai.wait_for(" 2 sh ");
    x8ai.keys("echo \"tab:$((6 * 7))\"\r");
    x8ai.wait_for("tab:42");
    x8ai.keys(&format!("{CTRL_G}1"));
    let screen = x8ai.wait_for("left:4");
    assert!(!screen.contains("tab:42"), "{screen}");
    x8ai.keys(&format!("{CTRL_G}2"));
    x8ai.wait_for("tab:42");

    // Ctrl-g f: the file list; Enter on a file opens it in $EDITOR, in a tab
    // of its own, which closes when the editor ends well.
    x8ai.keys(&format!("{CTRL_G}f"));
    let screen = x8ai.wait_for(" PROJ");
    assert!(screen.contains("▸ src"), "{screen}");
    assert!(screen.contains("notes.txt"), "{screen}");
    x8ai.keys("j\r");
    let screen = x8ai.wait_for("EDITING:notes.txt");
    assert!(screen.contains(" 3 notes.txt "), "{screen}");
    x8ai.keys("\r");
    x8ai.wait_until("the editor's tab closed", |s| !s.contains(" 3 notes.txt "));
    x8ai.wait_for("tab:42");

    // Dragging over text selects it, and letting go copies it.
    x8ai.keys("printf 'CO%sME\\n' PY\r");
    x8ai.wait_for("COPYME");
    let (col, row) = x8ai.find("COPYME").unwrap();
    x8ai.drag((col, row), (col + 5, row));
    x8ai.wait_for("Copied 6 characters.");
    assert_eq!(
        fs::read_to_string(home.join("copied.txt")).unwrap(),
        "COPYME"
    );

    // The wheel scrolls back, and forward again.
    x8ai.keys("seq 1 200; echo \"done:$((5 * 5))\"\r");
    x8ai.wait_for("done:25");
    x8ai.wheel(60, 15, true);
    x8ai.wheel(60, 15, true);
    x8ai.wait_for("Scrolled back 6 lines");
    x8ai.wheel(60, 15, false);
    x8ai.wheel(60, 15, false);
    x8ai.wait_until("back at the bottom", |s| !s.contains("Scrolled back"));

    // Ctrl-g ?: every key; any key closes it.
    x8ai.keys(&format!("{CTRL_G}?"));
    x8ai.wait_for("Keys, after ctrl-g");
    x8ai.keys("x");
    x8ai.wait_until("the keys closed", |s| !s.contains("Keys, after ctrl-g"));

    x8ai.keys(&format!("{CTRL_G}q"));
    assert_eq!(x8ai.wait_for_exit().code, 0);
}
