//! `x8ai` end to end, step 4: add-ons. One that is installed is added to the
//! space at once; one that is not asks first, showing the exact install
//! command (declined here: nothing is installed); Enter turns an added one
//! off, as d does; and a space's add-ons are given to another with `/share`.
//! A program for the whole Mac is turned off in one space only: new shells
//! there, and the editor, get a stand-in that says so. They are the app's own
//! (spaces.json).

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{CTRL_G, X8ai};

fn program(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn addons_are_added_removed_and_shared() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    fs::create_dir(home.join("proj")).unwrap();
    let bin = home.join("bin");
    fs::create_dir(&bin).unwrap();
    // fzf is "installed": its program is on the PATH. The shell here is sh,
    // so an added one waits for zsh.
    program(&bin.join("fzf"), "#!/bin/sh\n");
    let mut x8ai = X8ai::start_with(
        &home,
        &["proj"],
        &[(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
        )],
    );
    x8ai.wait_for_space("proj  Untrusted");

    x8ai.keys(&format!("{CTRL_G}e"));
    let screen = x8ai.wait_for(" ADD-ONS · proj");
    assert!(screen.contains("fzf  installed: Enter adds it"), "{screen}");

    // An installed one is added at once.
    let (col, row) = x8ai.find("fzf  installed").unwrap();
    x8ai.click(col, row);
    x8ai.keys("\r");
    x8ai.wait_for("fzf is added to proj.");
    x8ai.wait_for("fzf  added · needs zsh");
    let spaces = fs::read_to_string(home.join(".x8ai-data/spaces.json")).unwrap();
    assert!(spaces.contains("fzf"), "{spaces}");

    // One that is not installed asks first, with the exact command; or says
    // Homebrew is missing, where it is.
    let (col, row) = x8ai.find("zoxide").unwrap();
    x8ai.click(col, row);
    x8ai.keys("\r");
    let screen = x8ai.wait_until("asked or refused", |s| {
        s.contains("y install") || s.contains("Homebrew is not installed. Install it from brew.sh")
    });
    if screen.contains("y install") {
        assert!(screen.contains("Add zoxide to proj?"), "{screen}");
        assert!(screen.contains("install zoxide"), "{screen}");
        x8ai.keys("n");
        x8ai.wait_until("declined", |s| !s.contains("y install"));
    }

    // A space's add-ons go to another space with /share.
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("Type a command");
    x8ai.keys("/new other\r");
    x8ai.wait_for_space("other  Untrusted");
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("Type a command");
    x8ai.keys("/cd proj\r");
    x8ai.wait_for_space("proj  Untrusted");
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("Type a command");
    x8ai.keys("/share other\r");
    let screen = x8ai.wait_until("shared", |s| s.contains("now has proj's add-ons: fzf."));
    assert!(screen.contains("ws-"), "{screen}");

    // Enter on an added one takes it out of this space; the other keeps it.
    // The panel is still open: a space keeps its sidebar.
    x8ai.keys("\x1b");
    x8ai.wait_until("back with the panel", |s| {
        s.contains("proj  Untrusted")
            && s.contains(" ADD-ONS · proj")
            && s.contains("fzf  added · needs zsh")
    });
    let (col, row) = x8ai.find("fzf  added").unwrap();
    x8ai.click(col, row);
    x8ai.keys("\r");
    x8ai.wait_for("fzf is removed from proj. New terminals here start without it.");
    x8ai.wait_for("fzf  installed: Enter adds it");
    let spaces = fs::read_to_string(home.join(".x8ai-data/spaces.json")).unwrap();
    assert_eq!(spaces.matches("fzf").count(), 1, "{spaces}");

    // Enter adds it again, and d removes it too.
    x8ai.keys("\r");
    x8ai.wait_for("fzf is added to proj.");
    x8ai.wait_for("fzf  added · needs zsh");
    x8ai.keys("d");
    x8ai.wait_for("fzf is removed from proj.");
    x8ai.wait_for("fzf  installed: Enter adds it");
    let spaces = fs::read_to_string(home.join(".x8ai-data/spaces.json")).unwrap();
    assert_eq!(spaces.matches("fzf").count(), 1, "{spaces}");

    x8ai.keys(&format!("{CTRL_G}q"));
    assert_eq!(x8ai.wait_for_exit().code, 0);
}

