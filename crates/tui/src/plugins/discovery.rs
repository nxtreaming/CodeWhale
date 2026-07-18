use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::manifest::PluginManifest;
use super::registry::PluginRegistry;
use super::types::{
    LoadedPlugin, PluginDiagnostic, PluginId, PluginOrigin, PluginScope, PluginSkillSnapshot,
    PluginTrustStatus,
};

const PLUGIN_MANIFEST: &str = "plugin.toml";
const STATE_FILE: &str = "state.json";

#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub user_plugins_dir: PathBuf,
    pub workspace_plugins_dir: PathBuf,
    pub builtin_plugin_dirs: Vec<PathBuf>,
    pub state_path: PathBuf,
}

impl DiscoveryConfig {
    #[must_use]
    pub fn for_workspace(workspace: &Path) -> Self {
        let user_plugins_dir = default_user_plugins_dir();
        Self {
            state_path: user_plugins_dir.join(STATE_FILE),
            user_plugins_dir,
            workspace_plugins_dir: default_workspace_plugins_dir(workspace),
            // No packaged built-in bundle is activated in v0.9.1. The old
            // source-tree-only rust-toolkit example was misleading because a
            // distributed binary could not resolve its files.
            builtin_plugin_dirs: Vec::new(),
        }
    }
}

#[must_use]
pub fn default_user_plugins_dir() -> PathBuf {
    codewhale_config::codewhale_home()
        .map(|path| path.join("plugins"))
        .unwrap_or_else(|error| {
            // Never fall back to a shared, predictable temporary directory:
            // that would turn a home-resolution failure into ambient plugin
            // discovery. A fresh nonexistent sentinel keeps startup read-only
            // and fail-closed on every supported platform.
            tracing::warn!(
                target: "plugins",
                %error,
                "Codewhale home could not be resolved; user plugin discovery is disabled"
            );
            std::env::temp_dir()
                .join(format!(
                    ".codewhale-home-unavailable-{}",
                    uuid::Uuid::new_v4().simple()
                ))
                .join("plugins")
        })
}

#[must_use]
pub fn default_workspace_plugins_dir(workspace: &Path) -> PathBuf {
    workspace.join(".codewhale").join("plugins")
}

#[must_use]
pub fn default_state_path() -> PathBuf {
    default_user_plugins_dir().join(STATE_FILE)
}

#[must_use]
pub fn discover_all(workspace: &Path) -> PluginRegistry {
    discover_with_config(&DiscoveryConfig::for_workspace(workspace))
}

#[must_use]
pub fn discover_with_config(config: &DiscoveryConfig) -> PluginRegistry {
    let mut diagnostics = Vec::new();
    let mut candidates = Vec::new();

    for root in &config.builtin_plugin_dirs {
        scan_root(
            root,
            PluginScope::Builtin,
            PluginOrigin::Builtin,
            &mut candidates,
            &mut diagnostics,
        );
    }
    scan_root(
        &config.user_plugins_dir,
        PluginScope::User,
        PluginOrigin::CodeWhaleHome,
        &mut candidates,
        &mut diagnostics,
    );
    scan_root(
        &config.workspace_plugins_dir,
        PluginScope::Workspace,
        PluginOrigin::Workspace,
        &mut candidates,
        &mut diagnostics,
    );

    candidates.sort_by(|left, right| {
        left.scope
            .cmp(&right.scope)
            .then_with(|| left.name().cmp(right.name()))
            .then_with(|| left.canonical_root.cmp(&right.canonical_root))
    });

    let mut seen_roots = HashSet::new();
    let mut seen_names = BTreeSet::new();
    let mut plugins = Vec::new();
    for plugin in candidates {
        if !seen_roots.insert(plugin.canonical_root.clone()) {
            diagnostics.push(PluginDiagnostic::warning(
                "duplicate-root",
                format!(
                    "Ignoring duplicate plugin discovery at {}",
                    plugin.canonical_root.display()
                ),
                Some(plugin.canonical_root.clone()),
            ));
            continue;
        }
        if !seen_names.insert(plugin.name().to_string()) {
            diagnostics.push(PluginDiagnostic::warning(
                "name-conflict",
                format!(
                    "Plugin `{}` at {} is shadowed by the higher-precedence bundle with the same name",
                    plugin.name(),
                    plugin.canonical_root.display()
                ),
                Some(plugin.canonical_root.clone()),
            ));
            continue;
        }
        plugins.push(plugin);
    }

    PluginRegistry::from_discovery(plugins, diagnostics, config.state_path.clone())
}

