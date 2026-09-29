//! What the user configured about providers that is not a secret: model ids they
//! added. Credentials are never here; they are in the Keychain (`x8ai-secrets`).
//!
//! `providers.json` in the app's data directory, mode 0600, replaced atomically.
//! A damaged file is set aside and the settings start empty.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use x8ai_core::model::{MODEL_ID_RULE, is_model_id};

/// Most model ids kept per provider.
const MAX_MODELS: usize = 100;
const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSettings {
    /// Model ids the user added, in the order added.
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    providers: BTreeMap<String, ProviderSettings>,
}

#[derive(Debug)]
pub struct Settings {
    file: PathBuf,
    providers: BTreeMap<String, ProviderSettings>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("model id {0:?} {rule}", rule = MODEL_ID_RULE)]
    InvalidModel(String),
    #[error("{path}: {detail}")]
    Io { path: String, detail: String },
}

impl Settings {
    /// Loads the settings; a missing file is empty. Returns a warning for a
    /// damaged one, which is moved aside.
    pub fn load(file: PathBuf) -> (Self, Option<String>) {
        let read = fs::read(&file);
        let (providers, warning) = match read {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (BTreeMap::new(), None),
            Err(e) => (
                BTreeMap::new(),
                Some(format!("{} could not be read: {e}", file.display())),
            ),
            Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                Ok(parsed) if parsed.version == VERSION => {
                    let mut providers = parsed.providers;
                    for settings in providers.values_mut() {
                        settings.models.retain(|m| is_model_id(m));
                    }
                    (providers, None)
                }
                _ => {
                    let aside = file.with_extension("json.corrupt");
                    let _ = fs::rename(&file, &aside);
                    (
                        BTreeMap::new(),
                        Some(format!(
                            "{} was damaged; starting empty (the old file is at {})",
                            file.display(),
                            aside.display()
                        )),
                    )
                }
            },
        };
        (Self { file, providers }, warning)
    }

    pub fn get(&self, provider: &str) -> ProviderSettings {
        self.providers.get(provider).cloned().unwrap_or_default()
    }

    pub fn add_model(&mut self, provider: &str, model: &str) -> Result<(), Error> {
        if !is_model_id(model) {
            return Err(Error::InvalidModel(model.to_owned()));
        }
        let entry = self.providers.entry(provider.to_owned()).or_default();
        if entry.models.iter().any(|m| m == model) {
            return Ok(());
        }
        entry.models.push(model.to_owned());
        if entry.models.len() > MAX_MODELS {
            entry.models.remove(0);
        }
        self.save()
    }

    pub fn remove_model(&mut self, provider: &str, model: &str) -> Result<(), Error> {
        let Some(entry) = self.providers.get_mut(provider) else {
            return Ok(());
        };
        entry.models.retain(|m| m != model);
        self.save()
    }

    fn save(&self) -> Result<(), Error> {
        let shown = self.file.display().to_string();
        let io = |e: std::io::Error| Error::Io {
            path: shown.clone(),
            detail: e.to_string(),
        };
        let json = serde_json::to_vec_pretty(&File {
            version: VERSION,
            providers: self.providers.clone(),
        })
        .map_err(|e| Error::Io {
            path: shown.clone(),
            detail: e.to_string(),
        })?;
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir).map_err(io)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io)?;
        }
        let temp = self.file.with_extension("json.tmp");
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .map_err(io)?;
        out.write_all(&json)
            .and_then(|()| out.sync_all())
            .map_err(io)?;
        fs::rename(&temp, &self.file).map_err(io)
    }

    pub fn path(&self) -> &Path {
        &self.file
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_persist_and_nothing_else_does() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("data/providers.json");
        let (mut settings, warning) = Settings::load(file.clone());
        assert!(warning.is_none());
        settings
            .add_model("openrouter", "anthropic/claude-sonnet-5")
            .unwrap();
        settings
            .add_model("openrouter", "anthropic/claude-sonnet-5")
            .unwrap();
        settings.add_model("ollama", "qwen3-coder:30b").unwrap();
        assert!(settings.add_model("ollama", "--help").is_err());
        assert!(settings.add_model("ollama", "a b").is_err());

        let (settings, _) = Settings::load(file.clone());
        assert_eq!(
            settings.get("openrouter").models,
            ["anthropic/claude-sonnet-5"]
        );
        assert_eq!(settings.get("ollama").models, ["qwen3-coder:30b"]);
        assert!(settings.get("anthropic").models.is_empty());

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let text = fs::read_to_string(&file).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        // The only things stored: a version and model ids.
        assert_eq!(value["version"], 1);
        assert_eq!(
            value["providers"]["ollama"],
            serde_json::json!({ "models": ["qwen3-coder:30b"] })
        );
    }

    #[test]
    fn removing_a_model_forgets_it() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("providers.json");
        let (mut settings, _) = Settings::load(file.clone());
        settings.add_model("ollama", "a").unwrap();
        settings.add_model("ollama", "b").unwrap();
        settings.remove_model("ollama", "a").unwrap();
        settings.remove_model("nothing", "a").unwrap();
        assert_eq!(Settings::load(file).0.get("ollama").models, ["b"]);
    }

    #[test]
    fn a_damaged_file_is_set_aside_and_bad_entries_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("providers.json");
        fs::write(&file, "{ not json").unwrap();
        let (settings, warning) = Settings::load(file.clone());
        assert!(warning.unwrap().contains("damaged"));
        assert!(settings.get("ollama").models.is_empty());
        assert!(temp.path().join("providers.json.corrupt").exists());

        fs::write(
            &file,
            r#"{"version":1,"providers":{"ollama":{"models":["good:1","--evil","x y"]}}}"#,
        )
        .unwrap();
        assert_eq!(Settings::load(file).0.get("ollama").models, ["good:1"]);
    }
}
