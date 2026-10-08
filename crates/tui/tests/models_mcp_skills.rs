//! `x8ai` end to end, step 4: a provider's key and a model id saved in the
//! Models panel, an MCP server with a secret in the MCP panel, a skill written
//! in the Catalog; each chosen for Claude Code's next launch, asked about once,
//! and reaching the agent: its model, the key in its environment, the skill in
//! its system prompt, and the MCP server, which answers through `x8ai`'s
//! bridge with its secret.
//!
//! Secrets go to a file (`X8AI_TEST_SECRETS`, debug builds only), never the
//! user's Keychain. The agent is a Python script named `claude` that does what
//! Claude Code does with `--mcp-config`: it starts the bridge and talks to the
//! server through it.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{CTRL_G, X8ai};

fn script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// The test's own python3, by its absolute path, for the stand-in agent.
fn python() -> PathBuf {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|dir| Path::new(dir).join("python3"))
        .find(|p| p.is_file())
        .expect("python3 on PATH")
}

const AGENT: &str = r#"
import json, os, subprocess, sys
args = sys.argv[1:]
def value(flag):
    return args[args.index(flag) + 1] if flag in args else None
print("MODEL:%s" % value("--model"), flush=True)
print("KEY:%s" % ("set" if os.environ.get("ANTHROPIC_API_KEY") else "none"), flush=True)
prompt = value("--append-system-prompt") or ""
print("SKILL:%s" % ("zebra" if "ZEBRA" in prompt else "none"), flush=True)
config = value("--mcp-config")
if config:
    for name, server in json.loads(config)["mcpServers"].items():
        bridge = subprocess.Popen([server["command"]] + server["args"], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        bridge.stdin.write(b"ping\n")
        bridge.stdin.flush()
        print("MCP:%s" % bridge.stdout.readline().decode().strip(), flush=True)
        bridge.stdin.close()
        bridge.wait()
print("FAKE-CLAUDE-DONE", flush=True)
sys.stdin.readline()
"#;

#[test]
fn models_mcp_servers_and_skills_reach_the_agent() {
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
        &format!("#!{}\n{AGENT}", python().display()),
    );
    script(
        &bin.join("echo-mcp"),
        "#!/bin/sh\nwhile IFS= read -r line; do echo \"ECHO:$line:token=$TOKEN\"; done\n",
    );
    let secrets = home.join("secrets.json");
    // A short folder for the MCP sockets: the temporary home is too long for
    // a socket's path.
    let sockets = tempfile::Builder::new()
        .prefix("x8")
        .tempdir_in("/tmp")
        .unwrap();
    let mut x8ai = X8ai::start_with(
        &home,
        &["proj"],
        &[
            (
                "PATH",
                format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
            ),
            ("GIT_CONFIG_GLOBAL", "/dev/null".to_owned()),
            ("X8AI_TEST_SECRETS", secrets.display().to_string()),
            (
                "X8AI_TEST_MCP_SOCKETS",
                sockets.path().join("s").display().to_string(),
            ),
        ],
    );
    x8ai.wait_for("~/proj · not trusted");

    // Models: Anthropic's key, saved and never shown, and a model id of ours.
    x8ai.keys(&format!("{CTRL_G}m"));
    x8ai.wait_for("no key: s saves one");
    x8ai.keys("s");
    x8ai.wait_until("the key form", |s| {
        s.contains("Anthropic's API key") && s.contains("enter save")
    });
    x8ai.keys("sk-ant-test-0000\r");
    let screen = x8ai.wait_for("Anthropic's key is saved in your Keychain.");
    assert!(!screen.contains("sk-ant-test"), "{screen}");
    assert!(
        fs::read_to_string(&secrets)
            .unwrap()
            .contains("sk-ant-test-0000")
    );
    x8ai.keys("a");
    x8ai.wait_until("the model form", |s| {
        s.contains("Add a model id to Anthropic") && s.contains("enter save")
    });
    x8ai.keys("claude-test-1\r");
    x8ai.wait_for("claude-test-1 added.");
    let (col, row) = x8ai.find("claude-test-1").unwrap();
    x8ai.click(col, row);
    x8ai.keys("\r");
    x8ai.wait_until("which agent", |s| {
        s.contains("For which agent's next launch?") && s.contains("enter choose")
    });
    x8ai.keys("\r");
    x8ai.wait_for("Claude Code's next session uses anthropic · claude-test-1.");

    // MCP: a stdio server with a secret, chosen at launch.
    x8ai.keys(&format!("{CTRL_G}u"));
    x8ai.wait_for("None yet: n adds one.");
    x8ai.keys("n");
    x8ai.wait_until("the server form", |s| {
        s.contains("A new MCP server") && s.contains("enter save")
    });
    x8ai.keys("Echo\t\t");
    x8ai.keys(&bin.join("echo-mcp").display().to_string());
    x8ai.keys("\t\tTOKEN\r");
    x8ai.wait_for("Echo is saved. s saves its secret(s): TOKEN.");
    x8ai.keys("s");
    x8ai.wait_until("the secret form", |s| {
        s.contains("Echo: TOKEN") && s.contains("enter save")
    });
    x8ai.keys("tok-123\r");
    x8ai.wait_for("TOKEN is saved in your Keychain.");
    x8ai.wait_for("Echo  ready · stdio");
    x8ai.keys("l");
    x8ai.wait_until("which agent", |s| {
        s.contains("For which agent's next launch?") && s.contains("enter choose")
    });
    x8ai.keys("\r");
    x8ai.wait_for("Claude Code's next session gets the MCP server echo.");

    // A skill, written in the Catalog, found with its filter, chosen at launch.
    x8ai.keys(&format!("{CTRL_G}k"));
    x8ai.wait_for(" CATALOG");
    x8ai.keys("n");
    x8ai.wait_until("the skill form", |s| {
        s.contains("A new skill") && s.contains("enter save")
    });
    x8ai.keys("Zebra rule\tWrite a failing test first. ZEBRA\r");
    x8ai.wait_for("The skill Zebra rule is saved.");
    x8ai.keys("/zebra\r");
    x8ai.wait_for("/zebra");
    x8ai.keys("\r");
    x8ai.wait_for("Claude Code's next session gets the skill zebra-rule.");

    // The launch: everything chosen, asked about once, and given to the agent.
    x8ai.keys(&format!("{CTRL_G}a"));
    x8ai.wait_for("→ claude-test-1 · 1 MCP · 1 skill");
    x8ai.keys("g\r");
    x8ai.wait_until("trust asked", |s| {
        s.contains("Trust “proj”?") && s.contains("y trust")
    });
    x8ai.keys("y");
    let screen = x8ai.wait_until("the approval", |s| s.contains("y allow"));
    assert!(
        screen.contains("Model: claude-test-1 from Anthropic"),
        "{screen}"
    );
    assert!(screen.contains("MCP server Echo (stdio)"), "{screen}");
    assert!(screen.contains("Skills: Zebra rule"), "{screen}");
    x8ai.keys("y");
    let screen = x8ai.wait_for("FAKE-CLAUDE-DONE");
    assert!(screen.contains("MODEL:claude-test-1"), "{screen}");
    assert!(screen.contains("KEY:set"), "{screen}");
    assert!(screen.contains("SKILL:zebra"), "{screen}");
    assert!(screen.contains("MCP:ECHO:ping:token=tok-123"), "{screen}");

    x8ai.keys(&format!("{CTRL_G}q"));
    x8ai.wait_until("quit asked", |s| s.contains("y quit"));
    x8ai.keys("y");
    assert_eq!(x8ai.wait_for_exit().code, 0);
    // Its MCP sockets go with it.
    assert!(!sockets.path().join("s").exists());
}
