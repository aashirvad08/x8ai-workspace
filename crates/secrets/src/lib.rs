//! Provider credentials (docs/models.md, ADR 0014).
//!
//! A credential lives in the macOS Keychain, as a generic password of this app,
//! and nowhere else: not in a file the app writes, not in a worktree, not in the
//! webview. The webview can store one (it sends the key once, when the user types
//! it) and remove one, and learn whether one exists; it can never read one back.
//! The native side reads a credential only to start an agent session that uses
//! it, and places it only in that agent's environment.
//!
//! [`SecretValue`] holds a credential in memory. It has no `Display`, no
//! `Serialize`, and its `Debug` is redacted, so it cannot end up in a log line, an
//! error message or an IPC response by accident. No Tauri dependency.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, PoisonError};

/// Longest credential accepted. Real API keys are far shorter.
pub const MAX_SECRET_BYTES: usize = 4096;

/// A credential. Deliberately hard to leak: see the module documentation.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// Checks and wraps a credential the user entered: not empty, not too long,
    /// one line, no control characters. Surrounding whitespace is trimmed (a
    /// pasted key often carries a newline).
    pub fn new(value: &str) -> Result<Self, Error> {
        let value = value.trim();
        if value.is_empty() {
            return Err(Error::Invalid("the key is empty"));
        }
        if value.len() > MAX_SECRET_BYTES {
            return Err(Error::Invalid("the key is too long"));
        }
        if value.chars().any(char::is_control) {
            return Err(Error::Invalid("the key contains control characters"));
        }
        Ok(Self(value.to_owned()))
    }

    /// The credential itself, for the one place that needs it: an agent's
    /// environment.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

/// Why a secret operation failed. Never contains a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("the Keychain refused: {0}")]
    Keychain(String),
    #[error("there is no secret store on this platform yet")]
    Unsupported,
}

/// Where credentials are kept, by account name (a provider id).
pub trait SecretStore: Send + Sync {
    fn set(&self, account: &str, value: &SecretValue) -> Result<(), Error>;
    /// The credential, or `None` if there is none.
    fn get(&self, account: &str) -> Result<Option<SecretValue>, Error>;
    /// Whether a credential exists, without reading it.
    fn contains(&self, account: &str) -> Result<bool, Error>;
    /// Removes the credential; removing one that does not exist is not an error.
    fn remove(&self, account: &str) -> Result<(), Error>;
}

/// The login Keychain, one generic password per account, all under `service`.
#[derive(Debug, Clone)]
#[cfg_attr(
    not(target_os = "macos"),
    expect(dead_code, reason = "no secret store on this platform yet")
)]
pub struct Keychain {
    service: String,
    label: String,
}

