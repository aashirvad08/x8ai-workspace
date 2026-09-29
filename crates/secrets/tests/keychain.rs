//! The real login Keychain. Not run by default, because it touches the user's
//! Keychain (with a throwaway service name, removed at the end):
//!
//! ```sh
//! cargo test -p x8ai-secrets --test keychain -- --ignored
//! ```

#![cfg(target_os = "macos")]

use x8ai_secrets::{Keychain, SecretStore, SecretValue};

#[test]
#[ignore = "touches the login Keychain; run by hand"]
fn stores_finds_reads_and_removes_a_credential() {
    let service = format!("com.x8ai.workspace.test.{}", std::process::id());
    let keychain = Keychain::new(&service, "x8ai Workspace test credential");
    let value = SecretValue::new("sk-test-clearly-invalid-000000").unwrap();

    assert!(!keychain.contains("anthropic").unwrap());
    assert_eq!(keychain.get("anthropic").unwrap(), None);
    keychain.set("anthropic", &value).unwrap();
    assert!(keychain.contains("anthropic").unwrap());
    // Setting again replaces rather than duplicating.
    let replacement = SecretValue::new("sk-test-clearly-invalid-111111").unwrap();
    keychain.set("anthropic", &replacement).unwrap();
    assert_eq!(keychain.get("anthropic").unwrap(), Some(replacement));

    // Another instance (as after an app restart) finds it too.
    let again = Keychain::new(&service, "x8ai Workspace test credential");
    assert!(again.contains("anthropic").unwrap());

    keychain.remove("anthropic").unwrap();
    assert!(!keychain.contains("anthropic").unwrap());
    keychain.remove("anthropic").unwrap();
}
