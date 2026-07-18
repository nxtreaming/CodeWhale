use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::manifest::{PluginInventory, PluginManifest, ResolvedPluginComponents};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginScope {
    Builtin,
    User,
    Workspace,
}

impl PluginScope {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::User => "user",
            Self::Workspace => "workspace",
        }
    }
}

impl fmt::Display for PluginScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginOrigin {
    Builtin,
    CodeWhaleHome,
    Workspace,
}

impl PluginOrigin {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "codewhale-builtin",
            Self::CodeWhaleHome => "codewhale-home",
            Self::Workspace => "workspace-codewhale",
        }
    }
}

impl fmt::Display for PluginOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginId(pub String);

impl PluginId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginDiagnosticLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDiagnostic {
    pub level: PluginDiagnosticLevel,
    pub code: &'static str,
    pub message: String,
    pub path: Option<PathBuf>,
}

impl PluginDiagnostic {
    #[must_use]
    pub fn warning(code: &'static str, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            level: PluginDiagnosticLevel::Warning,
            code,
            message: message.into(),
            path,
        }
    }

    #[must_use]
    pub fn error(code: &'static str, message: impl Into<String>, path: Option<PathBuf>) -> Self {
        Self {
            level: PluginDiagnosticLevel::Error,
            code,
            message: message.into(),
            path,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginTrustStatus {
    Trusted,
    NeverReviewed,
    ContentChanged,
    CapabilitiesChanged,
}

impl PluginTrustStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::NeverReviewed => "not-reviewed",
            Self::ContentChanged => "content-changed",
            Self::CapabilitiesChanged => "capabilities-changed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PluginSkillSnapshot {
    pub name: String,
    pub description: String,
    pub localized_descriptions: HashMap<String, String>,
    pub body: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub id: PluginId,
    pub manifest: PluginManifest,
    pub base_path: PathBuf,
    pub canonical_root: PathBuf,
    pub scope: PluginScope,
    pub origin: PluginOrigin,
    pub enabled: bool,
    pub trust_status: PluginTrustStatus,
    pub applicable: bool,
    pub inventory: PluginInventory,
    pub components: ResolvedPluginComponents,
    pub content_hash: String,
    pub capability_hash: String,
    pub skill_snapshots: Vec<PluginSkillSnapshot>,
    pub diagnostics: Vec<PluginDiagnostic>,
}

impl LoadedPlugin {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.manifest.plugin.name
    }

    #[must_use]
    pub fn trusted(&self) -> bool {
        self.trust_status == PluginTrustStatus::Trusted
    }

    #[must_use]
    pub fn active(&self) -> bool {
        self.enabled
            && self.trusted()
            && self.applicable
            && !self.inventory.has_unsupported_capabilities()
            && !self
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.level == PluginDiagnosticLevel::Error)
    }

    #[must_use]
    pub fn state_label(&self) -> &'static str {
        if self.active() {
            "active"
        } else if !self.enabled {
            "disabled"
        } else if !self.trusted() {
            "enabled-untrusted"
        } else if !self.applicable {
            "inapplicable"
        } else if self.inventory.has_unsupported_capabilities() {
            "unsupported"
        } else {
            "inactive"
        }
    }
}