/// Says which `nvim` a shell finds: the stand-in of a program turned off
/// (`OFF-NVIM`), or another (`ON-NVIM`). Paths here are too long for a line.
const WHICH_NVIM: &str =
    "[[ $(whence -p nvim) == */off/nvim ]] && echo OFF-''NVIM || echo ON-''NVIM\r";

#[test]
fn a_program_for_the_whole_mac_is_turned_off_in_one_space_only() {
    if !Path::new("/bin/zsh").is_file() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    fs::create_dir(home.join("proj")).unwrap();
    fs::write(home.join("proj/notes.txt"), "hello\n").unwrap();
    // zsh asks a new user to set it up when they have no startup file at all.
    fs::write(home.join(".zshrc"), "").unwrap();
    let bin = home.join("bin");
    fs::create_dir(&bin).unwrap();
    // Neovim is "installed", and is the editor.
    program(&bin.join("nvim"), "#!/bin/sh\necho REAL-NVIM\n");
    let mut x8ai = X8ai::start_with(
        &home,
        &["proj"],
        &[
            (
                "PATH",
                format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
            ),
            ("SHELL", "/bin/zsh".to_owned()),
            ("EDITOR", "nvim".to_owned()),
        ],
    );
    x8ai.wait_for_space("proj  Untrusted");

    // On in every terminal, until Enter turns it off here.
    x8ai.keys(&format!("{CTRL_G}e"));
    x8ai.wait_for("Neovim  on · in every terminal");
    let (col, row) = x8ai.find("Neovim  on").unwrap();
    x8ai.click(col, row);
    x8ai.keys("\r");
    x8ai.wait_for("Neovim is off in proj: new shells here have no nvim. Other spaces keep it.");
    x8ai.wait_for("Neovim  off in this space");
    let spaces = fs::read_to_string(home.join(".x8ai-data/spaces.json")).unwrap();
    assert!(spaces.contains("\"off\""), "{spaces}");

    // A new shell here finds the stand-in, which says so and fails.
    x8ai.keys(&format!("{CTRL_G}t"));
    x8ai.wait_for(" 2 zsh ");
    x8ai.keys(WHICH_NVIM);
    x8ai.wait_for("OFF-NVIM");
    x8ai.keys("nvim notes.txt; echo \"EXIT:$?\"\r");
    let screen = x8ai.wait_for("EXIT:127");
    assert!(
        screen.contains("nvim: Neovim is off in proj (ws-"),
        "{screen}"
    );

    // So does the editor the file list opens.
    x8ai.keys(&format!("{CTRL_G}f"));
    x8ai.wait_for("notes.txt");
    x8ai.keys("\r");
    x8ai.wait_for(" 3 notes.txt ");
    x8ai.wait_until("the editor's stand-in", |s| {
        s.matches("Neovim is off in proj").count() == 1 && s.contains(" 3 notes.txt ")
    });

    // Another space keeps it.
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("Type a command");
    x8ai.keys("/new other\r");
    x8ai.wait_for_space("other  Untrusted");
    x8ai.keys(WHICH_NVIM);
    x8ai.wait_for("ON-NVIM");

    // On again in proj: a new shell there has it.
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("Type a command");
    x8ai.keys("/cd proj\r");
    x8ai.wait_for_space("proj  Untrusted");
    x8ai.keys(&format!("{CTRL_G}e"));
    x8ai.wait_for("Neovim  off in this space");
    let (col, row) = x8ai.find("Neovim  off").unwrap();
    x8ai.click(col, row);
    x8ai.keys("\r");
    x8ai.wait_for("Neovim is on in proj again: new shells here have nvim.");
    x8ai.wait_for("Neovim  on · in every terminal");
    x8ai.keys(&format!("{CTRL_G}t"));
    // Tab 3 is the editor's, which stays to show why it ended.
    x8ai.wait_for(" 4 zsh ");
    x8ai.keys(WHICH_NVIM);
    x8ai.wait_for("ON-NVIM");
    let spaces = fs::read_to_string(home.join(".x8ai-data/spaces.json")).unwrap();
    assert!(!spaces.contains("\"off\""), "{spaces}");

    x8ai.keys(&format!("{CTRL_G}q"));
    assert_eq!(x8ai.wait_for_exit().code, 0);
}
