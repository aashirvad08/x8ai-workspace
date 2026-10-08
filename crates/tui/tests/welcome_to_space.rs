//! `x8ai` end to end, step 1: the Welcome screen, spaces and their shells.

mod common;

use std::time::Duration;

use common::{CTRL_G, X8ai};

#[test]
fn welcome_new_space_shell_and_back() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let mut x8ai = X8ai::start(&home, &[]);

    let screen = x8ai.wait_for("W E L C O M E ,");
    assert!(screen.contains("S I R"), "{screen}");
    // Nothing opened yet: Esc goes to the workspace with no folder.
    assert!(screen.contains("Home no folder open"), "{screen}");

    // `/new` makes the space and opens its shell, in its folder, with its id.
    x8ai.keys("/new demo\r");
    let screen = x8ai.wait_for("~/Workspaces/demo · not trusted");
    assert!(screen.contains(" x8ai demo   1 sh "), "{screen}");
    x8ai.keys("echo \"in:$(pwd):$X8AI_SPACE:$((40 + 2))\"\r");
    let screen = x8ai.wait_for(":42");
    let demo = home.join("Workspaces/demo");
    assert!(
        screen.contains(&format!("in:{}:ws-", demo.display())),
        "{screen}"
    );

    // Ctrl-g h: the Welcome over the space, which keeps running.
    x8ai.keys(&format!("{CTRL_G}h"));
    let screen = x8ai.wait_for("Space demo");
    assert!(screen.contains("~/Workspaces/demo · ws-"), "{screen}");
    assert!(screen.contains("not trusted"), "{screen}");

    // `/home`: the workspace with no folder, a shell in the home folder.
    x8ai.keys("/home\r");
    x8ai.wait_for("~ · no folder");
    x8ai.keys("echo \"home:$(pwd):$((6 * 7))\"\r");
    x8ai.wait_for(&format!("home:{}:42", home.display()));

    // Back to demo by its name: its shell is as it was left.
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("Home no folder open");
    x8ai.keys("/cd dem\r");
    let screen = x8ai.wait_for(" x8ai demo ");
    assert!(screen.contains("in:"), "{screen}");

    // A folder that is not there is explained, not opened.
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("Space demo");
    x8ai.keys("/cd ~/nowhere\r");
    x8ai.wait_for("There is no folder ~/nowhere.");

    // Quitting with a program running asks first.
    x8ai.keys("\x1b");
    x8ai.wait_for(" x8ai demo ");
    x8ai.keys("sleep 60\r");
    // Give the shell a moment to start the job in the foreground.
    std::thread::sleep(Duration::from_millis(500));
    x8ai.keys(&format!("{CTRL_G}q"));
    let screen = x8ai.wait_for("Quit x8ai?");
    assert!(screen.contains("Still running: demo (sh)"), "{screen}");
    x8ai.keys("y");
    let exit = x8ai.wait_for_exit();
    assert_eq!(exit.code, 0, "{exit:?}");

    // The space is remembered where the app keeps its recent spaces.
    let recent = std::fs::read_to_string(home.join(".x8ai-data/recent-workspaces.json")).unwrap();
    assert!(recent.contains(&demo.display().to_string()), "{recent}");
}

#[test]
fn a_folder_given_on_the_command_line_opens_at_once() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    std::fs::create_dir(home.join("app")).unwrap();
    let mut x8ai = X8ai::start(&home, &["app"]);
    x8ai.wait_for("~/app · not trusted");
    x8ai.keys("echo \"at:$(pwd)\"\r");
    x8ai.wait_for(&format!("at:{}", home.join("app").display()));
    // The shell ends well: its pane closes, and with the last one gone the
    // Welcome says so; Esc starts a new shell.
    x8ai.keys("exit\r");
    x8ai.wait_for("app has no terminal open. Press Esc to start a new one.");
    x8ai.keys("\x1b");
    x8ai.wait_for("~/app · not trusted");
    x8ai.keys("echo \"again:$((1 + 1))\"\r");
    x8ai.wait_for("again:2");
    x8ai.keys(&format!("{CTRL_G}q"));
    assert_eq!(x8ai.wait_for_exit().code, 0);
}
