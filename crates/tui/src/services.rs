//! What `x8ai` keeps beyond spaces, through the app's own files and the
//! Keychain, so both share it: model providers and their API keys
//! (docs/models.md), MCP servers and their secrets (docs/mcp.md), and skills
//! (docs/catalog.md). Each is read when needed and written straight back, as
//! the spaces' stores are, so the app's changes show and the last write wins.
//!
//! Keys and secrets go into the Keychain and are never shown or kept: only
//! starting an agent reads one, for that agent's environment.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use x8ai_agents::adapter;
use x8ai_core::agent::AgentDefinition;
use x8ai_core::mcp::{
    McpAgentSupport, McpEnvSource, McpSecretStatus, McpServer, McpServerInput, McpServerStatus,
};
use x8ai_core::model::{CredentialState, ModelProviderDefinition, ProviderAuth, ProviderStatus};
use x8ai_core::skill::{Skill, SkillInput, SkillStatus};
use x8ai_mcp::{Approvals, Limits, McpRuntime, Registry, prepare, secret_account};
use x8ai_providers::Settings;
use x8ai_providers::ollama::{self, Detection};
use x8ai_secrets::{Cached, Keychain, SecretStore, SecretValue};
use x8ai_skills::SkillRegistry;

const PROVIDERS_FILE: &str = "providers.json";
const MCP_FILE: &str = "mcp-servers.json";
const MCP_APPROVALS_FILE: &str = "mcp-approvals.json";
const SKILLS_FILE: &str = "skills.json";

pub struct Services {
    data_dir: PathBuf,
    providers: Vec<ModelProviderDefinition>,
    provider_secrets: Box<dyn SecretStore>,
    mcp_secrets: Box<dyn SecretStore>,
    /// The last look for local models (Ollama); `None` until the user asks.
    local: Option<Detection>,
    /// The MCP servers of running agent sessions, behind sockets of this
    /// `x8ai` alone (see [`mcp_sockets`]).
    pub mcp: Arc<McpRuntime>,
    /// The folder of those sockets, removed on quitting.
    sockets: PathBuf,
    warnings: Vec<String>,
}

impl Services {
    pub fn new(data_dir: PathBuf, home: &Path) -> Self {
        let sockets = mcp_sockets(home);
        Self {
            data_dir,
            providers: x8ai_providers::builtin(),
            provider_secrets: secret_store(
                x8ai_providers::KEYCHAIN_SERVICE,
                x8ai_providers::KEYCHAIN_LABEL,
            ),
            mcp_secrets: secret_store(
                x8ai_mcp::environment::KEYCHAIN_SERVICE,
                x8ai_mcp::environment::KEYCHAIN_LABEL,
            ),
            local: None,
            mcp: Arc::new(McpRuntime::new(sockets.clone(), Limits::default())),
            sockets,
            warnings: Vec::new(),
        }
    }

    /// Problems reading the files since last asked.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    // Model providers

    pub fn provider_definitions(&self) -> &[ModelProviderDefinition] {
        &self.providers
    }

    pub fn provider(&self, id: &str) -> Option<&ModelProviderDefinition> {
        self.providers.iter().find(|p| p.id.as_str() == id)
    }

    /// Whether `provider` has what it needs to authenticate, without reading
    /// the key.
    pub fn credential_state(&self, provider: &ModelProviderDefinition) -> CredentialState {
        match &provider.auth {
            ProviderAuth::None => CredentialState::NotNeeded,
            ProviderAuth::ApiKey { secret } => {
                match self.provider_secrets.contains(secret.as_str()) {
                    Ok(true) => CredentialState::InKeychain,
                    // An unreadable Keychain counts as no key; using one says why.
                    Ok(false) | Err(_) => CredentialState::Missing,
                }
            }
        }
    }

    /// The saved key, for an agent about to start with this provider.
    pub fn credential(
        &self,
        provider: &ModelProviderDefinition,
    ) -> Result<Option<SecretValue>, String> {
        match &provider.auth {
            ProviderAuth::None => Ok(None),
            ProviderAuth::ApiKey { secret } => self
                .provider_secrets
                .get(secret.as_str())
                .map_err(|e| e.to_string()),
        }
    }

