//! What is on this Mac, and the commands that install what is not.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::registry::{Addon, Install, Need};

/// Where to look: the user's home and their login `PATH`.
#[derive(Debug, Clone)]
pub struct Mac {
    pub home: PathBuf,
    pub path: Option<String>,
}

/// Something an install needs that is not on the Mac.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    Homebrew,
    Git,
}

impl std::fmt::Display for Missing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Homebrew => "Homebrew is not installed. Install it from brew.sh, then try again.",
            Self::Git => "Git is not installed. Install Apple's command line tools (xcode-select --install), then try again.",
        })
    }
}

impl Mac {
    /// A program on the login `PATH`: only absolute entries, only executable files.
    pub fn find(&self, program: &str) -> Option<PathBuf> {
        self.path
            .as_deref()?
            .split(':')
            .map(Path::new)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join(program))
            .find(|candidate| is_executable(candidate))
    }

    /// Homebrew's `brew`: on the login `PATH`, or where its installer puts it.
    pub fn brew(&self) -> Option<PathBuf> {
        self.find("brew").or_else(|| {
            ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
                .into_iter()
                .map(PathBuf::from)
                .find(|p| is_executable(p))
        })
    }

    /// Homebrew's prefix: the folder above the one `brew` is in.
    pub fn brew_prefix(&self) -> Option<PathBuf> {
        self.brew()?.parent()?.parent().map(Path::to_owned)
    }

    pub fn git(&self) -> Option<PathBuf> {
        self.find("git")
            .or_else(|| Some(PathBuf::from("/usr/bin/git")).filter(|p| is_executable(p)))
    }

    /// The file a [`Need::BrewFile`] names, wherever Homebrew is.
    pub fn brew_file(&self, file: &str) -> Option<PathBuf> {
        Some(self.brew_prefix()?.join(file))
    }

    /// The folder of a Neovim configuration named `name`.
    pub fn nvim_config(&self, name: &str) -> PathBuf {
        self.home.join(".config").join(name)
    }

    /// Whether what the add-on needs is here.
    pub fn installed(&self, addon: &Addon) -> bool {
        match addon.need {
            Need::Program(program) => self.find(program).is_some(),
            Need::BrewFile(file) => self.brew_file(file).is_some_and(|f| f.is_file()),
            Need::Font(file) => [self.home.join("Library/Fonts"), "/Library/Fonts".into()]
                .iter()
                .any(|dir| dir.join(file).is_file()),
            Need::NvimConfig(name) => self.nvim_config(name).join("init.lua").is_file(),
        }
    }

    /// The commands that install those of `addons` that are not installed, in
    /// order: one `brew install` for every formula, one `brew install --cask`
    /// for every cask, then each clone. Each command is a program and its
    /// arguments, never a shell line.
    pub fn install_commands(&self, addons: &[&Addon]) -> Result<Vec<Vec<String>>, Missing> {
        let missing: Vec<&Addon> = addons
            .iter()
            .copied()
            .filter(|a| !self.installed(a))
            .collect();
        let formulae: Vec<&str> = missing
            .iter()
            .filter_map(|a| match a.install {
                Install::Formula(formula) => Some(formula),
                _ => None,
            })
            .collect();
        let casks: Vec<&str> = missing
            .iter()
            .filter_map(|a| match a.install {
                Install::Cask(cask) => Some(cask),
                _ => None,
            })
            .collect();
        let mut commands = Vec::new();
        if !formulae.is_empty() || !casks.is_empty() {
            let brew = self.brew().ok_or(Missing::Homebrew)?;
            let brew = brew.display().to_string();
            if !formulae.is_empty() {
                let mut command = vec![brew.clone(), "install".to_owned()];
                command.extend(formulae.iter().map(|f| (*f).to_owned()));
                commands.push(command);
            }
            if !casks.is_empty() {
                let mut command = vec![brew, "install".to_owned(), "--cask".to_owned()];
                command.extend(casks.iter().map(|c| (*c).to_owned()));
                commands.push(command);
            }
        }
        for addon in &missing {
            if let Install::Clone { url, config } = addon.install {
                let git = self.git().ok_or(Missing::Git)?;
                commands.push(vec![
                    git.display().to_string(),
                    "clone".to_owned(),
                    "--depth=1".to_owned(),
                    url.to_owned(),
                    self.nvim_config(config).display().to_string(),
                ]);
            }
        }
        Ok(commands)
    }
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// A command as the user reads it: the program's name, then its arguments,
/// quoted where a shell would need it (`brew install starship`).
pub fn shown(command: &[String]) -> String {
    command
        .iter()
        .enumerate()
        .map(|(i, word)| {
            let word = if i == 0 {
                Path::new(word)
                    .file_name()
                    .map_or_else(|| word.clone(), |n| n.to_string_lossy().into_owned())
            } else {
                word.clone()
            };
            if word
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_./=:@+,".contains(&b))
            {
                word
            } else {
                quote(&word)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `value` as one word for `sh` and `zsh`: single-quoted, with each `'` closed,
/// escaped and reopened.
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// The script an install runs with `/bin/sh -c`: each command shown, then run,
/// stopping at the first that fails. Every word is quoted, so nothing in a path
/// is read as shell syntax.
pub fn install_script(commands: &[Vec<String>]) -> String {
    let mut script = String::from("set -e\n");
    for command in commands {
        script.push_str(&format!(
            "printf '\\033[2m› %s\\033[0m\\n' {}\n",
            quote(&shown(command))
        ));
        script.push_str(
            &command
                .iter()
                .map(|w| quote(w))
                .collect::<Vec<_>>()
                .join(" "),
        );
        script.push('\n');
    }
    script.push_str("printf '\\n\\033[32m✓ Installed.\\033[0m This tab can be closed.\\n'\n");
    script
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::registry::{find, with_requirements};

    fn fake_mac() -> (tempfile::TempDir, Mac) {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("brew/bin");
        fs::create_dir_all(&bin).unwrap();
        for program in ["brew", "git", "nvim"] {
            let path = bin.join(program);
            fs::write(&path, "").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let home = dir.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let mac = Mac {
            home,
            path: Some(format!("relative:{}", bin.display())),
        };
        (dir, mac)
    }

    #[test]
    fn finds_what_is_installed() {
        let (dir, mac) = fake_mac();
        assert!(mac.installed(find("neovim").unwrap()));
        assert!(!mac.installed(find("starship").unwrap()));
        assert_eq!(mac.brew_prefix().unwrap(), dir.path().join("brew"));

        let highlighting = find("syntax-highlighting").unwrap();
        assert!(!mac.installed(highlighting));
        let file = dir
            .path()
            .join("brew/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "").unwrap();
        assert!(mac.installed(highlighting));
    }

    #[test]
    fn installs_only_what_is_missing_in_few_commands() {
        let (_dir, mac) = fake_mac();
        let addons = with_requirements(["lazyvim", "starship", "nerd-font"]);
        let commands = mac.install_commands(&addons).unwrap();
        let shown: Vec<String> = commands.iter().map(|c| shown(c)).collect();
        let config = mac.nvim_config("x8ai-lazyvim");
        assert_eq!(
            shown,
            [
                "brew install ripgrep fd starship".to_owned(),
                "brew install --cask font-jetbrains-mono-nerd-font".to_owned(),
                format!(
                    "git clone --depth=1 https://github.com/LazyVim/starter {}",
                    config.display()
                ),
            ]
        );
    }

    #[test]
    fn says_what_is_missing_to_install() {
        let mac = Mac {
            home: "/nonexistent".into(),
            path: Some("/nonexistent/bin".into()),
        };
        if mac.brew().is_none() {
            assert_eq!(
                mac.install_commands(&[find("starship").unwrap()]),
                Err(Missing::Homebrew)
            );
        }
        assert_eq!(mac.install_commands(&[]), Ok(Vec::new()));
    }

    #[test]
    fn the_script_quotes_every_word() {
        let script = install_script(&[vec![
            "/opt/home brew/bin/brew".into(),
            "install".into(),
            "it's; rm -rf ~".into(),
        ]]);
        assert!(script.contains(r"'/opt/home brew/bin/brew' 'install' 'it'\''s; rm -rf ~'"));
        assert!(script.starts_with("set -e\n"));
    }
}
