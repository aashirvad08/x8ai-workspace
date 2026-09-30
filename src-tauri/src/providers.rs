//! Provider commands: the IPC face of `x8ai-providers` and `x8ai-secrets`
//! (docs/models.md).
//!
//! The webview can list providers with their models and whether a key is saved,
//! save a key (sent once, when the user enters it), remove one, and add or remove
//! model ids. It can never read a key back: no command returns one, and errors
//! never contain one. Keys live in the macOS Keychain; everything else the user
//! configures is in `providers.json`, which holds no secret. Nothing here makes a
//! request to a hosted provider; the only network access is Ollama's detection on
//! the loopback address, when the webview asks for it.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Manager};
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::model::{
    CredentialState, ModelProviderDefinition, ProviderAuth, ProviderList, ProviderStatus,
};
use x8ai_providers::Settings;
use x8ai_providers::ollama::{self, Detection};
use x8ai_secrets::{Keychain, SecretStore, SecretValue};

use crate::workspace::Workspaces;

/// Groups the app's Keychain items; one generic password per provider.
pub const KEYCHAIN_SERVICE: &str = "com.x8ai.workspace.providers";
const KEYCHAIN_LABEL: &str = "x8ai Workspace provider key";

/// Built-in providers, the Keychain, the non-secret settings, and the last local
/// detection. Managed Tauri state.
pub struct Providers {
    definitions: Vec<ModelProviderDefinition>,
    secrets: Box<dyn SecretStore>,
    settings: Mutex<Option<Settings>>,
    /// The last Ollama detection; `None` until the webview asks for one.
    local: Mutex<Option<Detection>>,
}

impl Default for Providers {
    fn default() -> Self {
        Self::with_store(Box::new(Keychain::new(KEYCHAIN_SERVICE, KEYCHAIN_LABEL)))
    }
}

impl Providers {
    fn with_store(secrets: Box<dyn SecretStore>) -> Self {
        Self {
            definitions: x8ai_providers::builtin(),
            secrets,
            settings: Mutex::new(None),
            local: Mutex::new(None),
        }
    }

    /// Loads `providers.json` from the app's data directory. Problems are shown
    /// to the user as warnings.
    pub fn load_settings(&self, data_dir: &Path, workspaces: &Workspaces) {
        let (settings, warning) = Settings::load(data_dir.join("providers.json"));
        if let Some(warning) = warning {
            workspaces.warn(warning);
        }
        *lock(&self.settings) = Some(settings);
    }

    pub fn definition(&self, id: &str) -> Result<&ModelProviderDefinition, CommandError> {
        self.definitions
            .iter()
            .find(|d| d.id.as_str() == id)
            .ok_or_else(|| CommandError::new(ErrorCode::NotFound, format!("no provider {id:?}")))
    }

    pub fn definitions(&self) -> &[ModelProviderDefinition] {
        &self.definitions
    }

    /// Whether `provider` has what it needs to authenticate, without reading
    /// the key.
    pub fn credential_state(&self, provider: &ModelProviderDefinition) -> CredentialState {
        match &provider.auth {
            ProviderAuth::None => CredentialState::NotNeeded,
            ProviderAuth::ApiKey { secret } => match self.secrets.contains(secret.as_str()) {
                Ok(true) => CredentialState::InKeychain,
                // An unreadable Keychain counts as no key; using one reports why.
                Ok(false) | Err(_) => CredentialState::Missing,
            },
        }
    }

    /// The saved key, for an agent session about to start. Only the agent
    /// commands call this, and only to put the key in that agent's environment.
    pub fn credential(
        &self,
        provider: &ModelProviderDefinition,
    ) -> Result<Option<SecretValue>, CommandError> {
        match &provider.auth {
            ProviderAuth::None => Ok(None),
            ProviderAuth::ApiKey { secret } => self
                .secrets
                .get(secret.as_str())
                .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string())),
        }
    }

    /// Every provider's status, with the last local detection: never a new one.
    pub(crate) fn statuses(&self) -> Vec<ProviderStatus> {
        self.definitions.iter().map(|p| self.status(p)).collect()
    }

    /// A provider's status, with the last local detection: never a new one.
    pub(crate) fn status(&self, provider: &ModelProviderDefinition) -> ProviderStatus {
        let custom = lock(&self.settings)
            .as_ref()
            .map(|s| s.get(provider.id.as_str()).models)
            .unwrap_or_default();
        x8ai_providers::status(
            provider,
            self.credential_state(provider),
            lock(&self.local).as_ref(),
            &custom,
        )
    }

    fn with_settings<T>(
        &self,
        change: impl FnOnce(&mut Settings) -> Result<T, x8ai_providers::settings::Error>,
    ) -> Result<T, CommandError> {
        let mut settings = lock(&self.settings);
        let settings = settings.as_mut().ok_or_else(|| {
            CommandError::new(ErrorCode::Internal, "provider settings are unavailable")
        })?;
        change(settings).map_err(|e| {
            let code = match e {
                x8ai_providers::settings::Error::InvalidModel(_) => ErrorCode::InvalidInput,
                x8ai_providers::settings::Error::Io { .. } => ErrorCode::Internal,
            };
            CommandError::new(code, e.to_string())
        })
    }
}