    fn settings(&mut self) -> Settings {
        let (settings, warning) = Settings::load(self.data_dir.join(PROVIDERS_FILE));
        self.warnings.extend(warning);
        settings
    }

    /// Every provider: whether a key is saved, and its models.
    pub fn provider_statuses(&mut self) -> Vec<ProviderStatus> {
        let settings = self.settings();
        self.providers
            .iter()
            .map(|provider| {
                x8ai_providers::status(
                    provider,
                    self.credential_state(provider),
                    self.local.as_ref(),
                    &settings.get(provider.id.as_str()).models,
                )
            })
            .collect()
    }

    /// Saves `key` as the provider's API key in the Keychain.
    pub fn set_key(&self, provider: &str, key: &str) -> Result<(), String> {
        let value = SecretValue::new(key).map_err(|e| e.to_string())?;
        let definition = self.provider(provider).ok_or("No such provider.")?;
        let ProviderAuth::ApiKey { secret } = &definition.auth else {
            return Err(format!("{} does not use an API key.", definition.name));
        };
        self.provider_secrets
            .set(secret.as_str(), &value)
            .map_err(|e| e.to_string())
    }

    pub fn remove_key(&self, provider: &str) -> Result<(), String> {
        let definition = self.provider(provider).ok_or("No such provider.")?;
        match &definition.auth {
            ProviderAuth::ApiKey { secret } => self
                .provider_secrets
                .remove(secret.as_str())
                .map_err(|e| e.to_string()),
            ProviderAuth::None => Ok(()),
        }
    }

    pub fn add_model(&mut self, provider: &str, model: &str) -> Result<(), String> {
        self.settings()
            .add_model(provider, model.trim())
            .map_err(|e| e.to_string())
    }

    pub fn remove_model(&mut self, provider: &str, model: &str) -> Result<(), String> {
        self.settings()
            .remove_model(provider, model)
            .map_err(|e| e.to_string())
    }

    /// Looks for local models (Ollama), on this machine only: its program on
    /// `path`, and its server on the loopback address.
    pub fn detect_local(path: Option<&str>) -> Detection {
        ollama::detect(
            path.map(std::ffi::OsStr::new),
            SocketAddr::from(ollama::ADDRESS),
        )
    }

    pub fn set_local(&mut self, detection: Detection) {
        self.local = Some(detection);
    }

    // MCP servers

    fn registry(&mut self) -> Registry {
        let (registry, warnings) = Registry::load(self.data_dir.join(MCP_FILE));
        self.warnings.extend(warnings);
        registry
    }

    /// The MCP approvals, as the app's store holds them now.
    pub fn mcp_approvals(&mut self) -> Approvals {
        let (approvals, warning) = Approvals::load(self.data_dir.join(MCP_APPROVALS_FILE));
        self.warnings.extend(warning);
        approvals
    }

    pub fn mcp_servers(&mut self) -> Vec<McpServer> {
        self.registry().servers().to_vec()
    }

    pub fn mcp_secrets(&self) -> &dyn SecretStore {
        self.mcp_secrets.as_ref()
    }

    /// Every server: its secrets (saved or not), whether it can run, and which
    /// agents can use it. Nothing is started or contacted.
    pub fn mcp_statuses(
        &mut self,
        path: Option<&str>,
        agents: &[AgentDefinition],
    ) -> Vec<McpServerStatus> {
        self.mcp_servers()
            .iter()
            .map(|server| self.mcp_status(server, path, agents))
            .collect()
    }

