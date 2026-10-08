//! `x8ai` end to end, step 3: an agent launched from the Agents panel, after
//! trusting the folder and allowing the agent; its session's worktree,
//! review, restart, stop and removal; and trust taken back.
//!
//! The agent is a script named `claude`, the program the built-in Claude Code
//! definition runs, found on the test's `PATH`, which agents get as `x8ai`
//! has it.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use common::{CTRL_G, X8ai};

fn script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn an_agent_is_trusted_allowed_run_reviewed_and_removed() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let proj = home.join("proj");
    fs::create_dir(&proj).unwrap();
    fs::write(proj.join("README.md"), "hello\n").unwrap();
    git(&proj, &["init", "-q"]);
    git(&proj, &["add", "."]);
    git(&proj, &["commit", "-qm", "first"]);

    let bin = home.join("bin");
    fs::create_dir(&bin).unwrap();
    script(
        &bin.join("claude"),
        "#!/bin/sh\n\
         echo \"FAKE-CLAUDE in $(basename \"$(dirname \"$(pwd)\")\")\"\n\
         while IFS= read -r line; do\n\
           case \"$line\" in\n\
             change) echo hi > agent.txt; echo CHANGED;;\n\
             commit) git add -A && git -c user.name=Agent -c user.email=agent@example.com commit -qm agent && echo COMMITTED;;\n\
             dirty) echo more > dirty.txt; echo DIRTIED;;\n\
             quit) echo BYE; exit 0;;\n\
           esac\n\
         done\n",
    );
    let mut x8ai = X8ai::start_with(
        &home,
        &["proj"],
        &[
            (
                "PATH",
                format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
            ),
            ("GIT_CONFIG_GLOBAL", "/dev/null".to_owned()),
        ],
    );
    x8ai.wait_for("~/proj · not trusted");

    // Ctrl-g a: the panel.
    x8ai.keys(&format!("{CTRL_G}a"));
    let screen = x8ai.wait_for("Claude Code  installed");
    assert!(screen.contains("○ not trusted"), "{screen}");
    assert!(screen.contains("each session: a worktree"), "{screen}");
    assert!(screen.contains("None yet"), "{screen}");

    // Enter: trust the folder first, then allow the agent, with its program.
    x8ai.keys("\r");
    x8ai.wait_until("trust asked", |s| {
        s.contains("Trust “proj”?") && s.contains("y trust")
    });
    x8ai.keys("y");
    let screen = x8ai.wait_until("the approval asked", |s| {
        s.contains("Allow Claude Code to work in “proj”?") && s.contains("y allow")
    });
    assert!(screen.contains("bin/claude"), "{screen}");
    assert!(screen.contains("worktree of its own"), "{screen}");
    x8ai.keys("y");

    // It runs in a worktree of its own, in a tab of its own.
    let screen = x8ai.wait_for("FAKE-CLAUDE in proj-");
    assert!(screen.contains(" 2 Claude Code "), "{screen}");
    let approvals = fs::read_to_string(home.join(".x8ai-data/agent-approvals.json")).unwrap();
    assert!(approvals.contains("claude-code"), "{approvals}");
    x8ai.keys("change\r");
    x8ai.wait_for("CHANGED");
    x8ai.keys("commit\r");
    x8ai.wait_for("COMMITTED");
    x8ai.keys("dirty\r");
    x8ai.wait_for("DIRTIED");
    // The user's working tree is not touched.
    assert!(!proj.join("agent.txt").exists());
    assert_eq!(git(&proj, &["status", "--porcelain"]), "");

    // The session, and what it changed, in a pager of its own.
    x8ai.keys(&format!("{CTRL_G}a"));
    x8ai.wait_for("Claude Code  running");
    x8ai.keys("G");
    x8ai.keys("c");
    let screen = x8ai.wait_for("1 commit · changes not committed");
    assert!(screen.contains("agent.txt"), "{screen}");
    assert!(screen.contains("dirty.txt"), "{screen}");
    assert!(screen.contains("+hi"), "{screen}");
    x8ai.keys("q");
    x8ai.wait_until("the review closed", |s| !s.contains("changes: Claude Code"));

    // The agent ends: its pane stays, and Enter runs it again.
    x8ai.keys(&format!("{CTRL_G}a"));
    x8ai.keys("G\r");
    x8ai.keys("quit\r");
    x8ai.wait_for("Claude Code exited with 0. Enter runs it again");
    x8ai.keys("\r");
    x8ai.wait_until("the agent running again", |s| {
        !s.contains("exited with 0") && s.contains("FAKE-CLAUDE in proj-")
    });

    // s stops it; the session and its worktree stay.
    x8ai.keys(&format!("{CTRL_G}a"));
    x8ai.keys("Gs");
    x8ai.wait_for("Claude Code was ended by");
    let worktrees = home.join(".x8ai/worktrees");
    let repo_dir = fs::read_dir(&worktrees)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(
        fs::read_dir(&repo_dir)
            .unwrap()
            .any(|e| e.unwrap().path().is_dir())
    );

    // d removes it, saying what goes: its uncommitted change; its branch and
    // commit stay.
    x8ai.keys("d");
    let screen = x8ai.wait_until("removal asked", |s| {
        s.contains("Remove this Claude Code session?") && s.contains("y remove")
    });
    assert!(screen.contains("not committed are discarded"), "{screen}");
    assert!(screen.contains("keeps its 1"), "{screen}");
    x8ai.keys("y");
    x8ai.wait_for("Removed. Its branch agent/claude-code/");
    assert!(
        !fs::read_dir(&repo_dir)
            .unwrap()
            .any(|e| e.unwrap().path().is_dir())
    );
    assert!(git(&proj, &["branch", "--list", "agent/*"]).contains("agent/claude-code/"));

    // A session whose pane is closed while its agent runs: the panel shows it
    // ended by itself, and it can be removed at once.
    x8ai.keys("g\r");
    x8ai.wait_until("a second session", |s| {
        s.contains(" 2 Claude Code ") && s.contains("FAKE-CLAUDE in proj-")
    });
    x8ai.keys(&format!("{CTRL_G}x"));
    x8ai.wait_until("stop asked", |s| {
        s.contains("Stop Claude Code?") && s.contains("y stop")
    });
    x8ai.keys("y");
    x8ai.keys(&format!("{CTRL_G}a"));
    x8ai.wait_for("Claude Code  stopped");
    x8ai.keys("Gd");
    x8ai.wait_until("removal asked", |s| s.contains("y remove"));
    x8ai.keys("y");
    x8ai.wait_for("Removed.");

    // t takes trust back, and the approval with it.
    x8ai.keys("t");
    x8ai.wait_until("untrust asked", |s| {
        s.contains("Stop trusting “proj”?") && s.contains("y stop trusting")
    });
    x8ai.keys("y");
    x8ai.wait_for("○ not trusted");
    let approvals = fs::read_to_string(home.join(".x8ai-data/agent-approvals.json")).unwrap();
    assert!(!approvals.contains("claude-code"), "{approvals}");

    x8ai.keys(&format!("{CTRL_G}q"));
    assert_eq!(x8ai.wait_for_exit().code, 0);
}
