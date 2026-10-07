//! The built-in add-ons. The list is part of the app: nothing outside it (the
//! webview, a store file, a folder) can add one, or change what one runs.

use x8ai_core::addon::{AddonGroup, AddonReach};

/// An add-on, as the app defines it.
#[derive(Debug)]
pub struct Addon {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub usage: Option<&'static str>,
    pub group: AddonGroup,
    pub reach: AddonReach,
    /// Added with it, and installed first.
    pub requires: &'static [&'static str],
    /// It runs programs in the folder (Starship runs `git` there, and a
    /// repository's own configuration can make `git` run commands), so it is on
    /// only in a trusted folder.
    pub needs_trust: bool,
    /// What must be on the Mac for it to work.
    pub need: Need,
    pub install: Install,
    /// What turns it on in a space's zsh, after the user's own `.zshrc`. Each
    /// does nothing if what it needs is missing, or if it is already on. `{file}`
    /// stands for the file of a [`Need::BrewFile`], quoted.
    pub zshrc: Option<&'static str>,
    /// Variables the space's terminals start with.
    pub env: &'static [(&'static str, &'static str)],
    /// Text that, on a line of the user's own startup files that is not a
    /// comment, shows they already turn it on in every terminal.
    pub markers: &'static [&'static str],
}