    fn mcp_status(
        &self,
        server: &McpServer,
        path: Option<&str>,
        agents: &[AgentDefinition],
    ) -> McpServerStatus {
        let secrets: Vec<McpSecretStatus> = server
            .secret_names()
            .map(|name| McpSecretStatus {
                name: name.to_owned(),
                state: match self
                    .mcp_secrets
                    .contains(&secret_account(server.id.as_str(), name))
                {
                    Ok(true) => CredentialState::InKeychain,
                    Ok(false) | Err(_) => CredentialState::Missing,
                },
            })
            .collect();
        let missing: Vec<&str> = secrets
            .iter()
            .filter(|s| s.state == CredentialState::Missing)
            .map(|s| s.name.as_str())
            .collect();
        let problem = prepare(server, path)
            .err()
            .map(|e| e.to_string())
            .or_else(|| {
                (!missing.is_empty()).then(|| format!("secret not saved: {}", missing.join(", ")))
            });
        McpServerStatus {
            configured: problem.is_none(),
            problem,
            secrets,
            agents: agents
                .iter()
                .map(|agent| {
                    let transports = &agent.capabilities.mcp_transports;
                    let support =
                        adapter::mcp_support(agent.id.as_str(), transports).and_then(|()| {
                            if transports.contains(&server.transport.kind()) {
                                Ok(())
                            } else {
                                Err("it does not support this transport".to_owned())
                            }
                        });
                    McpAgentSupport {
                        agent: agent.id.clone(),
                        supported: support.is_ok(),
                        reason: support.err(),
                    }
                })
                .collect(),
            server: server.clone(),
        }
    }

    pub fn mcp_add(
        &mut self,
        input: &McpServerInput,
        root: Option<&Path>,
    ) -> Result<McpServer, String> {
        self.registry().add(input, root).map_err(|e| e.to_string())
    }

    /// Replaces a server's configuration; secrets of variables it no longer
    /// has are deleted.
    pub fn mcp_update(
        &mut self,
        id: &str,
        input: &McpServerInput,
        root: Option<&Path>,
    ) -> Result<McpServer, String> {
        let (old, new) = self
            .registry()
            .update(id, input, root)
            .map_err(|e| e.to_string())?;
        let kept: Vec<&str> = new.secret_names().collect();
        for name in old.secret_names().filter(|n| !kept.contains(n)) {
            self.mcp_secrets
                .remove(&secret_account(old.id.as_str(), name))
                .map_err(|e| e.to_string())?;
        }
        Ok(new)
    }

    pub fn mcp_set_enabled(&mut self, id: &str, enabled: bool) -> Result<McpServer, String> {
        self.registry()
            .set_enabled(id, enabled)
            .map_err(|e| e.to_string())
    }

    /// Removes a server, its saved secrets, and its approvals.
    pub fn mcp_remove(&mut self, id: &str) -> Result<McpServer, String> {
        let removed = self.registry().remove(id).map_err(|e| e.to_string())?;
        for name in removed.secret_names() {
            self.mcp_secrets
                .remove(&secret_account(removed.id.as_str(), name))
                .map_err(|e| e.to_string())?;
        }
        self.mcp_approvals()
            .forget(removed.id.as_str())
            .map_err(|e| e.to_string())?;
        Ok(removed)
    }

    /// Saves the value of a server's secret variable in the Keychain.
    pub fn mcp_set_secret(&mut self, id: &str, name: &str, value: &str) -> Result<(), String> {
        let value = SecretValue::new(value).map_err(|e| e.to_string())?;
        let server = self
            .mcp_servers()
            .into_iter()
            .find(|s| s.id.as_str() == id)
            .ok_or("No such MCP server.")?;
        if !server
            .env
            .iter()
            .any(|v| v.name == name && v.source == McpEnvSource::Secret)
        {
            return Err(format!("{} has no secret variable {name}.", server.name));
        }
        self.mcp_secrets
            .set(&secret_account(server.id.as_str(), name), &value)
            .map_err(|e| e.to_string())
    }

    // Skills

    fn skill_registry(&mut self) -> SkillRegistry {
        let (registry, warnings) = SkillRegistry::load(self.data_dir.join(SKILLS_FILE));
        self.warnings.extend(warnings);
        registry
    }

    /// Every skill, built-in and the user's.
    pub fn skills(&mut self) -> Vec<Skill> {
        self.skill_registry().skills()
    }

