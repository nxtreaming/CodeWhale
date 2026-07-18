//! Codewhale bundle lifecycle and legacy executable plugin-tool inventory.
//!
//! `/plugin` owns declarative bundles (`plugin.toml`). Script tools under
//! `[tools].plugin_dir` remain supported, but are labeled as legacy executable
//! tools and never share bundle trust state.

use std::fmt::Write as _;
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::CommandResult;
use crate::commands::traits::{
    Command, CommandGroup, CommandInfo, FunctionCommand, RegisterCommand,
};
use crate::localization::{MessageId, tr};
use crate::plugins::types::{LoadedPlugin, PluginDiagnosticLevel};
use crate::tools::plugin::{PluginMetadata, scan_plugin_dir};
use crate::tools::spec::ApprovalRequirement;
use crate::tui::app::App;

pub struct PluginsCommands;

impl CommandGroup for PluginsCommands {
    fn commands(&self) -> &'static [Box<dyn Command>] {
        cached_command_list!(vec![Box::new(FunctionCommand::new(
            PluginsCmd::info(),
            PluginsCmd::execute,
        ))])
    }
}

pub(in crate::commands) const PLUGINS_INFO: CommandInfo = CommandInfo {
    name: "plugin",
    aliases: &["plugins"],
    usage: "/plugin [list|show|validate|trust|enable|disable|revoke|reload|tools]",
    description_id: MessageId::CmdPluginDescription,
};

pub(in crate::commands) struct PluginsCmd;

impl RegisterCommand for PluginsCmd {
    fn info() -> &'static CommandInfo {
        &PLUGINS_INFO
    }

    fn execute(app: &mut App, arg: Option<&str>) -> CommandResult {
        plugins(app, arg)
    }
}

fn plugins(app: &mut App, arg: Option<&str>) -> CommandResult {
    let words = arg
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>();
    match words.as_slice() {
        [] | ["list"] => list_bundles_and_legacy_tools(app),
        ["help"] => CommandResult::message(tr(app.ui_locale, MessageId::CmdPluginBundleUsage)),
        ["show", selector] => show_bundle(app, selector),
        ["validate"] => validate_bundles(app, None),
        ["validate", selector] => validate_bundles(app, Some(selector)),
        ["trust", selector] => review_bundle(app, selector),
        ["trust", selector, token] => mutate_bundle(app, selector, Mutation::Trust(token)),
        ["enable", selector] => mutate_bundle(app, selector, Mutation::Enable),
        ["disable", selector] => mutate_bundle(app, selector, Mutation::Disable),
        ["revoke", selector] => mutate_bundle(app, selector, Mutation::Revoke),
        ["reload"] => match crate::plugins::reload_registry(&app.workspace) {
            Ok(count) => CommandResult::message(
                tr(app.ui_locale, MessageId::CmdPluginBundleReloaded)
                    .replace("{count}", &count.to_string())
                    .replace("{workspace}", &app.workspace.display().to_string()),
            ),
            Err(error) => action_error(app, &error),
        },
        ["tools"] => legacy_tools(app, None),
        ["tools", name] => legacy_tools(app, Some(name)),
        [selector] => {
            if crate::plugins::try_with_registry(|registry| registry.get(selector).is_some())
                .unwrap_or(false)
            {
                show_bundle(app, selector)
            } else {
                // Preserve `/plugin <script-tool>` compatibility while making
                // its distinct execution model explicit in the output.
                legacy_tools(app, Some(selector))
            }
        }
        _ => CommandResult::error(tr(app.ui_locale, MessageId::CmdPluginBundleUsage)),
    }
}