fn scan_root(
    root: &Path,
    scope: PluginScope,
    origin: PluginOrigin,
    plugins: &mut Vec<LoadedPlugin>,
    diagnostics: &mut Vec<PluginDiagnostic>,
) {
    let Ok(metadata) = fs::symlink_metadata(root) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        diagnostics.push(PluginDiagnostic::error(
            "root-symlink",
            format!(
                "Plugin discovery root may not be a symbolic link: {}",
                root.display()
            ),
            Some(root.to_path_buf()),
        ));
        return;
    }
    if !metadata.is_dir() {
        diagnostics.push(PluginDiagnostic::error(
            "root-not-directory",
            format!(
                "Plugin discovery root is not a directory: {}",
                root.display()
            ),
            Some(root.to_path_buf()),
        ));
        return;
    }
    let canonical_discovery_root = match root.canonicalize() {
        Ok(root) => root,
        Err(error) => {
            diagnostics.push(PluginDiagnostic::error(
                "root-canonicalize-failed",
                format!(
                    "Failed to canonicalize plugin root {}: {error}",
                    root.display()
                ),
                Some(root.to_path_buf()),
            ));
            return;
        }
    };

    let mut entries = match fs::read_dir(root) {
        Ok(entries) => match entries.collect::<Result<Vec<_>, _>>() {
            Ok(entries) => entries,
            Err(error) => {
                diagnostics.push(PluginDiagnostic::error(
                    "root-read-failed",
                    format!("Failed to read plugin root {}: {error}", root.display()),
                    Some(root.to_path_buf()),
                ));
                return;
            }
        },
        Err(error) => {
            diagnostics.push(PluginDiagnostic::error(
                "root-read-failed",
                format!("Failed to read plugin root {}: {error}", root.display()),
                Some(root.to_path_buf()),
            ));
            return;
        }
    };
    entries.sort_by_key(fs::DirEntry::file_name);

    for entry in entries {
        let plugin_root = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            diagnostics.push(PluginDiagnostic::error(
                "bundle-symlink",
                format!(
                    "Plugin bundle directory may not be a symbolic link: {}",
                    plugin_root.display()
                ),
                Some(plugin_root),
            ));
            continue;
        }
        if !file_type.is_dir() {
            continue;
        }
        let manifest_path = plugin_root.join(PLUGIN_MANIFEST);
        if !manifest_path.exists() {
            continue;
        }
        match load_plugin(&manifest_path, &canonical_discovery_root, scope, origin) {
            Ok(plugin) => plugins.push(plugin),
            Err(error) => diagnostics.push(PluginDiagnostic::error(
                "manifest-invalid",
                error,
                Some(manifest_path),
            )),
        }
    }
}

fn load_plugin(
    manifest_path: &Path,
    canonical_discovery_root: &Path,
    scope: PluginScope,
    origin: PluginOrigin,
) -> Result<LoadedPlugin, String> {
    let validated = PluginManifest::validate_from_path(manifest_path)?;
    if validated.canonical_root.parent() != Some(canonical_discovery_root) {
        return Err(format!(
            "plugin bundle resolved outside its Codewhale-owned discovery root: {}",
            validated.canonical_root.display()
        ));
    }
    let id = plugin_id(
        scope,
        &validated.manifest.plugin.name,
        &validated.canonical_root,
    );
    let mut diagnostics = validated
        .warnings
        .iter()
        .map(|warning| {
            PluginDiagnostic::warning(
                "manifest-legacy",
                warning.clone(),
                Some(manifest_path.to_path_buf()),
            )
        })
        .collect::<Vec<_>>();

    let mut skill_snapshots = Vec::new();
    for skills_dir in &validated.components.skills {
        let registry = crate::skills::SkillRegistry::discover(skills_dir);
        for warning in registry.warnings() {
            diagnostics.push(PluginDiagnostic::warning(
                "skill-invalid",
                warning.clone(),
                Some(skills_dir.clone()),
            ));
        }
        for skill in registry.list() {
            skill_snapshots.push(PluginSkillSnapshot {
                name: skill.name.clone(),
                description: skill.description.clone(),
                localized_descriptions: skill.localized_descriptions.clone(),
                body: skill.body.clone(),
                path: skill.path.clone(),
            });
        }
    }
    skill_snapshots.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.path.cmp(&right.path))
    });
    let mut seen_skills = BTreeSet::new();
    for skill in &skill_snapshots {
        if !seen_skills.insert(skill.name.clone()) {
            return Err(format!(
                "plugin `{}` declares duplicate skill name `{}`",
                validated.manifest.plugin.name, skill.name
            ));
        }
    }

    // Skill parsing happens after hashing. Revalidate once so a concurrent
    // bundle edit cannot pair a reviewed hash with different in-memory Skill
    // instructions or MCP configuration.
    let refreshed = PluginManifest::validate_from_path(manifest_path)?;
    if refreshed.content_hash != validated.content_hash
        || refreshed.capability_hash != validated.capability_hash
    {
        return Err(format!(
            "plugin `{}` changed during discovery; reload and review the stable bundle",
            validated.manifest.plugin.name
        ));
    }
    let validated = refreshed;

    Ok(LoadedPlugin {
        id,
        manifest: validated.manifest,
        base_path: validated.canonical_root.clone(),
        canonical_root: validated.canonical_root,
        scope,
        origin,
        enabled: false,
        trust_status: PluginTrustStatus::NeverReviewed,
        applicable: validated.applicable,
        inventory: validated.inventory,
        components: validated.components,
        content_hash: validated.content_hash,
        capability_hash: validated.capability_hash,
        skill_snapshots,
        diagnostics,
    })
}

