//! Add-ons (ADR 0019): well-known command-line tools a space's terminals can use,
//! installed with Homebrew when the user adds one, and turned on only in the
//! spaces it is added to.
//!
//! This crate is the list ([`ADDONS`]), what is on the Mac ([`Mac`]), the
//! commands an install runs ([`Mac::install_commands`], [`install_script`]) and
//! the zsh setup of a space's terminals ([`terminal_setup`]). It runs nothing:
//! the desktop host runs an install in a terminal the user watches, once they
//! confirm it, and starts each terminal with the setup.

#![forbid(unsafe_code)]

mod mac;
mod registry;
mod shell;

pub use mac::{Mac, Missing, install_script, quote, shown};
pub use registry::{ADDONS, Addon, Install, LAZYVIM_CONFIG, Need, find, with_requirements};
pub use shell::{TerminalSetup, in_your_shell, terminal_setup};
