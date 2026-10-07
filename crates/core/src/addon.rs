//! Add-ons: well-known command-line tools (a prompt, syntax highlighting, an
//! editor setup, a font) a workspace's terminals can use, and the spaces they
//! are added to. See `docs/decisions/0019-add-ons.md`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A space: an open folder, or the workspace with no folder open. Each has an id
/// of its own, made when it is first opened, that never changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpaceInfo {
    /// `ws-` and six letters or digits (`ws-k3f9qa`).
    pub id: String,
    /// The folder's name, or `Home` for the workspace with no folder.
    pub name: String,
    /// The folder; `None` for the workspace with no folder.
    pub root: Option<String>,
    /// The add-ons added to it, by id.
    pub addons: Vec<String>,
}

/// What an add-on is for, to group them in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AddonGroup {
    Shell,
    Editor,
    Tools,
    Look,
}

/// Where an added add-on makes a difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AddonReach {
    /// Only in the terminals of the spaces it is added to: shell setup, the
    /// editor's configuration, the terminal font.
    Space,
    /// A program that, once installed, is on the Mac's `PATH` in every
    /// terminal. Adding it to a space installs it and keeps it in the space's set.
    Mac,
}

/// An add-on, and its state in the open space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AddonStatus {
    /// `starship`, `lazyvim`, `nerd-font`, …
    pub id: String,
    pub name: String,
    /// One line: what it does.
    pub description: String,
    /// How to use it once added (`z <folder>`), when there is something to type.
    pub usage: Option<String>,
    pub group: AddonGroup,
    pub reach: AddonReach,
    /// Add-ons adding this one adds too.
    pub requires: Vec<String>,
    /// Whether what it needs is on this Mac.
    pub installed: bool,
    /// Added to the open space.
    pub added: bool,
    /// It reads the folder (Starship runs `git` there), so it is on only in a
    /// trusted folder.
    pub needs_trust: bool,
    /// Added, and on in the space's new terminals: what it needs is installed,
    /// the shell can take it, and the folder is trusted if it must be.
    pub active: bool,
    /// The user's own shell startup files already turn it on, so it is in every
    /// terminal whether added or not.
    pub in_your_shell: bool,
    /// What adding it runs while it is not installed, one command per line, as
    /// the confirmation shows them. Empty once installed.
    pub install: Vec<String>,
}

/// Every add-on, in the open space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AddonList {
    pub space: SpaceInfo,
    /// Homebrew was found; add-ons install with it.
    pub homebrew: bool,
    /// The user's shell. Shell add-ons need zsh.
    pub shell: String,
    pub shell_supported: bool,
    pub addons: Vec<AddonStatus>,
}

/// What adding an add-on did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum AddonAddResult {
    /// Everything it needs was installed: it is added, with what it requires.
    Added { list: AddonList },
    /// The user confirmed installing it. The install runs in a terminal started
    /// with this token (`addon_install`), once; the add-on is added when it ends
    /// well.
    Install { token: u32, name: String },
    /// The user did not confirm the install. Nothing changed.
    Cancelled,
}
