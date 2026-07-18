use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::PluginInventory;
use super::types::{
    LoadedPlugin, PluginDiagnostic, PluginDiagnosticLevel, PluginId, PluginTrustStatus,
};

const STATE_SCHEMA_VERSION: u32 = 1;
const MAX_REVIEW_HISTORY: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginStateFile {
    schema_version: u32,
    #[serde(default)]
    plugins: BTreeMap<PluginId, PersistedPluginState>,
}

impl Default for PluginStateFile {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            plugins: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPluginState {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    trust: Option<TrustReceipt>,
    #[serde(default)]
    review_history: Vec<TrustReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustReceipt {
    content_hash: String,
    capability_hash: String,
    reviewed_capabilities: PluginInventory,
    reviewed_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct PluginRegistry {
    plugins: BTreeMap<PluginId, LoadedPlugin>,
    names: BTreeMap<String, PluginId>,
    diagnostics: Vec<PluginDiagnostic>,
    state: PluginStateFile,
    state_path: Option<PathBuf>,
    state_error: Option<String>,
}

impl PluginRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn from_discovery(
        plugins: Vec<LoadedPlugin>,
        mut diagnostics: Vec<PluginDiagnostic>,
        state_path: PathBuf,
    ) -> Self {
        let (state, state_error) = match load_state(&state_path) {
            Ok(state) => (state, None),
            Err(error) => {
                diagnostics.push(PluginDiagnostic::error(
                    "state-invalid",
                    format!("Plugin state is fail-closed and will not be overwritten: {error}"),
                    Some(state_path.clone()),
                ));
                (PluginStateFile::default(), Some(error))
            }
        };
        let mut registry = Self {
            plugins: BTreeMap::new(),
            names: BTreeMap::new(),
            diagnostics,
            state,
            state_path: Some(state_path),
            state_error,
        };
        for plugin in plugins {
            registry.register_loaded(plugin);
        }
        registry.apply_state();
        registry
    }

    fn register_loaded(&mut self, plugin: LoadedPlugin) {
        self.names
            .insert(plugin.name().to_string(), plugin.id.clone());
        self.plugins.insert(plugin.id.clone(), plugin);
    }

    fn apply_state(&mut self) {
        for (id, plugin) in &mut self.plugins {
            let persisted = self.state.plugins.get(id);
            plugin.enabled = persisted.is_some_and(|state| state.enabled);
            plugin.trust_status = match persisted.and_then(|state| state.trust.as_ref()) {
                Some(receipt) if receipt.capability_hash != plugin.capability_hash => {
                    PluginTrustStatus::CapabilitiesChanged
                }
                Some(receipt) if receipt.content_hash != plugin.content_hash => {
                    PluginTrustStatus::ContentChanged
                }
                Some(_) => PluginTrustStatus::Trusted,
                None => PluginTrustStatus::NeverReviewed,
            };
            if self.state_error.is_some() {
                plugin.enabled = false;
                plugin.trust_status = PluginTrustStatus::NeverReviewed;
            }
        }
    }

    #[must_use]
    pub fn list(&self) -> Vec<&LoadedPlugin> {
        let mut plugins = self.plugins.values().collect::<Vec<_>>();
        plugins.sort_by(|left, right| {
            left.scope
                .cmp(&right.scope)
                .then_with(|| left.name().cmp(right.name()))
                .then_with(|| left.id.cmp(&right.id))
        });
        plugins
    }

    #[must_use]
    pub fn get(&self, selector: &str) -> Option<&LoadedPlugin> {
        let id = self.resolve_id(selector)?;
        self.plugins.get(id)
    }

    #[must_use]
    pub fn active_plugins(&self) -> Vec<&LoadedPlugin> {
        self.list()
            .into_iter()
            .filter(|plugin| plugin.active())
            .collect()
    }

    /// Compatibility name retained for the MCP adapter. Unlike the old
    /// registry this returns only trusted, active bundles.
    #[must_use]
    pub fn list_enabled(&self) -> Vec<&LoadedPlugin> {
        self.active_plugins()
    }

    #[must_use]
    pub fn enabled_plugins(&self) -> Vec<&LoadedPlugin> {
        self.list()
            .into_iter()
            .filter(|plugin| plugin.enabled)
            .collect()
    }

    #[must_use]
    pub fn is_enabled(&self, selector: &str) -> bool {
        self.get(selector).is_some_and(|plugin| plugin.enabled)
    }

    #[must_use]
    pub fn is_active(&self, selector: &str) -> bool {
        self.get(selector).is_some_and(LoadedPlugin::active)
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[PluginDiagnostic] {
        &self.diagnostics
    }

    #[must_use]
    pub fn validation_is_clean(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == PluginDiagnosticLevel::Error)
            && self.plugins.values().all(|plugin| {
                !plugin
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.level == PluginDiagnosticLevel::Error)
            })
    }

