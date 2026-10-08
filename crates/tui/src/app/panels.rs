//! The Models (Ctrl-g m), MCP (Ctrl-g u), Catalog (Ctrl-g k) and Add-ons
//! (Ctrl-g e) panels, and what their forms and pickers do when sent.
//!
//! Each reads the app's own files when it opens and after every change, so
//! what the app changed shows too. Keys and secrets are typed into a form,
//! saved in the Keychain, and never shown again.

use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use x8ai_agents::adapter;
use x8ai_agents::environment::var;
use x8ai_core::addon::AddonGroup;
use x8ai_core::catalog::{CatalogDetails, CatalogItemType, CatalogStatus};
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{
    McpEnvSource, McpEnvVar, McpScope, McpScopeKind, McpServerInput, McpServerTransport,
    McpTransportKind,
};
use x8ai_core::model::{
    CredentialState, LocalAvailability, ModelSelection, ModelSource, ProviderAuth, ProviderKind,
};
use x8ai_core::skill::{SkillInput, SkillScope, SkillSource};
use x8ai_pty::{Environment, Program};

use super::{App, Ask, Msg, Question};
use crate::dialog::{
    Field, Form, FormFor, Outcome, PickFor, PickOption, Picker, Use, join_args, split_args,
};
use crate::listing::{Item, Listing, Tone, matches};
use crate::space::{Focus, Kind, ListKey, Sidebar};
use crate::welcome::tilde;

/// What an agent's next launch gets: chosen in the Agents panel, or from the
/// Models, MCP and Catalog panels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub model: Option<ModelSelection>,
    pub mcp: Vec<IntegrationId>,
    pub skills: Vec<IntegrationId>,
}

/// A box on top: a form or a picker.
pub enum Dialog {
    Form(Form),
    Picker(Picker),
}

impl App {
    // Opening and reading

    /// Ctrl-g m, u, k, e: a list panel, with the keys; hidden when it has them.
    pub(super) fn toggle_list(&mut self, sidebar: Sidebar) {
        let Some(at) = self.view_at() else {
            return;
        };
        if self.views[at].sidebar != sidebar {
            self.views[at].list = Listing::default();
        }
        self.toggle_sidebar(at, sidebar);
        self.refresh_list();
    }

    /// Reads the list panel that shows again.
    pub(super) fn refresh_list(&mut self) {
        let Some(at) = self.view_at() else {
            return;
        };
        let items = match self.views[at].sidebar {
            Sidebar::Models => self.models_items(),
            Sidebar::Mcp => self.mcp_items(),
            Sidebar::Catalog => {
                let filter = self.views[at].list.filter.clone();
                self.catalog_items(&filter)
            }
            Sidebar::Addons => self.addons_items(),
            _ => return,
        };
        self.views[at].list.set(items);
        let warnings = self.services.take_warnings();
        if !warnings.is_empty() {
            self.complain(warnings.join(" "));
        }
    }

    fn path(&self) -> Option<String> {
        var(self.agents.env(), "PATH").map(str::to_owned)
    }

    fn models_items(&mut self) -> Vec<Item<ListKey>> {
        let mut items = Vec::new();
        for provider in self.services.provider_statuses() {
            let (detail, tone) = match (provider.hosting, provider.credential, &provider.local) {
                (_, CredentialState::InKeychain, _) => ("key saved".to_owned(), Tone::Good),
                (_, CredentialState::Missing, _) => {
                    ("no key: s saves one".to_owned(), Tone::Wanting)
                }
                (ProviderKind::Local, _, None) => {
                    ("not looked for: r looks".to_owned(), Tone::Muted)
                }
                (ProviderKind::Local, _, Some(LocalAvailability::Available { .. })) => {
                    ("running here".to_owned(), Tone::Good)
                }
                (ProviderKind::Local, _, Some(LocalAvailability::Installed)) => {
                    ("installed, not running".to_owned(), Tone::Wanting)
                }
                (ProviderKind::Local, _, Some(LocalAvailability::Unavailable)) => {
                    ("not found here".to_owned(), Tone::Muted)
                }
                (_, CredentialState::NotNeeded, _) => ("no key needed".to_owned(), Tone::Good),
            };
            items.push(Item::Row {
                key: ListKey::Provider(provider.id.as_str().to_owned()),
                label: provider.name.clone(),
                detail,
                tone,
                nested: false,
            });
            for model in &provider.models {
                items.push(Item::Row {
                    key: ListKey::Model {
                        provider: provider.id.as_str().to_owned(),
                        model: model.id.clone(),
                        added: model.source == ModelSource::Custom,
                    },
                    label: model.id.clone(),
                    detail: match model.source {
                        ModelSource::BuiltIn => String::new(),
                        ModelSource::Local => "on this Mac".to_owned(),
                        ModelSource::Custom => "added".to_owned(),
                    },
                    tone: Tone::Muted,
                    nested: true,
                });
            }
        }
        items
    }

