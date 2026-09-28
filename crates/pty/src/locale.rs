//! The locale a terminal session starts with.
//!
//! macOS terminals (Terminal.app, iTerm2) set `LANG` from the user's language and
//! region. An app launched from Finder or the Dock inherits no `LANG`, so without
//! this a session falls back to the C locale: zsh's `/etc/zprofile` picks
//! `C.UTF-8`, and bash gets no locale at all, which breaks UTF-8 input and output.

use std::path::Path;

/// The `LANG` to set when the inherited environment names no locale, or `None` to
/// leave it to the shell.
#[cfg(target_os = "macos")]
pub(crate) fn user_lang() -> Option<String> {
    let preferred = sys_locale::get_locale()?;
    posix_utf8_locale(&preferred).filter(|name| Path::new("/usr/share/locale").join(name).is_dir())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn user_lang() -> Option<String> {
    // Linux desktop sessions already export LANG; there is nothing to derive.
    None
}

/// Converts a BCP 47 tag such as `en-IN` or `zh-Hans-CN` to a POSIX UTF-8 locale
/// name such as `en_IN.UTF-8`. Returns `None` when the tag has no two-letter
/// language and region, since there is then no locale name to guess.
pub(crate) fn posix_utf8_locale(tag: &str) -> Option<String> {
    let mut subtags = tag.split(['-', '_']);
    let language = subtags.next().filter(|l| is_alpha(l, 2))?;
    let region = subtags.find(|s| is_alpha(s, 2))?;
    Some(format!(
        "{}_{}.UTF-8",
        language.to_ascii_lowercase(),
        region.to_ascii_uppercase()
    ))
}

fn is_alpha(subtag: &str, len: usize) -> bool {
    subtag.len() == len && subtag.bytes().all(|b| b.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::posix_utf8_locale;

    #[test]
    fn converts_language_and_region() {
        assert_eq!(posix_utf8_locale("en-IN").as_deref(), Some("en_IN.UTF-8"));
        assert_eq!(posix_utf8_locale("de_DE").as_deref(), Some("de_DE.UTF-8"));
        assert_eq!(posix_utf8_locale("EN-us").as_deref(), Some("en_US.UTF-8"));
    }

    #[test]
    fn skips_the_script_subtag() {
        assert_eq!(
            posix_utf8_locale("zh-Hans-CN").as_deref(),
            Some("zh_CN.UTF-8")
        );
    }

    #[test]
    fn needs_a_language_and_a_region() {
        assert_eq!(posix_utf8_locale("en"), None);
        assert_eq!(posix_utf8_locale("es-419"), None);
        assert_eq!(posix_utf8_locale(""), None);
    }
}