impl Keychain {
    /// `service` groups the app's items (`com.x8ai.workspace.providers`); `label`
    /// is what Keychain Access shows for them.
    pub fn new(service: &str, label: &str) -> Self {
        Self {
            service: service.to_owned(),
            label: label.to_owned(),
        }
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use security_framework::item::{ItemClass, ItemSearchOptions};
    use security_framework::passwords::{
        PasswordOptions, delete_generic_password, generic_password, set_generic_password_options,
    };

    use super::{Error, Keychain, SecretStore, SecretValue};

    /// `errSecItemNotFound`.
    const NOT_FOUND: i32 = -25300;

    fn keychain_error(error: &security_framework::base::Error) -> Error {
        // The message describes the status code only; it never contains data.
        Error::Keychain(format!(
            "{} ({})",
            error.message().unwrap_or_default(),
            error.code()
        ))
    }

    impl SecretStore for Keychain {
        fn set(&self, account: &str, value: &SecretValue) -> Result<(), Error> {
            let mut options = PasswordOptions::new_generic_password(&self.service, account);
            options.set_label(&format!("{} ({account})", self.label));
            set_generic_password_options(value.expose().as_bytes(), options)
                .map_err(|e| keychain_error(&e))
        }

        fn get(&self, account: &str) -> Result<Option<SecretValue>, Error> {
            match generic_password(PasswordOptions::new_generic_password(
                &self.service,
                account,
            )) {
                Ok(bytes) => {
                    let text = String::from_utf8(bytes)
                        .map_err(|_| Error::Invalid("the stored key is not text"))?;
                    SecretValue::new(&text).map(Some)
                }
                Err(error) if error.code() == NOT_FOUND => Ok(None),
                Err(error) => Err(keychain_error(&error)),
            }
        }

        fn contains(&self, account: &str) -> Result<bool, Error> {
            let found = ItemSearchOptions::new()
                .class(ItemClass::generic_password())
                .service(&self.service)
                .account(account)
                .load_attributes(true)
                .search();
            match found {
                Ok(items) => Ok(!items.is_empty()),
                Err(error) if error.code() == NOT_FOUND => Ok(false),
                Err(error) => Err(keychain_error(&error)),
            }
        }

        fn remove(&self, account: &str) -> Result<(), Error> {
            match delete_generic_password(&self.service, account) {
                Ok(()) => Ok(()),
                Err(error) if error.code() == NOT_FOUND => Ok(()),
                Err(error) => Err(keychain_error(&error)),
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl SecretStore for Keychain {
    fn set(&self, _: &str, _: &SecretValue) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    fn get(&self, _: &str) -> Result<Option<SecretValue>, Error> {
        Err(Error::Unsupported)
    }
    fn contains(&self, _: &str) -> Result<bool, Error> {
        Ok(false)
    }
    fn remove(&self, _: &str) -> Result<(), Error> {
        Ok(())
    }
}

/// Credentials in memory only, for tests and for a machine without a store.
#[derive(Debug, Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<String, SecretValue>>,
}

impl SecretStore for MemoryStore {
    fn set(&self, account: &str, value: &SecretValue) -> Result<(), Error> {
        self.lock().insert(account.to_owned(), value.clone());
        Ok(())
    }
    fn get(&self, account: &str) -> Result<Option<SecretValue>, Error> {
        Ok(self.lock().get(account).cloned())
    }
    fn contains(&self, account: &str) -> Result<bool, Error> {
        Ok(self.lock().contains_key(account))
    }
    fn remove(&self, account: &str) -> Result<(), Error> {
        self.lock().remove(account);
        Ok(())
    }
}

impl MemoryStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, SecretValue>> {
        self.values.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_never_printed() {
        let secret = SecretValue::new("sk-test-not-a-real-key").unwrap();
        assert_eq!(format!("{secret:?}"), "SecretValue(<redacted>)");
        assert!(!format!("{:?}", Some(secret.clone())).contains("sk-test"));
        assert_eq!(secret.expose(), "sk-test-not-a-real-key");
    }

    #[test]
    fn keys_are_checked_and_trimmed() {
        assert_eq!(
            SecretValue::new("  key-with-newline\n").unwrap().expose(),
            "key-with-newline"
        );
        assert!(SecretValue::new("   ").is_err());
        assert!(SecretValue::new("two\nlines").is_err());
        assert!(SecretValue::new(&"x".repeat(MAX_SECRET_BYTES + 1)).is_err());
        // Errors say what is wrong, never what was entered.
        let error = SecretValue::new("bad\u{7}key").unwrap_err();
        assert!(!error.to_string().contains("bad"));
    }

    #[test]
    fn the_memory_store_stores_checks_and_forgets() {
        let store = MemoryStore::default();
        let key = SecretValue::new("sk-test-1").unwrap();
        assert!(!store.contains("anthropic").unwrap());
        store.set("anthropic", &key).unwrap();
        assert!(store.contains("anthropic").unwrap());
        assert_eq!(store.get("anthropic").unwrap(), Some(key));
        store.remove("anthropic").unwrap();
        store.remove("anthropic").unwrap();
        assert_eq!(store.get("anthropic").unwrap(), None);
    }
}