    fn mcp_items(&mut self) -> Vec<Item<ListKey>> {
        let path = self.path();
        let statuses = self
            .services
            .mcp_statuses(path.as_deref(), self.agents.definitions());
        if statuses.is_empty() {
            return vec![Item::Note("None yet: n adds one.".to_owned())];
        }
        let root = self.current.root.clone();
        statuses
            .into_iter()
            .map(|status| {
                let server = &status.server;
                let transport = match server.transport.kind() {
                    McpTransportKind::Stdio => "stdio",
                    McpTransportKind::StreamableHttp => "http",
                };
                let scope = match &server.scope {
                    McpScope::Global => "every session".to_owned(),
                    McpScope::Workspace { root: r } if root.as_deref() == Some(Path::new(r)) => {
                        "this folder".to_owned()
                    }
                    McpScope::Workspace { root: r } => {
                        format!("in {}", tilde(Path::new(r), self.spaces.home()))
                    }
                    McpScope::Session => "chosen at launch".to_owned(),
                };
                let (state, tone) = if !server.enabled {
                    ("off".to_owned(), Tone::Muted)
                } else if let Some(problem) = &status.problem {
                    (problem.clone(), Tone::Wanting)
                } else {
                    ("ready".to_owned(), Tone::Good)
                };
                Item::Row {
                    key: ListKey::Mcp(server.id.as_str().to_owned()),
                    label: server.name.clone(),
                    detail: format!("{state} · {transport} · {scope}"),
                    tone,
                    nested: false,
                }
            })
            .collect()
    }