fn list_bundles_and_legacy_tools(app: &App) -> CommandResult {
    let mut output = crate::plugins::try_with_registry(|registry| {
        let plugins = registry.list();
        let mut output = if plugins.is_empty() {
            tr(app.ui_locale, MessageId::CmdPluginBundleNoneFound).into_owned()
        } else {
            let mut output = tr(app.ui_locale, MessageId::CmdPluginBundleListHeader)
                .replace("{count}", &plugins.len().to_string());
            output.push('\n');
            for plugin in plugins {
                let _ = writeln!(
                    output,
                    "• {} — {}\n  {} · {} · {}\n  {}",
                    plugin.name(),
                    plugin.state_label(),
                    plugin.scope,
                    plugin.trust_status.as_str(),
                    plugin.inventory.summary(),
                    plugin.id
                );
            }
            output
        };
        append_diagnostics(app, &mut output, registry.diagnostics());
        output
    })
    .unwrap_or_else(|| tr(app.ui_locale, MessageId::CmdPluginBundleNoneFound).into_owned());

    if let Some((dir, tools)) = scan_legacy_tools(app) {
        output.push('\n');
        output.push_str(
            &tr(app.ui_locale, MessageId::CmdPluginLegacyListHeader)
                .replace("{count}", &tools.len().to_string())
                .replace("{dir}", &dir.display().to_string()),
        );
        output.push('\n');
        for (path, metadata) in tools {
            let _ = writeln!(
                output,
                "• {} — {}\n  {}",
                metadata.name,
                metadata.description,
                path.display()
            );
        }
    }

    CommandResult::message(output)
}

fn show_bundle(app: &App, selector: &str) -> CommandResult {
    let Some(plugin) =
        crate::plugins::try_with_registry(|registry| registry.get(selector).cloned()).flatten()
    else {
        return CommandResult::error(
            tr(app.ui_locale, MessageId::CmdPluginBundleNotFound).replace("{name}", selector),
        );
    };
    CommandResult::message(render_bundle_detail(app, &plugin, true))
}

fn review_bundle(app: &App, selector: &str) -> CommandResult {
    let Some(plugin) =
        crate::plugins::try_with_registry(|registry| registry.get(selector).cloned()).flatten()
    else {
        return CommandResult::error(
            tr(app.ui_locale, MessageId::CmdPluginBundleNotFound).replace("{name}", selector),
        );
    };
    let mut output = render_bundle_detail(app, &plugin, true);
    let _ = writeln!(
        output,
        "\n/plugin trust {} {}",
        plugin.name(),
        review_token(&plugin)
    );
    CommandResult::message(output)
}

fn validate_bundles(app: &App, selector: Option<&str>) -> CommandResult {
    let Some((plugins, diagnostics, clean)) = crate::plugins::try_with_registry(|registry| {
        let plugins: Vec<LoadedPlugin> = match selector {
            Some(selector) => registry.get(selector).cloned().into_iter().collect(),
            None => registry.list().into_iter().cloned().collect(),
        };
        (
            plugins,
            registry.diagnostics().to_vec(),
            registry.validation_is_clean(),
        )
    }) else {
        return CommandResult::error(tr(app.ui_locale, MessageId::CmdPluginBundleNoneFound));
    };
    if selector.is_some() && plugins.is_empty() {
        return CommandResult::error(
            tr(app.ui_locale, MessageId::CmdPluginBundleNotFound)
                .replace("{name}", selector.unwrap_or_default()),
        );
    }

    let mut output = String::new();
    for plugin in &plugins {
        let _ = writeln!(
            output,
            "{} — {} — {}",
            plugin.name(),
            if plugin
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.level == PluginDiagnosticLevel::Error)
            {
                "invalid"
            } else {
                "valid"
            },
            plugin.inventory.summary()
        );
        append_diagnostics(app, &mut output, &plugin.diagnostics);
    }
    append_diagnostics(app, &mut output, &diagnostics);
    if output.is_empty() {
        output.push_str(if clean { "valid" } else { "invalid" });
    }
    CommandResult::message(output)
}

#[derive(Clone, Copy)]
enum Mutation<'a> {
    Trust(&'a str),
    Enable,
    Disable,
    Revoke,
}