    #[must_use]
    pub fn state_error(&self) -> Option<&str> {
        self.state_error.as_deref()
    }

    #[must_use]
    pub fn state_path(&self) -> Option<&Path> {
        self.state_path.as_deref()
    }

    pub fn trust(&mut self, selector: &str) -> Result<(), String> {
        let plugin = self
            .get(selector)
            .ok_or_else(|| format!("Plugin bundle `{selector}` was not found"))?;
        let id = plugin.id.clone();
        let receipt = TrustReceipt {
            content_hash: plugin.content_hash.clone(),
            capability_hash: plugin.capability_hash.clone(),
            reviewed_capabilities: plugin.inventory.clone(),
            reviewed_at: chrono::Utc::now().to_rfc3339(),
        };
        self.commit_state_change(|state| {
            let entry = state.plugins.entry(id).or_default();
            entry.trust = Some(receipt.clone());
            entry.review_history.push(receipt);
            if entry.review_history.len() > MAX_REVIEW_HISTORY {
                let remove = entry.review_history.len() - MAX_REVIEW_HISTORY;
                entry.review_history.drain(..remove);
            }
        })
    }

    pub fn revoke_trust(&mut self, selector: &str) -> Result<(), String> {
        let id = self
            .resolve_id(selector)
            .cloned()
            .ok_or_else(|| format!("Plugin bundle `{selector}` was not found"))?;
        self.commit_state_change(|state| {
            state.plugins.entry(id).or_default().trust = None;
        })
    }

    pub fn enable(&mut self, selector: &str) -> Result<(), String> {
        let plugin = self
            .get(selector)
            .ok_or_else(|| format!("Plugin bundle `{selector}` was not found"))?;
        if !plugin.trusted() {
            return Err(format!(
                "Plugin bundle `{}` requires capability review before enablement (trust: {})",
                plugin.name(),
                plugin.trust_status.as_str()
            ));
        }
        if !plugin.applicable {
            return Err(format!(
                "Plugin bundle `{}` does not apply to this host",
                plugin.name()
            ));
        }
        let unsupported = plugin.inventory.unsupported_labels();
        if !unsupported.is_empty() {
            return Err(format!(
                "Plugin bundle `{}` declares v0.9.1-inactive capabilities: {}",
                plugin.name(),
                unsupported.join(", ")
            ));
        }
        let id = plugin.id.clone();
        self.commit_state_change(|state| {
            state.plugins.entry(id).or_default().enabled = true;
        })
    }

    pub fn disable(&mut self, selector: &str) -> Result<(), String> {
        let id = self
            .resolve_id(selector)
            .cloned()
            .ok_or_else(|| format!("Plugin bundle `{selector}` was not found"))?;
        self.commit_state_change(|state| {
            state.plugins.entry(id).or_default().enabled = false;
        })
    }

    fn commit_state_change(
        &mut self,
        mutate: impl FnOnce(&mut PluginStateFile),
    ) -> Result<(), String> {
        if let Some(error) = &self.state_error {
            return Err(format!(
                "Plugin state is fail-closed; repair or move the malformed state file before mutating it: {error}"
            ));
        }
        let Some(path) = self.state_path.as_deref() else {
            return Err("Plugin registry has no persistence store".to_string());
        };
        let mut next = self.state.clone();
        mutate(&mut next);
        save_state(path, &next)?;
        self.state = next;
        self.apply_state();
        Ok(())
    }

    fn resolve_id(&self, selector: &str) -> Option<&PluginId> {
        self.plugins
            .keys()
            .find(|id| id.as_str() == selector)
            .or_else(|| self.names.get(selector))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }
}

fn load_state(path: &Path) -> Result<PluginStateFile, String> {
    if !path.exists() {
        return Ok(PluginStateFile::default());
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let state: PluginStateFile = serde_json::from_str(&raw)
        .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
    if state.schema_version != STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported plugin state schema {}; expected {STATE_SCHEMA_VERSION}",
            state.schema_version
        ));
    }
    Ok(state)
}

fn save_state(path: &Path, state: &PluginStateFile) -> Result<(), String> {
    codewhale_config::persistence::atomic_write_json(path, state)
        .map_err(|e| format!("failed to atomically persist {}: {e}", path.display()))
}
