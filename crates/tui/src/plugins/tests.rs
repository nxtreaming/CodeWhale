use std::fs;
use std::path::{Path, PathBuf};

use super::discovery::{DiscoveryConfig, discover_with_config};
use super::types::PluginTrustStatus;

fn config(root: &Path) -> DiscoveryConfig {
    DiscoveryConfig {
        user_plugins_dir: root.join("user"),
        workspace_plugins_dir: root.join("workspace"),
        builtin_plugin_dirs: Vec::new(),
        state_path: root.join("state/plugin-state.json"),
    }
}

fn write_plugin(config: &DiscoveryConfig, extra: &str) -> PathBuf {
    let plugin = config.user_plugins_dir.join("demo");
    fs::create_dir_all(&plugin).unwrap();
    fs::write(
        plugin.join("plugin.toml"),
        format!("schema_version = 1\n[plugin]\nname = \"demo\"\nversion = \"1.0.0\"\n{extra}"),
    )
    .unwrap();
    plugin
}

#[test]
fn trust_and_enablement_are_separate_atomic_state_transitions() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config(tmp.path());
    write_plugin(&config, "");

    let mut registry = discover_with_config(&config);
    assert!(registry.enable("demo").is_err());
    assert!(!config.state_path.exists());

    registry.trust("demo").unwrap();
    assert!(registry.get("demo").unwrap().trusted());
    assert!(!registry.get("demo").unwrap().enabled);
    registry.enable("demo").unwrap();
    assert!(registry.is_active("demo"));

    let raw = fs::read_to_string(&config.state_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed["schema_version"], 1);
    let receipt = parsed["plugins"]
        .as_object()
        .and_then(|plugins| plugins.values().next())
        .and_then(|plugin| plugin.get("trust"))
        .expect("trust receipt");
    assert!(receipt["content_hash"].as_str().is_some());
    assert!(receipt["capability_hash"].as_str().is_some());
    assert_eq!(receipt["reviewed_capabilities"]["skills"], 0);
    assert!(receipt["reviewed_at"].as_str().is_some());
    let history = parsed["plugins"]
        .as_object()
        .and_then(|plugins| plugins.values().next())
        .and_then(|plugin| plugin["review_history"].as_array())
        .expect("review history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["content_hash"], receipt["content_hash"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&config.state_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert_eq!(
        fs::read_dir(config.state_path.parent().unwrap())
            .unwrap()
            .count(),
        1,
        "atomic persistence must not strand temp files"
    );
}

#[test]
fn content_change_invalidates_trust_without_changing_capabilities() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config(tmp.path());
    let plugin = write_plugin(&config, "\n[skills]\npath = \"skills\"\n");
    fs::create_dir_all(plugin.join("skills/demo")).unwrap();
    fs::write(
        plugin.join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: first\n---\nbody\n",
    )
    .unwrap();

    let mut first = discover_with_config(&config);
    first.trust("demo").unwrap();
    first.enable("demo").unwrap();
    assert!(first.is_active("demo"));

    fs::write(
        plugin.join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: changed\n---\nbody\n",
    )
    .unwrap();
    let second = discover_with_config(&config);
    let plugin = second.get("demo").unwrap();
    assert!(plugin.enabled, "enablement is independent from trust");
    assert_eq!(plugin.trust_status, PluginTrustStatus::ContentChanged);
    assert!(!plugin.active());
}

#[test]
fn capability_escalation_invalidates_trust_and_stays_inactive() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config(tmp.path());
    let plugin = write_plugin(&config, "");

    let mut first = discover_with_config(&config);
    first.trust("demo").unwrap();
    first.enable("demo").unwrap();

    fs::create_dir_all(plugin.join("hooks")).unwrap();
    fs::write(
        plugin.join("plugin.toml"),
        "schema_version = 1\n[plugin]\nname = \"demo\"\nversion = \"1.0.0\"\n[hooks]\npath = \"hooks\"\n",
    )
    .unwrap();
    let second = discover_with_config(&config);
    let plugin = second.get("demo").unwrap();
    assert_eq!(plugin.trust_status, PluginTrustStatus::CapabilitiesChanged);
    assert!(plugin.enabled);
    assert!(!plugin.active());
}

#[test]
fn malformed_state_is_fail_closed_and_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config(tmp.path());
    write_plugin(&config, "");
    fs::create_dir_all(config.state_path.parent().unwrap()).unwrap();
    fs::write(&config.state_path, "{ malformed").unwrap();

    let mut registry = discover_with_config(&config);
    assert!(registry.state_error().is_some());
    assert!(!registry.get("demo").unwrap().enabled);
    assert!(!registry.get("demo").unwrap().trusted());
    assert!(registry.trust("demo").is_err());
    assert_eq!(
        fs::read_to_string(&config.state_path).unwrap(),
        "{ malformed"
    );
}

#[test]
fn atomic_write_failure_does_not_mutate_live_enablement() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config(tmp.path());
    write_plugin(&config, "");

    let mut registry = discover_with_config(&config);
    registry.trust("demo").unwrap();
    fs::remove_file(&config.state_path).unwrap();
    fs::create_dir(&config.state_path).unwrap();

    assert!(registry.enable("demo").is_err());
    let plugin = registry.get("demo").unwrap();
    assert!(plugin.trusted());
    assert!(!plugin.enabled);
    assert!(!plugin.active());
}

#[test]
fn revoking_trust_does_not_rewrite_enablement() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config(tmp.path());
    write_plugin(&config, "");

    let mut registry = discover_with_config(&config);
    registry.trust("demo").unwrap();
    registry.enable("demo").unwrap();
    registry.revoke_trust("demo").unwrap();

    let plugin = registry.get("demo").unwrap();
    assert!(plugin.enabled);
    assert!(!plugin.trusted());
    assert!(!plugin.active());
}

#[test]
fn unsupported_components_can_be_reviewed_but_not_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config(tmp.path());
    let plugin = write_plugin(&config, "\n[commands]\npath = \"commands\"\n");
    fs::create_dir_all(plugin.join("commands")).unwrap();

    let mut registry = discover_with_config(&config);
    registry.trust("demo").unwrap();
    let error = registry.enable("demo").unwrap_err();
    assert!(error.contains("inactive capabilities"));
    assert!(!registry.is_active("demo"));
}