fn mutate_bundle(app: &App, selector: &str, mutation: Mutation<'_>) -> CommandResult {
    if matches!(mutation, Mutation::Enable) {
        let needs_review = crate::plugins::try_with_registry(|registry| {
            registry
                .get(selector)
                .is_some_and(|plugin| !plugin.trusted())
        })
        .unwrap_or(false);
        if needs_review {
            // Enabling is the natural entry point. Open the exact capability
            // review instead of leaving the user at an opaque denial.
            return review_bundle(app, selector);
        }
    }
    if let Mutation::Trust(token) = mutation {
        let Some(expected) =
            crate::plugins::try_with_registry(|registry| registry.get(selector).map(review_token))
                .flatten()
        else {
            return CommandResult::error(
                tr(app.ui_locale, MessageId::CmdPluginBundleNotFound).replace("{name}", selector),
            );
        };
        if token != expected {
            return action_error(
                app,
                "Review token does not match this bundle content and capability set; run `/plugin trust <name>` again",
            );
        }
    }

    let result = crate::plugins::with_registry(|registry| match mutation {
        Mutation::Trust(_) => registry.trust(selector).map(|()| "trusted"),
        Mutation::Enable => registry.enable(selector).map(|()| "enabled"),
        Mutation::Disable => registry.disable(selector).map(|()| "disabled"),
        Mutation::Revoke => registry.revoke_trust(selector).map(|()| "trust-revoked"),
    });
    match result {
        Some(Ok(action)) => CommandResult::message(
            tr(app.ui_locale, MessageId::CmdPluginBundleMutationSuccess)
                .replace("{name}", selector)
                .replace("{action}", action),
        ),
        Some(Err(error)) => action_error(app, &error),
        None => action_error(app, "Plugin registry is not initialized"),
    }
}

fn render_bundle_detail(app: &App, plugin: &LoadedPlugin, include_hashes: bool) -> String {
    let unsupported = plugin.inventory.unsupported_labels();
    let unsupported = if unsupported.is_empty() {
        "none".to_string()
    } else {
        unsupported.join(", ")
    };
    let (content_hash, capability_hash) = if include_hashes {
        (
            plugin.content_hash.as_str(),
            plugin.capability_hash.as_str(),
        )
    } else {
        ("hidden", "hidden")
    };
    let mut output = tr(app.ui_locale, MessageId::CmdPluginBundleDetail)
        .replace("{name}", plugin.name())
        .replace("{id}", plugin.id.as_str())
        .replace("{version}", &plugin.manifest.plugin.version)
        .replace("{origin}", plugin.origin.as_str())
        .replace("{scope}", plugin.scope.as_str())
        .replace("{state}", plugin.state_label())
        .replace("{trust}", plugin.trust_status.as_str())
        .replace("{inventory}", &plugin.inventory.summary())
        .replace("{permissions}", &render_permissions(plugin))
        .replace("{mcp}", &render_mcp_inventory(plugin))
        .replace("{unsupported}", &unsupported)
        .replace("{content_hash}", content_hash)
        .replace("{capability_hash}", capability_hash)
        .replace("{path}", &plugin.canonical_root.display().to_string());
    append_diagnostics(app, &mut output, &plugin.diagnostics);
    output
}

fn render_permissions(plugin: &LoadedPlugin) -> String {
    let filesystem = if plugin.inventory.filesystem_roots.is_empty() {
        "none".to_string()
    } else {
        plugin.inventory.filesystem_roots.join(", ")
    };
    let network = if plugin.inventory.network_hosts.is_empty() {
        "none".to_string()
    } else {
        plugin.inventory.network_hosts.join(", ")
    };
    format!(
        "filesystem_roots=[{filesystem}] network_hosts=[{network}] lifecycle_mutation={}",
        plugin.inventory.lifecycle_mutation
    )
}