    fn catalog_items(&mut self, filter: &str) -> Vec<Item<ListKey>> {
        let root = self
            .current
            .root
            .clone()
            .unwrap_or_else(|| PathBuf::from("/"));
        let definitions = self.agents.definitions().to_vec();
        let providers = self.services.provider_statuses();
        let agents: Vec<_> = definitions
            .iter()
            .map(|d| {
                let spaces = &mut self.spaces;
                let planned = x8ai_agents::plan(d, self.agents.env(), &root);
                let approved = planned.as_ref().is_ok_and(|p| {
                    self.current.root.is_some() && spaces.is_approved_for_any_provider(p)
                });
                let mut status = x8ai_agents::status(
                    d,
                    self.agents.env(),
                    &root,
                    self.services.provider_definitions(),
                    |_| false,
                );
                status.approved = approved;
                status
            })
            .collect();
        let path = self.path();
        let mcp = self.services.mcp_statuses(path.as_deref(), &definitions);
        let skills = self.services.skill_statuses(&definitions);
        let catalog = x8ai_catalog::assemble(
            &x8ai_catalog::Metadata::builtin(),
            &x8ai_catalog::Facts {
                agents: &agents,
                providers: &providers,
                mcp: &mcp,
                skills: &skills,
            },
        );
        let mut items = Vec::new();
        if !filter.is_empty() {
            items.push(Item::Note(format!("/{filter}")));
        }
        for (kind, heading) in [
            (CatalogItemType::Agent, "AGENTS"),
            (CatalogItemType::Model, "MODELS"),
            (CatalogItemType::McpServer, "MCP SERVERS"),
            (CatalogItemType::Skill, "SKILLS"),
        ] {
            let rows: Vec<Item<ListKey>> = catalog
                .items
                .iter()
                .filter(|item| item.item_type == kind)
                .filter(|item| {
                    matches(
                        filter,
                        &format!(
                            "{} {} {}",
                            item.display_name,
                            item.description,
                            item.tags.join(" ")
                        ),
                    )
                })
                .map(|item| {
                    let tone = match item.status {
                        CatalogStatus::Configured | CatalogStatus::Installed => Tone::Good,
                        CatalogStatus::Available => Tone::Wanting,
                        CatalogStatus::Unsupported | CatalogStatus::Unavailable => Tone::Muted,
                    };
                    let detail = item.status_detail.clone().unwrap_or_else(|| {
                        match item.status {
                            CatalogStatus::Installed => "installed",
                            CatalogStatus::Available => "available",
                            CatalogStatus::Configured => "ready",
                            CatalogStatus::Unsupported => "not supported here",
                            CatalogStatus::Unavailable => "unavailable",
                        }
                        .to_owned()
                    });
                    Item::Row {
                        key: ListKey::Catalog(item.id.clone()),
                        label: item.display_name.clone(),
                        detail,
                        tone,
                        nested: false,
                    }
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            if !items.is_empty() {
                items.push(Item::Blank);
            }
            items.push(Item::Heading(heading.to_owned()));
            items.extend(rows);
        }
        if items.is_empty() || (items.len() == 1 && !filter.is_empty()) {
            items.push(Item::Note("Nothing matches.".to_owned()));
        }
        self.catalog = catalog.items;
        items
    }

    /// What is on this Mac, from the `PATH` agents and terminals get.
    pub(super) fn mac(&self) -> x8ai_addons::Mac {
        x8ai_addons::Mac {
            home: self.spaces.home().to_owned(),
            path: self.path(),
        }
    }

    /// Whether add-ons that need trust are on in the space: the workspace with
    /// no folder is the user's home, and counts as trusted, as in the app.
    fn addons_trusted(&mut self, root: Option<&Path>) -> bool {
        root.is_none_or(|root| self.spaces.is_trusted(root))
    }

    fn addons_items(&mut self) -> Vec<Item<ListKey>> {
        let root = self.current.root.clone();
        let space = match self.spaces.space(root.as_deref()) {
            Ok(space) => space,
            Err(error) => return vec![Item::Note(error)],
        };
        let mac = self.mac();
        let trusted = self.addons_trusted(root.as_deref());
        let zsh = x8ai_addons::is_zsh(&x8ai_pty::user_shell());
        let startup = if zsh {
            x8ai_addons::startup_text(self.spaces.home())
        } else {
            String::new()
        };
        let mut items = Vec::new();
        if mac.brew().is_none() {
            items.push(Item::Note(
                "Homebrew is not installed: brew.sh has it. Add-ons install with it.".to_owned(),
            ));
        }
        for (group, heading) in [
            (AddonGroup::Shell, "SHELL"),
            (AddonGroup::Editor, "EDITOR"),
            (AddonGroup::Tools, "TOOLS"),
            (AddonGroup::Look, "LOOK"),
        ] {
            if !items.is_empty() {
                items.push(Item::Blank);
            }
            items.push(Item::Heading(heading.to_owned()));
            for addon in x8ai_addons::ADDONS.iter().filter(|a| a.group == group) {
                let installed = mac.installed(addon);
                let added = space.addons.iter().any(|a| a == addon.id);
                let (mut detail, tone) = if added {
                    if !installed {
                        (
                            "added, not installed: Enter installs it".to_owned(),
                            Tone::Wanting,
                        )
                    } else if addon.needs_trust && !trusted {
                        (
                            "added · on once the folder is trusted".to_owned(),
                            Tone::Wanting,
                        )
                    } else if addon.zshrc.is_some() && !zsh {
                        ("added · needs zsh".to_owned(), Tone::Muted)
                    } else {
                        ("on".to_owned(), Tone::Good)
                    }
                } else if installed {
                    ("installed: Enter adds it".to_owned(), Tone::Plain)
                } else {
                    ("Enter installs it".to_owned(), Tone::Muted)
                };
                if zsh && x8ai_addons::in_your_shell(addon, &startup) {
                    detail.push_str(" · already in your shell");
                }
                items.push(Item::Row {
                    key: ListKey::Addon(addon.id),
                    label: addon.name.to_owned(),
                    detail,
                    tone,
                    nested: false,
                });
            }
        }
        items
    }

    // Keys

    /// Keys for a list panel.
    pub(super) fn list_key(&mut self, key: KeyEvent) {
        let (Some(at), Some(layout)) = (self.view_at(), self.space_layout()) else {
            return;
        };
        let height = layout
            .sidebar_rows()
            .map_or(1, |r| usize::from(r.height).max(1));
        let sidebar = self.views[at].sidebar;
        // The Catalog's filter takes the keys while it is typed.
        if self.views[at].list.filtering {
            let list = &mut self.views[at].list;
            match key.code {
                KeyCode::Esc => {
                    list.filter.clear();
                    list.filtering = false;
                }
                KeyCode::Enter => list.filtering = false,
                KeyCode::Backspace => {
                    list.filter.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    list.filter.push(c);
                }
                _ => {}
            }
            list.selected = 0;
            self.refresh_list();
            return;
        }
        let selected = self.views[at].list.key().cloned();
        let list = &mut self.views[at].list;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => list.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => list.move_by(1),
            KeyCode::Home | KeyCode::Char('g') => list.selected = 0,
            KeyCode::End | KeyCode::Char('G') => list.move_by(isize::MAX),
            KeyCode::PageUp => list.move_by(-(height as isize)),
            KeyCode::PageDown => list.move_by(height as isize),
            KeyCode::Esc | KeyCode::Tab => self.views[at].focus = Focus::Panes,
            _ => match sidebar {
                Sidebar::Models => self.models_key(key, selected),
                Sidebar::Mcp => self.mcp_key(key, selected),
                Sidebar::Catalog => self.catalog_key(key, selected),
                Sidebar::Addons => self.addons_key(key, selected),
                _ => {}
            },
        }
        if let Some(view) = self.views.get_mut(at) {
            view.list.keep_in_view(height);
        }
    }

    fn models_key(&mut self, key: KeyEvent, selected: Option<ListKey>) {
        let provider = match &selected {
            Some(ListKey::Provider(p) | ListKey::Model { provider: p, .. }) => Some(p.clone()),
            _ => None,
        };
        match (key.code, selected) {
            (
                KeyCode::Enter,
                Some(ListKey::Model {
                    provider, model, ..
                }),
            ) => {
                let Ok(provider) = IntegrationId::new(provider) else {
                    return;
                };
                self.use_for_launch(Use::Model(ModelSelection { provider, model }));
            }
            (KeyCode::Char('s'), _) => {
                let Some(provider) = provider.and_then(|p| self.services.provider(&p).cloned())
                else {
                    return;
                };
                if matches!(provider.auth, ProviderAuth::None) {
                    self.say(format!("{} needs no key.", provider.name));
                    return;
                }
                let form = Form::new(
                    format!("{}'s API key", provider.name),
                    FormFor::ProviderKey(provider.id.as_str().to_owned()),
                    vec![Field::secret(
                        "API key",
                        "Saved in your Keychain, and never shown again.",
                    )],
                )
                .with_note("Agents you start with one of its models get it, in their own environment only.");
                self.dialog = Some(Dialog::Form(form));
            }
            (KeyCode::Char('a'), _) => {
                let Some(provider) = provider.and_then(|p| self.services.provider(&p).cloned())
                else {
                    return;
                };
                let form = Form::new(
                    format!("Add a model id to {}", provider.name),
                    FormFor::AddModel(provider.id.as_str().to_owned()),
                    vec![Field::text(
                        "Model id",
                        "",
                        "An id the provider serves, as its documentation names it.",
                    )],
                );
                self.dialog = Some(Dialog::Form(form));
            }
            (
                KeyCode::Char('d'),
                Some(ListKey::Model {
                    provider,
                    model,
                    added: true,
                }),
            ) => {
                match self.services.remove_model(&provider, &model) {
                    Ok(()) => self.say(format!("{model} removed.")),
                    Err(error) => self.complain(error),
                }
                self.refresh_list();
            }
            (KeyCode::Char('d'), Some(ListKey::Provider(provider))) => {
                let Some(definition) = self.services.provider(&provider).cloned() else {
                    return;
                };
                if self.services.credential_state(&definition) != CredentialState::InKeychain {
                    self.say(format!("{} has no saved key.", definition.name));
                    return;
                }
                self.question = Some(Question {
                    title: format!("Remove {}'s key?", definition.name),
                    lines: vec![
                        "It is deleted from your Keychain. Agents already running keep it; starting one with this provider needs a key again.".to_owned(),
                    ],
                    yes: "remove",
                    ask: Ask::RemoveKey(provider),
                });
            }
            (KeyCode::Char('r'), _) => {
                let path = self.path();
                let tx = self.tx.clone();
                self.say("Looking for local models (Ollama), on this Mac only…");
                std::thread::spawn(move || {
                    let detection = crate::services::Services::detect_local(path.as_deref());
                    let _ = tx.send(Msg::Local(detection));
                });
            }
            _ => {}
        }
    }

    fn mcp_key(&mut self, key: KeyEvent, selected: Option<ListKey>) {
        let server = match &selected {
            Some(ListKey::Mcp(id)) => self
                .services
                .mcp_servers()
                .into_iter()
                .find(|s| s.id.as_str() == id),
            _ => None,
        };
        match key.code {
            KeyCode::Char('n') => self.dialog = Some(Dialog::Form(mcp_form(None))),
            KeyCode::Enter | KeyCode::Char('e') => {
                if let Some(server) = server {
                    self.dialog = Some(Dialog::Form(mcp_form(Some(&server))));
                }
            }
            KeyCode::Char(' ') => {
                if let Some(server) = server {
                    match self
                        .services
                        .mcp_set_enabled(server.id.as_str(), !server.enabled)
                    {
                        Ok(s) => self.say(format!(
                            "{} is {}.",
                            s.name,
                            if s.enabled { "on" } else { "off" }
                        )),
                        Err(error) => self.complain(error),
                    }
                    self.refresh_list();
                }
            }
            KeyCode::Char('s') => {
                let Some(server) = server else {
                    return;
                };
                let names: Vec<String> = server.secret_names().map(str::to_owned).collect();
                match names.as_slice() {
                    [] => self.say(format!("{} has no secret variables.", server.name)),
                    [one] => {
                        self.dialog = Some(Dialog::Form(secret_form(
                            &server.name,
                            server.id.as_str(),
                            one,
                        )))
                    }
                    many => {
                        self.dialog = Some(Dialog::Picker(Picker {
                            title: format!("Which secret of {}?", server.name),
                            options: many
                                .iter()
                                .map(|n| PickOption {
                                    value: n.clone(),
                                    label: n.clone(),
                                    detail: String::new(),
                                    chosen: false,
                                })
                                .collect(),
                            multi: false,
                            selected: 0,
                            purpose: PickFor::Secret(server.id.as_str().to_owned()),
                        }));
                    }
                }
            }
            KeyCode::Char('d') => {
                if let Some(server) = server {
                    self.question = Some(Question {
                        title: format!("Remove the MCP server {}?", server.name),
                        lines: vec![
                            "Its saved secrets and its approvals go with it. Sessions that had it run without it.".to_owned(),
                        ],
                        yes: "remove",
                        ask: Ask::RemoveMcp(server.id.as_str().to_owned()),
                    });
                }
            }
            KeyCode::Char('l') => {
                if let Some(server) = server {
                    if server.scope != McpScope::Session {
                        self.say(format!(
                            "{} is attached to every session it applies to already.",
                            server.name
                        ));
                    } else {
                        self.use_for_launch(Use::Mcp(server.id.clone()));
                    }
                }
            }
            _ => {}
        }
    }

    fn catalog_key(&mut self, key: KeyEvent, selected: Option<ListKey>) {
        let item = match &selected {
            Some(ListKey::Catalog(id)) => self.catalog.iter().find(|i| i.id == *id).cloned(),
            _ => None,
        };
        match key.code {
            KeyCode::Char('/') => {
                if let Some(at) = self.view_at() {
                    self.views[at].list.filtering = true;
                }
            }
            KeyCode::Char('n') => {
                self.dialog = Some(Dialog::Form(skill_form(None, self.current.root.is_some())))
            }
            KeyCode::Enter => {
                let Some(item) = item else {
                    return;
                };
                match &item.details {
                    CatalogDetails::Agent { agent, .. } => self.show_agent(agent.as_str()),
                    CatalogDetails::Model {
                        provider, model, ..
                    } => self.use_for_launch(Use::Model(ModelSelection {
                        provider: provider.clone(),
                        model: model.clone(),
                    })),
                    CatalogDetails::Provider { .. } => self.toggle_list(Sidebar::Models),
                    CatalogDetails::McpServer { server, scope, .. } => {
                        if *scope == McpScope::Session {
                            self.use_for_launch(Use::Mcp(server.clone()));
                        } else {
                            self.toggle_list(Sidebar::Mcp);
                        }
                    }
                    CatalogDetails::Skill { skill, scope, .. } => {
                        if *scope == SkillScope::Session {
                            self.use_for_launch(Use::Skill(skill.clone()));
                        } else {
                            self.say(format!(
                                "{} is attached to every session it applies to already.",
                                item.display_name
                            ));
                        }
                    }
                }
            }
            KeyCode::Char('e') | KeyCode::Char('d') => {
                let Some(CatalogDetails::Skill {
                    skill,
                    source: SkillSource::User,
                    ..
                }) = item.as_ref().map(|i| &i.details)
                else {
                    self.say("Only your own skills can be changed: n writes one.");
                    return;
                };
                let Some(found) = self.services.skills().into_iter().find(|s| s.id == *skill)
                else {
                    return;
                };
                if key.code == KeyCode::Char('e') {
                    self.dialog = Some(Dialog::Form(skill_form(
                        Some(&found),
                        self.current.root.is_some(),
                    )));
                } else {
                    self.question = Some(Question {
                        title: format!("Remove the skill {}?", found.name),
                        lines: vec![
                            "Sessions that have it stop running until started anew.".to_owned(),
                        ],
                        yes: "remove",
                        ask: Ask::RemoveSkill(found.id.as_str().to_owned()),
                    });
                }
            }
            _ => {}
        }
    }

    fn addons_key(&mut self, key: KeyEvent, selected: Option<ListKey>) {
        let addon = match selected {
            Some(ListKey::Addon(id)) => x8ai_addons::find(id),
            _ => None,
        };
        let root = self.current.root.clone();
        match key.code {
            KeyCode::Enter => {
                let Some(addon) = addon else {
                    return;
                };
                let set = x8ai_addons::with_requirements([addon.id]);
                let ids: Vec<&'static str> = set.iter().map(|a| a.id).collect();
                let commands = match self.mac().install_commands(&set) {
                    Ok(commands) => commands,
                    Err(missing) => {
                        self.complain(missing.to_string());
                        return;
                    }
                };
                if commands.is_empty() {
                    match self.spaces.add_addons(root.as_deref(), &ids) {
                        Ok(_) => self.say(format!(
                            "{} is added to {}. New terminals here have it.",
                            addon.name, self.current.name
                        )),
                        Err(error) => self.complain(error),
                    }
                    self.refresh_list();
                    return;
                }
                let mut lines = vec![
                    "It is installed first, in a tab of its own where you can watch it:".to_owned(),
                ];
                lines.extend(
                    commands
                        .iter()
                        .map(|c| format!("    {}", x8ai_addons::shown(c))),
                );
                lines.push(format!(
                    "Homebrew downloads it from its own sources. Then it is added to {}.",
                    self.current.name
                ));
                self.question = Some(Question {
                    title: format!("Add {} to {}?", addon.name, self.current.name),
                    lines,
                    yes: "install",
                    ask: Ask::Install {
                        addons: ids,
                        root,
                        name: addon.name.to_owned(),
                        script: x8ai_addons::install_script(&commands),
                    },
                });
            }
            KeyCode::Char('d') => {
                let Some(addon) = addon else {
                    return;
                };
                match self.spaces.remove_addon(root.as_deref(), addon.id) {
                    Ok(_) => self.say(format!(
                        "{} is removed from {}. New terminals here start without it.",
                        addon.name, self.current.name
                    )),
                    Err(error) => self.complain(error),
                }
                self.refresh_list();
            }
            KeyCode::Char('s') => self.ask_share(),
            KeyCode::Char('r') => self.refresh_list(),
            _ => {}
        }
    }

    // Using things for a launch

    /// Gives `what` to an agent's next launch: the one agent that can use it,
    /// or the one the user picks.
    pub(super) fn use_for_launch(&mut self, what: Use) {
        let mut candidates = Vec::new();
        for definition in self.agents.definitions() {
            let fits = match &what {
                Use::Model(model) => self
                    .services
                    .provider(model.provider.as_str())
                    .is_some_and(|p| adapter::support(definition.id.as_str(), p).is_ok()),
                Use::Mcp(id) => {
                    let transports = &definition.capabilities.mcp_transports;
                    adapter::mcp_support(definition.id.as_str(), transports).is_ok()
                        && self
                            .services
                            .mcp_servers()
                            .iter()
                            .find(|s| s.id == *id)
                            .is_some_and(|s| transports.contains(&s.transport.kind()))
                }
                Use::Skill(_) => adapter::skills_support(definition.id.as_str()).is_ok(),
            };
            if fits {
                candidates.push((definition.id.as_str().to_owned(), definition.name.clone()));
            }
        }
        match candidates.as_slice() {
            [] => self.complain("No agent here can use that."),
            [(agent, _)] => {
                let agent = agent.clone();
                self.apply_use(&agent, what);
            }
            many => {
                self.dialog = Some(Dialog::Picker(Picker {
                    title: "For which agent's next launch?".to_owned(),
                    options: many
                        .iter()
                        .map(|(id, name)| PickOption {
                            value: id.clone(),
                            label: name.clone(),
                            detail: String::new(),
                            chosen: false,
                        })
                        .collect(),
                    multi: false,
                    selected: 0,
                    purpose: PickFor::Agent(what),
                }));
            }
        }
    }

    fn apply_use(&mut self, agent: &str, what: Use) {
        let name = self
            .agents
            .definition(agent)
            .map_or_else(|| agent.to_owned(), |d| d.name.clone());
        let draft = self.drafts.entry(agent.to_owned()).or_default();
        let said = match what {
            Use::Model(model) => {
                let said = format!(
                    "{name}'s next session uses {} · {}.",
                    model.provider, model.model
                );
                draft.model = Some(model);
                said
            }
            Use::Mcp(id) => {
                if !draft.mcp.contains(&id) {
                    draft.mcp.push(id.clone());
                }
                format!("{name}'s next session gets the MCP server {id}.")
            }
            Use::Skill(id) => {
                if !draft.skills.contains(&id) {
                    draft.skills.push(id.clone());
                }
                format!("{name}'s next session gets the skill {id}.")
            }
        };
        self.say(format!("{said} Ctrl-g a, then Enter on {name}, starts it."));
        self.refresh_panel();
    }

    fn show_agent(&mut self, agent: &str) {
        let Some(at) = self.view_at() else {
            return;
        };
        if self.current.root.is_none() {
            self.say("Agents work in a folder: open one with /cd first.");
            return;
        }
        self.views[at].sidebar = crate::space::Sidebar::Agents;
        self.views[at].focus = Focus::Sidebar;
        self.refresh_panel();
        let panel = &mut self.views[at].panel;
        if let Some(index) = panel.rows.iter().position(
            |r| matches!(r, crate::space::PanelRow::Agent(a) if a.definition.id.as_str() == agent),
        ) {
            panel.selected = index;
        }
    }

    // Sharing add-ons

    fn ask_share(&mut self) {
        let root = self.current.root.clone();
        let Ok(space) = self.spaces.space(root.as_deref()) else {
            return;
        };
        if space.addons.is_empty() {
            self.say("This space has no add-ons to share yet.");
            return;
        }
        let others: Vec<PickOption> = self
            .spaces
            .all_spaces()
            .into_iter()
            .filter(|s| s.id != space.id)
            .map(|s| PickOption {
                label: s.root.as_deref().map_or_else(
                    || "Home".to_owned(),
                    |r| {
                        r.file_name().map_or_else(
                            || r.display().to_string(),
                            |n| n.to_string_lossy().into_owned(),
                        )
                    },
                ),
                detail: format!(
                    "{} · {}",
                    s.id,
                    s.root
                        .as_deref()
                        .map_or_else(|| "no folder".to_owned(), |r| tilde(r, self.spaces.home()))
                ),
                value: s.id,
                chosen: false,
            })
            .collect();
        if others.is_empty() {
            self.say("There is no other space to share with yet.");
            return;
        }
        self.dialog = Some(Dialog::Picker(Picker {
            title: format!("Give {}'s add-ons to which space?", self.current.name),
            options: others,
            multi: false,
            selected: 0,
            purpose: PickFor::Share,
        }));
    }

    /// `/share <space>` on the Welcome: by id, or a name only one space has.
    pub(super) fn share_with(&mut self, name: &str) {
        let root = self.current.root.clone();
        let Ok(space) = self.spaces.space(root.as_deref()) else {
            return;
        };
        let wanted = name.trim().to_lowercase();
        let matching: Vec<_> = self
            .spaces
            .all_spaces()
            .into_iter()
            .filter(|s| s.id != space.id)
            .filter(|s| {
                s.id == wanted
                    || s.root.as_deref().is_some_and(|r| {
                        r.file_name().is_some_and(|n| {
                            n.to_string_lossy().to_lowercase().starts_with(&wanted)
                        })
                    })
                    || (s.root.is_none() && "home".starts_with(&wanted))
            })
            .collect();
        match matching.as_slice() {
            [] => self.complain(format!(
                "No other space is “{name}”. Spaces get an id when first opened."
            )),
            [one] => {
                let id = one.id.clone();
                self.share_to(&id);
            }
            many => self.complain(format!(
                "Several spaces match “{name}”: {}. Use the id.",
                many.iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    fn share_to(&mut self, to: &str) {
        let root = self.current.root.clone();
        let Ok(space) = self.spaces.space(root.as_deref()) else {
            return;
        };
        if space.addons.is_empty() {
            self.say("This space has no add-ons to share yet.");
            return;
        }
        let ids: Vec<&str> = space.addons.iter().map(String::as_str).collect();
        match self.spaces.add_addons_to(to, &ids) {
            Ok(_) => self.say(format!(
                "{to} now has {}'s add-ons: {}.",
                self.current.name,
                ids.join(", ")
            )),
            Err(error) => self.complain(error),
        }
    }

    /// y to "Add … to …?": the install runs in a tab of its own.
    pub(super) fn install(
        &mut self,
        addons: Vec<&'static str>,
        root: Option<PathBuf>,
        name: String,
        script: String,
    ) {
        let Some(at) = self.view_at() else {
            return;
        };
        let program = Program::Exec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            cwd: Some(self.spaces.home().to_owned()),
            env: Environment::Exactly(self.agents.env().to_vec()),
        };
        let size = self.new_pane_size(at, None);
        match self.start(&program, Kind::Install { addons, root, name }, size) {
            Ok(slot) => self.views[at].add_tab(slot),
            Err(error) => self.complain(format!("Could not start the install: {error}")),
        }
    }

    /// An install tab ended well: its add-ons are added to their space, if
    /// they are found now.
    pub(super) fn installed(&mut self, addons: &[&'static str], root: Option<&Path>, name: &str) {
        let mac = self.mac();
        let all = addons
            .iter()
            .filter_map(|id| x8ai_addons::find(id))
            .all(|a| mac.installed(a));
        if !all {
            self.complain(format!("{name} is still not found after its install."));
            return;
        }
        match self.spaces.add_addons(root, addons) {
            Ok(_) => self.say(format!(
                "{name} is installed and added. New terminals here have it."
            )),
            Err(error) => self.complain(error),
        }
        self.refresh_list();
    }

    // Dialogs

    pub(super) fn dialog_key(&mut self, key: KeyEvent) {
        let outcome = match &mut self.dialog {
            Some(Dialog::Form(form)) => form.key(key),
            Some(Dialog::Picker(picker)) => picker.key(key),
            None => return,
        };
        match outcome {
            Outcome::Open => {}
            Outcome::Cancelled => self.dialog = None,
            Outcome::Sent => match self.dialog.take() {
                Some(Dialog::Form(form)) => {
                    if let Err(error) = self.form_sent(&form) {
                        let mut form = form;
                        form.error = Some(error);
                        self.dialog = Some(Dialog::Form(form));
                    } else {
                        self.refresh_list();
                    }
                }
                Some(Dialog::Picker(picker)) => self.pick_sent(&picker),
                None => {}
            },
        }
    }

    /// What a sent form does; an error keeps it open, saying why.
    fn form_sent(&mut self, form: &Form) -> Result<(), String> {
        let root = self.current.root.clone();
        match &form.purpose {
            FormFor::ProviderKey(provider) => {
                let key = form.value("API key").trim();
                if key.is_empty() {
                    return Err("Type or paste the key.".to_owned());
                }
                self.services.set_key(provider, key)?;
                let name = self
                    .services
                    .provider(provider)
                    .map_or_else(|| provider.clone(), |p| p.name.clone());
                self.say(format!("{name}'s key is saved in your Keychain."));
            }
            FormFor::AddModel(provider) => {
                let model = form.value("Model id").trim();
                if model.is_empty() {
                    return Err("Type the model id.".to_owned());
                }
                self.services.add_model(provider, model)?;
                self.say(format!("{model} added."));
            }
            FormFor::McpAdd | FormFor::McpEdit(_) => {
                let editing = match &form.purpose {
                    FormFor::McpEdit(id) => self
                        .services
                        .mcp_servers()
                        .into_iter()
                        .find(|s| s.id.as_str() == id),
                    _ => None,
                };
                let input = mcp_input(form, editing.as_ref().is_none_or(|s| s.enabled))?;
                if input.scope == McpScopeKind::Workspace && root.is_none() {
                    return Err("“This folder” needs a folder open: /cd to one first.".to_owned());
                }
                let server = match &form.purpose {
                    FormFor::McpEdit(id) => {
                        self.services.mcp_update(id, &input, root.as_deref())?
                    }
                    _ => self.services.mcp_add(&input, root.as_deref())?,
                };
                let missing: Vec<&str> = server.secret_names().collect();
                self.say(if missing.is_empty() {
                    format!("{} is saved.", server.name)
                } else {
                    format!(
                        "{} is saved. s saves its secret(s): {}.",
                        server.name,
                        missing.join(", ")
                    )
                });
            }
            FormFor::McpSecret { server, variable } => {
                let value = form.value("Value");
                if value.is_empty() {
                    return Err("Type or paste the value.".to_owned());
                }
                self.services.mcp_set_secret(server, variable, value)?;
                self.say(format!("{variable} is saved in your Keychain."));
            }
            FormFor::SkillAdd | FormFor::SkillEdit(_) => {
                let input = SkillInput {
                    name: form.value("Name").trim().to_owned(),
                    description: String::new(),
                    instructions: form.value("Instructions").trim().to_owned(),
                    allowed_tools: Vec::new(),
                    scope: [
                        McpScopeKind::Session,
                        McpScopeKind::Workspace,
                        McpScopeKind::Global,
                    ][form.chosen("Attached to")],
                };
                if input.scope == McpScopeKind::Workspace && root.is_none() {
                    return Err("“This folder” needs a folder open: /cd to one first.".to_owned());
                }
                let skill = match &form.purpose {
                    FormFor::SkillEdit(id) => {
                        self.services.skill_update(id, &input, root.as_deref())?
                    }
                    _ => self.services.skill_add(&input, root.as_deref())?,
                };
                self.say(format!("The skill {} is saved.", skill.name));
            }
        }
        Ok(())
    }

    fn pick_sent(&mut self, picker: &Picker) {
        let values = picker.values();
        match &picker.purpose {
            PickFor::Agent(what) => {
                if let Some(agent) = values.first() {
                    self.apply_use(agent, what.clone());
                }
            }
            PickFor::Secret(server) => {
                let Some(variable) = values.first() else {
                    return;
                };
                let name = self
                    .services
                    .mcp_servers()
                    .into_iter()
                    .find(|s| s.id.as_str() == server)
                    .map_or_else(|| server.clone(), |s| s.name);
                self.dialog = Some(Dialog::Form(secret_form(&name, server, variable)));
            }
            PickFor::Share => {
                if let Some(to) = values.first() {
                    let to = to.clone();
                    self.share_to(&to);
                }
            }
            PickFor::Model(agent) => {
                let draft = self.drafts.entry(agent.clone()).or_default();
                draft.model = values.first().and_then(|v| {
                    let (provider, model) = v.split_once('\t')?;
                    Some(ModelSelection {
                        provider: IntegrationId::new(provider.to_owned()).ok()?,
                        model: model.to_owned(),
                    })
                });
                self.refresh_panel();
            }
            PickFor::Mcp(agent) => {
                let ids = values
                    .iter()
                    .filter_map(|v| IntegrationId::new(v.clone()).ok())
                    .collect();
                self.drafts.entry(agent.clone()).or_default().mcp = ids;
                self.refresh_panel();
            }
            PickFor::Skills(agent) => {
                let ids = values
                    .iter()
                    .filter_map(|v| IntegrationId::new(v.clone()).ok())
                    .collect();
                self.drafts.entry(agent.clone()).or_default().skills = ids;
                self.refresh_panel();
            }
        }
    }
}

/// The form that adds an MCP server, or changes `server`.
fn mcp_form(server: Option<&x8ai_core::mcp::McpServer>) -> Form {
    let (transport, target, args) = match server.map(|s| &s.transport) {
        Some(McpServerTransport::Stdio { command, args }) => (0, command.clone(), join_args(args)),
        Some(McpServerTransport::StreamableHttp { url }) => (1, url.clone(), String::new()),
        None => (0, String::new(), String::new()),
    };
    let variables = server.map_or_else(String::new, |s| {
        s.env
            .iter()
            .map(|v| match v.source {
                McpEnvSource::Secret => v.name.clone(),
                McpEnvSource::Inherit => format!("{}=shell", v.name),
            })
            .collect::<Vec<_>>()
            .join(", ")
    });
    let scope = server.map_or(2, |s| match s.scope {
        McpScope::Global => 0,
        McpScope::Workspace { .. } => 1,
        McpScope::Session => 2,
    });
    let (title, purpose) = match server {
        Some(s) => (
            format!("Change {}", s.name),
            FormFor::McpEdit(s.id.as_str().to_owned()),
        ),
        None => ("A new MCP server".to_owned(), FormFor::McpAdd),
    };
    Form::new(
        title,
        purpose,
        vec![
            Field::text(
                "Name",
                server.map_or("", |s| s.name.as_str()),
                "How agents and you see it.",
            ),
            Field::choice(
                "Transport",
                vec!["stdio", "http"],
                transport,
                "←→: a program on this Mac, or a server at a URL.",
            ),
            Field::text(
                "Command or URL",
                &target,
                "stdio: a program (npx, /usr/local/bin/x); http: its https URL.",
            ),
            Field::text(
                "Arguments",
                &args,
                "stdio only: as you would type them; quotes keep spaces.",
            ),
            Field::text(
                "Variables",
                &variables,
                "NAME for a secret saved in your Keychain; NAME=shell to take it from your shell.",
            ),
            Field::choice(
                "Attached to",
                vec!["every session", "this folder", "chosen at launch"],
                scope,
                "←→: which agent sessions get it.",
            ),
        ],
    )
    .with_note(
        "It runs only for agent sessions, in trusted folders, once you allow it there as it runs.",
    )
}

/// A server as the form describes it.
fn mcp_input(form: &Form, enabled: bool) -> Result<McpServerInput, String> {
    let target = form.value("Command or URL").trim().to_owned();
    let transport = if form.chosen("Transport") == 0 {
        McpServerTransport::Stdio {
            command: target,
            args: split_args(form.value("Arguments"))?,
        }
    } else {
        McpServerTransport::StreamableHttp { url: target }
    };
    let mut env = Vec::new();
    for part in form
        .value("Variables")
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let (name, source) = match part.split_once('=') {
            Some((name, "shell")) => (name.trim(), McpEnvSource::Inherit),
            Some(_) => {
                return Err(format!(
                    "“{part}”: a value is never typed here; write NAME (a secret) or NAME=shell."
                ));
            }
            None => (part, McpEnvSource::Secret),
        };
        env.push(McpEnvVar {
            name: name.to_owned(),
            source,
        });
    }
    let input = McpServerInput {
        name: form.value("Name").trim().to_owned(),
        description: String::new(),
        transport,
        env,
        enabled,
        scope: [
            McpScopeKind::Global,
            McpScopeKind::Workspace,
            McpScopeKind::Session,
        ][form.chosen("Attached to")],
    };
    input.validate().map_err(|e| e.to_string())?;
    Ok(input)
}

fn secret_form(server: &str, id: &str, variable: &str) -> Form {
    Form::new(
        format!("{server}: {variable}"),
        FormFor::McpSecret {
            server: id.to_owned(),
            variable: variable.to_owned(),
        },
        vec![Field::secret(
            "Value",
            "Saved in your Keychain, and never shown again.",
        )],
    )
    .with_note("Only this server gets it, when it starts for an agent session.")
}

/// The form that writes a skill, or changes `skill`.
fn skill_form(skill: Option<&x8ai_core::skill::Skill>, folder: bool) -> Form {
    let scope = skill.map_or(0, |s| match s.scope {
        SkillScope::Session => 0,
        SkillScope::Workspace { .. } => 1,
        SkillScope::Global => 2,
    });
    let (title, purpose) = match skill {
        Some(s) => (
            format!("Change the skill {}", s.name),
            FormFor::SkillEdit(s.id.as_str().to_owned()),
        ),
        None => ("A new skill".to_owned(), FormFor::SkillAdd),
    };
    let mut form = Form::new(
        title,
        purpose,
        vec![
            Field::text(
                "Name",
                skill.map_or("", |s| s.name.as_str()),
                "Write a failing test first, Explain before changing…",
            ),
            Field::text(
                "Instructions",
                skill.map_or("", |s| s.instructions.as_str()),
                "What the agent is told. Plain text, never a secret.",
            ),
            Field::choice(
                "Attached to",
                vec!["chosen at launch", "this folder", "every session"],
                scope,
                "←→: which agent sessions get it.",
            ),
        ],
    )
    .with_note("A skill is instructions for an agent session. It runs nothing.");
    if !folder {
        form.note = Some("A skill is instructions for an agent session. It runs nothing. (“This folder” needs a folder open.)".to_owned());
    }
    form
}
