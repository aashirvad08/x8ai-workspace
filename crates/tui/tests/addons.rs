//! `x8ai` end to end, step 4: add-ons. One that is installed is added to the
//! space at once; one that is not asks first, showing the exact install
//! command (declined here: nothing is installed); an added one is removed;
//! and a space's add-ons are given to another with `/share`. They are the
//! app's own (spaces.json).

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use common::{CTRL_G, X8ai};

#[test]
fn addons_are_added_removed_and_shared() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    fs::create_dir(home.join("proj")).unwrap();
    let bin = home.join("bin");
    fs::create_dir(&bin).unwrap();
    // ripgrep is "installed": its program is on the PATH.
    fs::write(bin.join("rg"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(bin.join("rg"), fs::Permissions::from_mode(0o755)).unwrap();
    let mut x8ai = X8ai::start_with(
        &home,
        &["proj"],
        &[(
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
        )],
    );
    x8ai.wait_for("~/proj · not trusted");

    x8ai.keys(&format!("{CTRL_G}e"));
    let screen = x8ai.wait_for(" ADD-ONS · proj");
    assert!(
        screen.contains("ripgrep  installed: Enter adds it"),
        "{screen}"
    );

    // An installed one is added at once.
    let (col, row) = x8ai.find("ripgrep  installed").unwrap();
    x8ai.click(col, row);
    x8ai.keys("\r");
    x8ai.wait_for("ripgrep is added to proj.");
    x8ai.wait_for("ripgrep  on");
    let spaces = fs::read_to_string(home.join(".x8ai-data/spaces.json")).unwrap();
    assert!(spaces.contains("ripgrep"), "{spaces}");

    // One that is not installed asks first, with the exact command; or says
    // Homebrew is missing, where it is.
    let (col, row) = x8ai.find("fzf").unwrap();
    x8ai.click(col, row);
    x8ai.keys("\r");
    let screen = x8ai.wait_until("asked or refused", |s| {
        s.contains("y install") || s.contains("Homebrew is not installed. Install it from brew.sh")
    });
    if screen.contains("y install") {
        assert!(screen.contains("Add fzf to proj?"), "{screen}");
        assert!(screen.contains("install fzf"), "{screen}");
        x8ai.keys("n");
        x8ai.wait_until("declined", |s| !s.contains("y install"));
    }

    // A space's add-ons go to another space with /share.
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("W E L C O M E");
    x8ai.keys("/new other\r");
    x8ai.wait_for("~/Workspaces/other · not trusted");
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("W E L C O M E");
    x8ai.keys("/cd proj\r");
    x8ai.wait_for("~/proj · not trusted");
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("W E L C O M E");
    x8ai.keys("/share other\r");
    let screen = x8ai.wait_until("shared", |s| s.contains("now has proj's add-ons: ripgrep."));
    assert!(screen.contains("ws-"), "{screen}");

    // Removed from this space; the other keeps it. The panel is still open:
    // a space keeps its sidebar.
    x8ai.keys("\x1b");
    x8ai.wait_until("back with the panel", |s| {
        s.contains("~/proj · not trusted")
            && s.contains(" ADD-ONS · proj")
            && s.contains("ripgrep  on")
    });
    let (col, row) = x8ai.find("ripgrep  on").unwrap();
    x8ai.click(col, row);
    x8ai.keys("d");
    x8ai.wait_for("ripgrep is removed from proj.");
    x8ai.wait_for("ripgrep  installed: Enter adds it");
    let spaces = fs::read_to_string(home.join(".x8ai-data/spaces.json")).unwrap();
    assert_eq!(spaces.matches("ripgrep").count(), 1, "{spaces}");

    x8ai.keys(&format!("{CTRL_G}q"));
    assert_eq!(x8ai.wait_for_exit().code, 0);
}