fn render_mcp_inventory(plugin: &LoadedPlugin) -> String {
    let Some(servers) = plugin.manifest.mcp_servers.as_ref() else {
        return "none".to_string();
    };
    let mut servers = servers.iter().collect::<Vec<_>>();
    servers.sort_by_key(|(name, _)| *name);
    servers
        .into_iter()
        .map(|(name, server)| {
            let enabled = if server.is_enabled() {
                "configured-on"
            } else {
                "configured-off"
            };
            if let Some(command) = server.command.as_deref() {
                format!(
                    "{name}: stdio command={command} args={} {enabled}",
                    server.args.len()
                )
            } else if let Some(url) = server.url.as_deref() {
                let endpoint = reqwest::Url::parse(url)
                    .ok()
                    .map(|url| url.origin().ascii_serialization())
                    .unwrap_or_else(|| "invalid-url".to_string());
                format!("{name}: remote endpoint={endpoint} {enabled}")
            } else {
                format!("{name}: invalid")
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn review_token(plugin: &LoadedPlugin) -> String {
    format!(
        "{}.{}",
        &plugin.content_hash[..12],
        &plugin.capability_hash[..12]
    )
}

fn append_diagnostics(
    app: &App,
    output: &mut String,
    diagnostics: &[crate::plugins::types::PluginDiagnostic],
) {
    if diagnostics.is_empty() {
        return;
    }
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(
        &tr(app.ui_locale, MessageId::CmdPluginBundleDiagnosticsHeader)
            .replace("{count}", &diagnostics.len().to_string()),
    );
    output.push('\n');
    for diagnostic in diagnostics {
        let level = match diagnostic.level {
            PluginDiagnosticLevel::Warning => "warning",
            PluginDiagnosticLevel::Error => "error",
        };
        let path = diagnostic
            .path
            .as_deref()
            .map(|path| format!(" ({})", path.display()))
            .unwrap_or_default();
        let _ = writeln!(
            output,
            "• {level} [{}]: {}{path}",
            diagnostic.code, diagnostic.message
        );
    }
}

fn action_error(app: &App, error: &str) -> CommandResult {
    CommandResult::error(
        tr(app.ui_locale, MessageId::CmdPluginActionFailed).replace("{error}", error),
    )
}

fn legacy_tools(app: &App, name: Option<&str>) -> CommandResult {
    let Some(plugin_dir) = plugin_dir_for(app) else {
        return action_error(
            app,
            "Could not resolve the legacy executable plugin-tool directory",
        );
    };
    if !plugin_dir.exists() {
        return CommandResult::message(
            tr(app.ui_locale, MessageId::CmdPluginNoneFound)
                .replace("{dir}", &plugin_dir.display().to_string()),
        );
    }
    let discovered = scan_plugin_dir(&plugin_dir);
    match name {
        Some(name) => show_legacy_tool_detail(app, name, &discovered),
        None => list_legacy_tools(app, &plugin_dir, &discovered),
    }
}

fn list_legacy_tools(
    app: &App,
    plugin_dir: &Path,
    discovered: &[(PathBuf, PluginMetadata)],
) -> CommandResult {
    if discovered.is_empty() {
        return CommandResult::message(
            tr(app.ui_locale, MessageId::CmdPluginNoneFound)
                .replace("{dir}", &plugin_dir.display().to_string()),
        );
    }
    let mut output = tr(app.ui_locale, MessageId::CmdPluginLegacyListHeader)
        .replace("{count}", &discovered.len().to_string())
        .replace("{dir}", &plugin_dir.display().to_string());
    output.push('\n');
    for (path, metadata) in discovered {
        let _ = writeln!(
            output,
            "• {} — {}\n  {}",
            metadata.name,
            metadata.description,
            path.display()
        );
    }
    CommandResult::message(output)
}

fn show_legacy_tool_detail(
    app: &App,
    name: &str,
    discovered: &[(PathBuf, PluginMetadata)],
) -> CommandResult {
    let Some((path, metadata)) = discovered
        .iter()
        .find(|(_, metadata)| metadata.name == name)
    else {
        return CommandResult::error(
            tr(app.ui_locale, MessageId::CmdPluginNotFound).replace("{name}", name),
        );
    };
    let schema = serde_json::to_string_pretty(&metadata.input_schema).unwrap_or_default();
    let mut output = format!("{}\n{:=<40}\n", metadata.name, "");
    let _ = writeln!(
        output,
        "{}",
        tr(app.ui_locale, MessageId::CmdPluginDetailDescription)
            .replace("{description}", &metadata.description)
    );
    let _ = writeln!(
        output,
        "{}",
        tr(app.ui_locale, MessageId::CmdPluginDetailSchema).replace("{schema}", &schema)
    );
    let _ = writeln!(
        output,
        "{}",
        tr(app.ui_locale, MessageId::CmdPluginDetailApproval)
            .replace("{approval}", approval_label(metadata.approval))
    );
    let _ = writeln!(
        output,
        "{}",
        tr(app.ui_locale, MessageId::CmdPluginDetailPath)
            .replace("{path}", &path.display().to_string())
    );
    CommandResult::message(output)
}

fn scan_legacy_tools(app: &App) -> Option<(PathBuf, Vec<(PathBuf, PluginMetadata)>)> {
    let dir = plugin_dir_for(app)?;
    dir.exists().then(|| {
        let tools = scan_plugin_dir(&dir);
        (dir, tools)
    })
}

fn approval_label(approval: ApprovalRequirement) -> &'static str {
    match approval {
        ApprovalRequirement::Auto => "auto",
        ApprovalRequirement::Suggest => "suggest",
        ApprovalRequirement::Required => "required",
    }
}

fn plugin_dir_for(app: &App) -> Option<PathBuf> {
    app.legacy_plugin_tools_dir
        .clone()
        .or_else(default_codewhale_tools_dir)
}

fn default_codewhale_tools_dir() -> Option<PathBuf> {
    codewhale_config::codewhale_home()
        .ok()
        .map(|home| home.join("tools"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::localization::Locale;
    use crate::tui::app::{App, TuiOptions};
    use tempfile::TempDir;

    fn create_test_app(root: &Path) -> (App, TempDir) {
        let temp = TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        let tools_dir = root.join("tools");
        fs::create_dir_all(&tools_dir).unwrap();
        fs::write(
            &config_path,
            format!(
                "[tools]\nplugin_dir = {}\n",
                toml::Value::String(tools_dir.to_string_lossy().to_string())
            ),
        )
        .unwrap();
        let options = TuiOptions {
            model: "deepseek-v4-pro".to_string(),
            workspace: root.to_path_buf(),
            config_path: Some(config_path),
            config_profile: None,
            allow_shell: false,
            use_alt_screen: true,
            use_mouse_capture: false,
            use_bracketed_paste: true,
            max_subagents: 1,
            skills_dir: temp.path().join("skills"),
            memory_path: temp.path().join("memory.md"),
            notes_path: temp.path().join("notes.txt"),
            mcp_config_path: temp.path().join("mcp.json"),
            use_memory: false,
            start_in_agent_mode: false,
            skip_onboarding: true,
            yolo: false,
            resume_session_id: None,
            initial_input: None,
        };
        let config = Config {
            tools: Some(crate::config::ToolsConfig {
                plugin_dir: Some(tools_dir.to_string_lossy().into_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut app = App::new(options, &config);
        app.ui_locale = Locale::En;
        (app, temp)
    }

    fn write_bundle(root: &Path) {
        let bundle = root.join(".codewhale/plugins/demo");
        fs::create_dir_all(bundle.join("skills/hello")).unwrap();
        fs::write(
            bundle.join("plugin.toml"),
            "schema_version = 1\n[plugin]\nname = \"demo\"\nversion = \"1.0.0\"\n[skills]\npath = \"skills\"\n",
        )
        .unwrap();
        fs::write(
            bundle.join("skills/hello/SKILL.md"),
            "---\nname: hello\ndescription: hello\n---\nbody\n",
        )
        .unwrap();
    }

    #[test]
    fn list_show_validate_are_read_only_and_label_legacy_tools() {
        let _lock = crate::test_support::lock_test_env();
        let root = TempDir::new().unwrap();
        let codewhale_home = root.path().join("home");
        let _home = crate::test_support::EnvVarGuard::set("CODEWHALE_HOME", &codewhale_home);
        write_bundle(root.path());
        let (mut app, _temp) = create_test_app(root.path());
        fs::write(
            root.path().join("tools/greet.sh"),
            "# name: greet\n# description: hello\n",
        )
        .unwrap();
        // The app already resolved the legacy tools path during startup.
        // Read-only plugin commands must not reopen a credential-bearing
        // config file merely to inventory those tools.
        fs::write(
            app.config_path.as_ref().unwrap(),
            "api_key = [\"must-not-be-re-read\"\n",
        )
        .unwrap();
        crate::plugins::init_registry(root.path());
        let state_path = codewhale_home.join("plugins/state.json");

        for arg in [Some("list"), Some("show demo"), Some("validate")] {
            let result = plugins(&mut app, arg);
            assert!(!result.is_error, "{:?}", result.message);
            assert!(!state_path.exists(), "read-only command wrote plugin state");
        }
        let list = plugins(&mut app, Some("list")).message.unwrap();
        assert!(list.contains("Plugin bundles (1)"));
        assert!(list.contains("disabled"));
        assert!(list.contains("Legacy executable plugin tools (1)"));
    }

    #[test]
    fn trust_requires_content_and_capability_bound_review_token() {
        let _lock = crate::test_support::lock_test_env();
        let root = TempDir::new().unwrap();
        let _home =
            crate::test_support::EnvVarGuard::set("CODEWHALE_HOME", root.path().join("home"));
        write_bundle(root.path());
        let (mut app, _temp) = create_test_app(root.path());
        crate::plugins::init_registry(root.path());

        let enable_review = plugins(&mut app, Some("enable demo"));
        assert!(!enable_review.is_error);
        assert!(
            enable_review
                .message
                .as_deref()
                .is_some_and(|message| message.contains("/plugin trust demo "))
        );
        assert!(!crate::plugins::try_with_registry(|r| r.get("demo").unwrap().trusted()).unwrap());

        let review = plugins(&mut app, Some("trust demo")).message.unwrap();
        let confirmation = review
            .lines()
            .find(|line| line.starts_with("/plugin trust demo "))
            .unwrap();
        assert!(!crate::plugins::try_with_registry(|r| r.get("demo").unwrap().trusted()).unwrap());

        assert!(plugins(&mut app, Some("trust demo wrong")).is_error);
        let arg = confirmation.trim_start_matches("/plugin ");
        assert!(!plugins(&mut app, Some(arg)).is_error);
        assert!(!plugins(&mut app, Some("enable demo")).is_error);
        assert!(crate::plugins::try_with_registry(|r| r.is_active("demo")).unwrap());
        assert!(!plugins(&mut app, Some("disable demo")).is_error);
        assert!(!crate::plugins::try_with_registry(|r| r.is_active("demo")).unwrap());
    }

    #[test]
    fn legacy_tool_detail_remains_available_under_tools_namespace() {
        let _lock = crate::test_support::lock_test_env();
        let root = TempDir::new().unwrap();
        let _home =
            crate::test_support::EnvVarGuard::set("CODEWHALE_HOME", root.path().join("home"));
        let (mut app, _temp) = create_test_app(root.path());
        fs::write(
            root.path().join("tools/greet.sh"),
            "# name: greet\n# description: Say hello\n# approval: required\n",
        )
        .unwrap();
        crate::plugins::init_registry(root.path());
        let result = plugins(&mut app, Some("tools greet"));
        assert!(!result.is_error);
        let message = result.message.unwrap();
        assert!(message.contains("Say hello"));
        assert!(message.contains("required"));
    }
}