#[cfg(test)]
pub(crate) fn load_plugin_for_test(manifest_path: &Path) -> Result<LoadedPlugin, String> {
    let discovery_root = manifest_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "test plugin manifest needs a bundle and discovery root".to_string())?
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize test discovery root: {error}"))?;
    load_plugin(
        manifest_path,
        &discovery_root,
        PluginScope::User,
        PluginOrigin::CodeWhaleHome,
    )
}

fn plugin_id(scope: PluginScope, name: &str, canonical_root: &Path) -> PluginId {
    let mut hasher = Sha256::new();
    hasher.update(b"codewhale-plugin-id-v1\0");
    hasher.update(scope.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(canonical_root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let suffix = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    PluginId(format!("{}/{suffix}/{name}", scope.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_plugin(root: &Path, dir: &str, name: &str) -> PathBuf {
        let plugin = root.join(dir);
        fs::create_dir_all(&plugin).unwrap();
        fs::write(
            plugin.join("plugin.toml"),
            format!("schema_version = 1\n[plugin]\nname = {name:?}\nversion = \"1.0.0\"\n"),
        )
        .unwrap();
        plugin
    }

    fn config(tmp: &Path) -> DiscoveryConfig {
        DiscoveryConfig {
            user_plugins_dir: tmp.join("user"),
            workspace_plugins_dir: tmp.join("workspace"),
            builtin_plugin_dirs: vec![tmp.join("builtin")],
            state_path: tmp.join("state.json"),
        }
    }

    #[test]
    fn user_and_workspace_bundles_are_disabled_and_untrusted_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path());
        write_plugin(&cfg.user_plugins_dir, "a", "user-plugin");
        write_plugin(&cfg.workspace_plugins_dir, "b", "workspace-plugin");

        let registry = discover_with_config(&cfg);
        assert_eq!(registry.len(), 2);
        assert!(registry.list().iter().all(|plugin| !plugin.enabled));
        assert!(registry.list().iter().all(|plugin| !plugin.trusted()));
        assert!(!cfg.state_path.exists(), "discovery must be read-only");
    }

    #[test]
    fn precedence_is_builtin_then_user_then_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path());
        write_plugin(&cfg.builtin_plugin_dirs[0], "z", "same");
        write_plugin(&cfg.user_plugins_dir, "a", "same");
        write_plugin(&cfg.workspace_plugins_dir, "b", "same");

        let registry = discover_with_config(&cfg);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.get("same").unwrap().scope, PluginScope::Builtin);
        assert_eq!(
            registry
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code == "name-conflict")
                .count(),
            2
        );
    }

    #[test]
    fn discovery_is_sorted_and_plugin_ids_are_deterministic() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path());
        write_plugin(&cfg.user_plugins_dir, "z", "zulu");
        write_plugin(&cfg.user_plugins_dir, "a", "alpha");

        let first = discover_with_config(&cfg);
        let second = discover_with_config(&cfg);
        let names = first
            .list()
            .iter()
            .map(|plugin| plugin.name())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["alpha", "zulu"]);
        assert_eq!(
            first.get("alpha").unwrap().id,
            second.get("alpha").unwrap().id
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_discovery_root_fails_closed() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write_plugin(outside.path(), "a", "outside");
        let cfg = config(tmp.path());
        symlink(outside.path(), &cfg.workspace_plugins_dir).unwrap();

        let registry = discover_with_config(&cfg);
        assert!(registry.is_empty());
        assert!(
            registry
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "root-symlink")
        );
    }

    #[test]
    fn discovery_ignores_ambient_compatibility_roots() {
        let _lock = crate::test_support::lock_test_env();
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let workspace = tmp.path().join("workspace");
        let _home = crate::test_support::EnvVarGuard::set("CODEWHALE_HOME", &home);
        write_plugin(
            &workspace.join(".claude/plugins"),
            "ambient",
            "ambient-plugin",
        );
        write_plugin(
            &workspace.join(".cursor/plugins"),
            "ambient",
            "cursor-plugin",
        );

        let registry = discover_all(&workspace);
        assert!(registry.is_empty());
        assert!(!home.join("plugins/state.json").exists());
    }
}