/// Every provider: whether a key is saved, and its models. With `check_local`,
/// local providers (Ollama) are looked for first, on this machine only; without
/// it, the last result is kept.
#[tauri::command]
pub async fn provider_list(
    check_local: bool,
    app: AppHandle,
) -> Result<ProviderList, CommandError> {
    let path = if check_local {
        Some(crate::agents::login_path(&app).await?)
    } else {
        None
    };
    blocking(&app, move |providers| {
        if let Some(path) = path {
            let detection = ollama::detect(
                path.as_deref().map(std::ffi::OsStr::new),
                SocketAddr::from(ollama::ADDRESS),
            );
            *lock(&providers.local) = Some(detection);
        }
        Ok(ProviderList {
            providers: providers
                .definitions
                .iter()
                .map(|p| providers.status(p))
                .collect(),
        })
    })
    .await
}

/// Saves `key` as the provider's API key in the Keychain, replacing any earlier
/// one. The key is not returned, and not kept anywhere else.
#[tauri::command]
pub async fn provider_set_credential(
    provider: String,
    key: String,
    app: AppHandle,
) -> Result<ProviderStatus, CommandError> {
    // Checked and wrapped at once, so only the redacted form travels on.
    let value = SecretValue::new(&key)
        .map_err(|e| CommandError::new(ErrorCode::InvalidInput, e.to_string()));
    drop(key);
    let value = value?;
    blocking(&app, move |providers| {
        let definition = providers.definition(&provider)?;
        let ProviderAuth::ApiKey { secret } = &definition.auth else {
            return Err(CommandError::new(
                ErrorCode::InvalidInput,
                format!("{} does not use an API key", definition.name),
            ));
        };
        providers
            .secrets
            .set(secret.as_str(), &value)
            .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
        Ok(providers.status(definition))
    })
    .await
}

/// Deletes the provider's key from the Keychain. An agent already running keeps
/// the key it was started with; starting it again needs a key.
#[tauri::command]
pub async fn provider_remove_credential(
    provider: String,
    app: AppHandle,
) -> Result<ProviderStatus, CommandError> {
    blocking(&app, move |providers| {
        let definition = providers.definition(&provider)?;
        if let ProviderAuth::ApiKey { secret } = &definition.auth {
            providers
                .secrets
                .remove(secret.as_str())
                .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
        }
        Ok(providers.status(definition))
    })
    .await
}

/// Adds a model id the user knows the provider serves.
#[tauri::command]
pub async fn provider_add_model(
    provider: String,
    model: String,
    app: AppHandle,
) -> Result<ProviderStatus, CommandError> {
    blocking(&app, move |providers| {
        let definition = providers.definition(&provider)?;
        providers.with_settings(|s| s.add_model(definition.id.as_str(), model.trim()))?;
        Ok(providers.status(definition))
    })
    .await
}

/// Removes a model id the user added.
#[tauri::command]
pub async fn provider_remove_model(
    provider: String,
    model: String,
    app: AppHandle,
) -> Result<ProviderStatus, CommandError> {
    blocking(&app, move |providers| {
        let definition = providers.definition(&provider)?;
        providers.with_settings(|s| s.remove_model(definition.id.as_str(), &model))?;
        Ok(providers.status(definition))
    })
    .await
}

/// Runs `work` off the IPC runtime: the Keychain can block, for example while
/// macOS asks the user to unlock it.
async fn blocking<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(&Providers) -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || work(&app.state::<Providers>()))
        .await
        .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use x8ai_secrets::MemoryStore;

    use super::*;

    const KEY: &str = "sk-x8ai-test-0000-invalid";

    #[test]
    fn what_the_webview_receives_never_holds_a_key() {
        let providers = Providers::with_store(Box::new(MemoryStore::default()));
        let temp = std::env::temp_dir().join(format!("x8ai-providers-test-{}", std::process::id()));
        *lock(&providers.settings) = Some(Settings::load(temp.join("providers.json")).0);
        let anthropic = providers.definition("anthropic").unwrap();
        let ProviderAuth::ApiKey { secret } = &anthropic.auth else {
            panic!("Anthropic takes a key")
        };
        assert_eq!(
            providers.credential_state(anthropic),
            CredentialState::Missing
        );
        providers
            .secrets
            .set(secret.as_str(), &SecretValue::new(KEY).unwrap())
            .unwrap();

        let status = providers.status(anthropic);
        assert_eq!(status.credential, CredentialState::InKeychain);
        let list = ProviderList {
            providers: providers
                .definitions()
                .iter()
                .map(|p| providers.status(p))
                .collect(),
        };
        let json = serde_json::to_string(&list).unwrap();
        assert!(!json.contains(KEY), "{json}");
        // Nor does the catalog built from it.
        let catalog = x8ai_catalog::assemble(
            &x8ai_catalog::Metadata::builtin(),
            &x8ai_catalog::Facts {
                agents: &[],
                providers: &list.providers,
                mcp: &[],
                skills: &[],
            },
        );
        let listed = serde_json::to_string(&catalog).unwrap();
        assert!(
            listed.contains(r#""credential":"inKeychain""#) && !listed.contains(KEY),
            "{listed}"
        );
        // Only the agent commands read it, natively.
        assert_eq!(
            providers.credential(anthropic).unwrap().unwrap().expose(),
            KEY
        );
        let ollama = providers.definition("ollama").unwrap();
        assert_eq!(
            providers.credential_state(ollama),
            CredentialState::NotNeeded
        );
        assert!(providers.credential(ollama).unwrap().is_none());
        let _ = std::fs::remove_dir_all(temp);
    }
}