/// What must be on the Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// A program on the login `PATH`.
    Program(&'static str),
    /// A file under Homebrew's prefix (`share/zsh-autosuggestions/…`).
    BrewFile(&'static str),
    /// A font file in `~/Library/Fonts` or `/Library/Fonts`.
    Font(&'static str),
    /// A Neovim configuration in `~/.config/<name>` (one with an `init.lua`).
    NvimConfig(&'static str),
}

/// How it is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Install {
    /// `brew install <formula>`
    Formula(&'static str),
    /// `brew install --cask <cask>`
    Cask(&'static str),
    /// `git clone --depth=1 <url> ~/.config/<config>`
    Clone {
        url: &'static str,
        config: &'static str,
    },
}

/// LazyVim's own configuration: Neovim reads it only where `NVIM_APPNAME` names
/// it, so the user's `~/.config/nvim` is neither used nor changed.
pub const LAZYVIM_CONFIG: &str = "x8ai-lazyvim";

/// Every add-on, in the order the list shows them. Shell add-ons are also
/// turned on in this order, which is why syntax highlighting, which must come
/// last, is last.
pub const ADDONS: &[Addon] = &[
    Addon {
        id: "starship",
        name: "Starship",
        description: "A fast prompt that shows the folder, Git branch and status, and language versions.",
        usage: None,
        group: AddonGroup::Shell,
        reach: AddonReach::Space,
        requires: &[],
        needs_trust: true,
        need: Need::Program("starship"),
        install: Install::Formula("starship"),
        zshrc: Some(
            "(( $+commands[starship] && ! $+functions[prompt_starship_precmd] )) && eval \"$(starship init zsh)\"",
        ),
        env: &[],
        markers: &["starship init"],
    },
    Addon {
        id: "autosuggestions",
        name: "Autosuggestions",
        description: "Suggests the rest of a command from your history as you type; → takes it.",
        usage: None,
        group: AddonGroup::Shell,
        reach: AddonReach::Space,
        requires: &[],
        needs_trust: false,
        need: Need::BrewFile("share/zsh-autosuggestions/zsh-autosuggestions.zsh"),
        install: Install::Formula("zsh-autosuggestions"),
        zshrc: Some(
            "(( ! $+functions[_zsh_autosuggest_start] )) && [[ -r {file} ]] && builtin source {file}",
        ),
        env: &[],
        markers: &["zsh-autosuggestions.zsh"],
    },
    Addon {
        id: "fzf",
        name: "fzf",
        description: "Fuzzy finder: ⌃R searches your history, ⌃T finds files, ⌥C goes to folders.",
        usage: None,
        group: AddonGroup::Shell,
        reach: AddonReach::Space,
        requires: &[],
        needs_trust: false,
        need: Need::Program("fzf"),
        install: Install::Formula("fzf"),
        zshrc: Some(
            "(( $+commands[fzf] && ! $+functions[fzf-history-widget] )) && eval \"$(fzf --zsh)\"",
        ),
        env: &[],
        markers: &["fzf --zsh", ".fzf.zsh"],
    },
    Addon {
        id: "zoxide",
        name: "zoxide",
        description: "A smarter cd: jumps to the folder you use most that matches what you type.",
        usage: Some("z <part of a folder name>"),
        group: AddonGroup::Shell,
        reach: AddonReach::Space,
        requires: &[],
        needs_trust: false,
        need: Need::Program("zoxide"),
        install: Install::Formula("zoxide"),
        zshrc: Some(
            "(( $+commands[zoxide] && ! $+functions[__zoxide_z] )) && eval \"$(zoxide init zsh)\"",
        ),
        env: &[],
        markers: &["zoxide init"],
    },
    Addon {
        id: "eza",
        name: "eza",
        description: "ls with colours, icons and Git status: ls, ll and tree use it here.",
        usage: Some("ls · ll · tree"),
        group: AddonGroup::Shell,
        reach: AddonReach::Space,
        requires: &[],
        needs_trust: false,
        need: Need::Program("eza"),
        install: Install::Formula("eza"),
        zshrc: Some(concat!(
            "if (( $+commands[eza] )); then\n",
            "  alias ls='eza --group-directories-first --icons=auto'\n",
            "  alias ll='eza -l --git --group-directories-first --icons=auto'\n",
            "  alias tree='eza --tree --icons=auto'\n",
            "fi",
        )),
        env: &[],
        markers: &["alias ls='eza", "alias ls=\"eza", "alias ls=eza"],
    },
    Addon {
        id: "syntax-highlighting",
        name: "Syntax highlighting",
        description: "Colours commands as you type: green when they exist, red when they don't.",
        usage: None,
        group: AddonGroup::Shell,
        reach: AddonReach::Space,
        requires: &[],
        needs_trust: false,
        need: Need::BrewFile("share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh"),
        install: Install::Formula("zsh-syntax-highlighting"),
        zshrc: Some(
            "(( ! ${+ZSH_HIGHLIGHT_VERSION} )) && [[ -r {file} ]] && builtin source {file}",
        ),
        env: &[],
        markers: &["zsh-syntax-highlighting.zsh"],
    },
    Addon {
        id: "neovim",
        name: "Neovim",
        description: "The modern Vim, in the terminal.",
        usage: Some("nvim <file>"),
        group: AddonGroup::Editor,
        reach: AddonReach::Mac,
        requires: &[],
        needs_trust: false,
        need: Need::Program("nvim"),
        install: Install::Formula("neovim"),
        zshrc: None,
        env: &[],
        markers: &[],
    },
    Addon {
        id: "lazyvim",
        name: "LazyVim",
        description: "Neovim set up like an IDE: file tree, fuzzy search, language servers. A setup of its own, apart from yours.",
        usage: Some("nvim"),
        group: AddonGroup::Editor,
        reach: AddonReach::Space,
        requires: &["neovim", "ripgrep", "fd"],
        needs_trust: false,
        need: Need::NvimConfig(LAZYVIM_CONFIG),
        install: Install::Clone {
            url: "https://github.com/LazyVim/starter",
            config: LAZYVIM_CONFIG,
        },
        zshrc: None,
        env: &[("NVIM_APPNAME", LAZYVIM_CONFIG)],
        markers: &[],
    },
    Addon {
        id: "ripgrep",
        name: "ripgrep",
        description: "Searches file contents fast, skipping what .gitignore lists.",
        usage: Some("rg <text>"),
        group: AddonGroup::Tools,
        reach: AddonReach::Mac,
        requires: &[],
        needs_trust: false,
        need: Need::Program("rg"),
        install: Install::Formula("ripgrep"),
        zshrc: None,
        env: &[],
        markers: &[],
    },
    Addon {
        id: "fd",
        name: "fd",
        description: "Finds files by name fast, skipping what .gitignore lists.",
        usage: Some("fd <name>"),
        group: AddonGroup::Tools,
        reach: AddonReach::Mac,
        requires: &[],
        needs_trust: false,
        need: Need::Program("fd"),
        install: Install::Formula("fd"),
        zshrc: None,
        env: &[],
        markers: &[],
    },
    Addon {
        id: "lazygit",
        name: "lazygit",
        description: "A terminal interface for Git: stage, commit, branch and rebase with a few keys.",
        usage: Some("lazygit"),
        group: AddonGroup::Tools,
        reach: AddonReach::Mac,
        requires: &[],
        needs_trust: false,
        need: Need::Program("lazygit"),
        install: Install::Formula("lazygit"),
        zshrc: None,
        env: &[],
        markers: &[],
    },
    Addon {
        id: "bat",
        name: "bat",
        description: "cat with syntax highlighting, line numbers and Git changes.",
        usage: Some("bat <file>"),
        group: AddonGroup::Tools,
        reach: AddonReach::Mac,
        requires: &[],
        needs_trust: false,
        need: Need::Program("bat"),
        install: Install::Formula("bat"),
        zshrc: None,
        env: &[],
        markers: &[],
    },
    Addon {
        id: "memray",
        name: "Memray",
        description: "A memory profiler for Python: shows what allocates, and where.",
        usage: Some("memray run <script.py>"),
        group: AddonGroup::Tools,
        reach: AddonReach::Mac,
        requires: &[],
        needs_trust: false,
        need: Need::Program("memray"),
        install: Install::Formula("memray"),
        zshrc: None,
        env: &[],
        markers: &[],
    },
    Addon {
        id: "nerd-font",
        name: "JetBrains Mono Nerd Font",
        description: "A clear coding font with icons, for this space's terminals. Starship, eza and LazyVim draw their icons with it.",
        usage: None,
        group: AddonGroup::Look,
        reach: AddonReach::Space,
        requires: &[],
        needs_trust: false,
        need: Need::Font("JetBrainsMonoNerdFontMono-Regular.ttf"),
        install: Install::Cask("font-jetbrains-mono-nerd-font"),
        zshrc: None,
        env: &[],
        markers: &[],
    },
];

pub fn find(id: &str) -> Option<&'static Addon> {
    ADDONS.iter().find(|a| a.id == id)
}

/// The add-ons with what each requires, each once, requirements first.
/// Unknown ids are left out.
pub fn with_requirements<'a>(ids: impl IntoIterator<Item = &'a str>) -> Vec<&'static Addon> {
    fn visit(addon: &'static Addon, out: &mut Vec<&'static Addon>) {
        if out.iter().any(|a| a.id == addon.id) {
            return;
        }
        for required in addon.requires {
            if let Some(required) = find(required) {
                visit(required, out);
            }
        }
        out.push(addon);
    }
    let mut out = Vec::new();
    for id in ids {
        if let Some(addon) = find(id) {
            visit(addon, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_requirements_exist() {
        for (i, addon) in ADDONS.iter().enumerate() {
            assert!(
                ADDONS[..i].iter().all(|a| a.id != addon.id),
                "{} twice",
                addon.id
            );
            for required in addon.requires {
                assert!(find(required).is_some(), "{} requires {required}", addon.id);
            }
        }
    }

    #[test]
    fn requirements_come_first_and_once() {
        let ids: Vec<_> = with_requirements(["lazyvim", "fd", "nope"])
            .iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(ids, ["neovim", "ripgrep", "fd", "lazyvim"]);
    }

    #[test]
    fn syntax_highlighting_is_turned_on_last() {
        let last = ADDONS.iter().rev().find(|a| a.zshrc.is_some()).unwrap();
        assert_eq!(last.id, "syntax-highlighting");
    }
}