    /// Every skill, with which agents can take it.
    pub fn skill_statuses(&mut self, agents: &[AgentDefinition]) -> Vec<SkillStatus> {
        self.skills()
            .into_iter()
            .map(|skill| SkillStatus {
                skill,
                agents: agents
                    .iter()
                    .map(|agent| {
                        let support = adapter::skills_support(agent.id.as_str());
                        McpAgentSupport {
                            agent: agent.id.clone(),
                            supported: support.is_ok(),
                            reason: support.err(),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    pub fn skill_add(&mut self, input: &SkillInput, root: Option<&Path>) -> Result<Skill, String> {
        self.skill_registry()
            .add(input, root)
            .map_err(|e| e.to_string())
    }

    pub fn skill_update(
        &mut self,
        id: &str,
        input: &SkillInput,
        root: Option<&Path>,
    ) -> Result<Skill, String> {
        self.skill_registry()
            .update(id, input, root)
            .map_err(|e| e.to_string())
    }

    pub fn skill_remove(&mut self, id: &str) -> Result<Skill, String> {
        self.skill_registry().remove(id).map_err(|e| e.to_string())
    }

    /// Stops every MCP server and removes this `x8ai`'s sockets.
    pub fn shut_down(&self) {
        self.mcp.stop_all();
        let _ = std::fs::remove_dir_all(&self.sockets);
    }
}

/// Where this `x8ai`'s MCP sockets go: `~/.x8ai/mcp-<pid>`, beside the app's
/// `~/.x8ai/mcp`, so it never removes the app's sockets, or another `x8ai`'s,
/// as stale; folders of an `x8ai` that is gone are removed. Kept short: a
/// socket's path has at most 104 bytes on macOS. A debug build can be given
/// another folder (`X8AI_TEST_MCP_SOCKETS`), for tests whose home is long.
fn mcp_sockets(home: &Path) -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("X8AI_TEST_MCP_SOCKETS") {
        return PathBuf::from(dir);
    }
    let parent = home.join(".x8ai");
    if let Ok(entries) = std::fs::read_dir(&parent) {
        for entry in entries.flatten() {
            let gone = entry
                .file_name()
                .to_str()
                .and_then(|name| name.strip_prefix("mcp-"))
                .and_then(|pid| pid.parse::<i32>().ok())
                .is_some_and(|pid| {
                    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
                        == Err(nix::errno::Errno::ESRCH)
                });
            if gone {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    parent.join(format!("mcp-{}", std::process::id()))
}

/// The Keychain under `service`, asked at most once per run for each item.
/// A debug build can be pointed at a file instead (`X8AI_TEST_SECRETS`), for
/// tests, which must never touch the user's Keychain; a release build cannot.
fn secret_store(service: &str, label: &str) -> Box<dyn SecretStore> {
    #[cfg(debug_assertions)]
    if let Some(file) = std::env::var_os("X8AI_TEST_SECRETS") {
        return Box::new(test_store::FileStore::new(PathBuf::from(file), service));
    }
    Box::new(Cached::new(Box::new(Keychain::new(service, label))))
}

#[cfg(debug_assertions)]
mod test_store {
    //! A secret store in a plain file, for end-to-end tests of debug builds.

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use x8ai_secrets::{Error, SecretStore, SecretValue};

    pub struct FileStore {
        file: PathBuf,
        service: String,
    }

    impl FileStore {
        pub fn new(file: PathBuf, service: &str) -> Self {
            Self {
                file,
                service: service.to_owned(),
            }
        }

        fn read(&self) -> BTreeMap<String, String> {
            std::fs::read_to_string(&self.file)
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default()
        }

        fn write(&self, all: &BTreeMap<String, String>) -> Result<(), Error> {
            let text =
                serde_json::to_string_pretty(all).map_err(|e| Error::Keychain(e.to_string()))?;
            std::fs::write(&self.file, text).map_err(|e| Error::Keychain(e.to_string()))
        }

        fn key(&self, account: &str) -> String {
            format!("{}/{account}", self.service)
        }
    }

    impl SecretStore for FileStore {
        fn set(&self, account: &str, value: &SecretValue) -> Result<(), Error> {
            let mut all = self.read();
            all.insert(self.key(account), value.expose().to_owned());
            self.write(&all)
        }

        fn get(&self, account: &str) -> Result<Option<SecretValue>, Error> {
            self.read()
                .get(&self.key(account))
                .map(|v| SecretValue::new(v))
                .transpose()
        }

        fn contains(&self, account: &str) -> Result<bool, Error> {
            Ok(self.read().contains_key(&self.key(account)))
        }

        fn remove(&self, account: &str) -> Result<(), Error> {
            let mut all = self.read();
            all.remove(&self.key(account));
            self.write(&all)
        }
    }
}
