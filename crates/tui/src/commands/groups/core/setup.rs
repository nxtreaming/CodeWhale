//! `/setup` command.

use crate::commands::traits::{CommandInfo, RegisterCommand};
use crate::localization::MessageId;
use crate::tui::app::{App, AppAction};

use super::CommandResult;
use codewhale_config::SetupStep;

pub(in crate::commands) const COMMAND_INFO: CommandInfo = CommandInfo {
    name: "setup",
    aliases: &[],
    usage: "/setup",
    description_id: MessageId::CmdSetupDescription,
};

pub(in crate::commands) struct SetupCmd;

impl RegisterCommand for SetupCmd {
    fn info() -> &'static CommandInfo {
        &COMMAND_INFO
    }

    fn execute(_app: &mut App, arg: Option<&str>) -> CommandResult {
        match arg.map(str::trim).filter(|arg| !arg.is_empty()) {
            None | Some("open" | "wizard" | "checkpoint") => {
                CommandResult::action(AppAction::OpenSetupWizard)
            }
            Some("provider" | "providers" | "model" | "models" | "route") => {
                CommandResult::action(AppAction::OpenSetupWizardAt {
                    step: SetupStep::ProviderModel,
                })
            }
            Some("runtime" | "posture" | "trust" | "sandbox") => {
                CommandResult::action(AppAction::OpenSetupWizardAt {
                    step: SetupStep::TrustSandbox,
                })
            }
            Some("constitution" | "law") => CommandResult::action(AppAction::OpenSetupWizardAt {
                step: SetupStep::Constitution,
            }),
            Some("status" | "report" | "verification" | "verify") => {
                CommandResult::action(AppAction::OpenSetupWizardAt {
                    step: SetupStep::Verification,
                })
            }
            Some("operate" | "fleet" | "operate-fleet" | "operate_fleet") => {
                CommandResult::action(AppAction::OpenSetupWizardAt {
                    step: SetupStep::OperateFleet,
                })
            }
            Some("hotbar" | "hotkeys" | "shortcuts" | "keys") => {
                CommandResult::action(AppAction::OpenSetupWizardAt {
                    step: SetupStep::Hotbar,
                })
            }
            Some(
                "remote" | "remote-runtime" | "remote_runtime" | "cloud" | "bridge" | "mobile"
                | "phone",
            ) => CommandResult::action(AppAction::OpenSetupWizardAt {
                step: SetupStep::RemoteRuntime,
            }),
            Some(other) => CommandResult::error(format!(
                "Unknown /setup target '{other}'. Try `/setup` to open the setup wizard."
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::tui::app::TuiOptions;
    use std::path::PathBuf;

    fn test_app() -> App {
        let options = TuiOptions {
            model: "deepseek-v4-pro".to_string(),
            workspace: PathBuf::from("."),
            config_path: None,
            config_profile: None,
            allow_shell: false,
            use_alt_screen: true,
            use_mouse_capture: false,
            use_bracketed_paste: true,
            max_subagents: 1,
            skills_dir: PathBuf::from("."),
            memory_path: PathBuf::from("memory.md"),
            notes_path: PathBuf::from("notes.txt"),
            mcp_config_path: PathBuf::from("mcp.json"),
            use_memory: false,
            start_in_agent_mode: false,
            skip_onboarding: true,
            yolo: false,
            resume_session_id: None,
            initial_input: None,
        };
        App::new(options, &Config::default())
    }

    #[test]
    fn setup_command_opens_wizard() {
        let mut app = test_app();

        let result = SetupCmd::execute(&mut app, None);

        assert_eq!(result.action, Some(AppAction::OpenSetupWizard));
        assert!(result.message.is_none());
    }

    #[test]
    fn setup_checkpoint_alias_opens_wizard() {
        let mut app = test_app();

        let result = SetupCmd::execute(&mut app, Some("checkpoint"));

        assert_eq!(result.action, Some(AppAction::OpenSetupWizard));
        assert!(result.message.is_none());
    }

    #[test]
    fn setup_report_opens_verification_step() {
        let mut app = test_app();

        let result = SetupCmd::execute(&mut app, Some("report"));

        assert_eq!(
            result.action,
            Some(AppAction::OpenSetupWizardAt {
                step: SetupStep::Verification
            })
        );
        assert!(result.message.is_none());
    }

    #[test]
    fn setup_named_steps_open_matching_wizard_cards() {
        let cases = [
            ("provider", SetupStep::ProviderModel),
            ("model", SetupStep::ProviderModel),
            ("runtime", SetupStep::TrustSandbox),
            ("posture", SetupStep::TrustSandbox),
            ("constitution", SetupStep::Constitution),
            ("hotbar", SetupStep::Hotbar),
            ("shortcuts", SetupStep::Hotbar),
            ("remote", SetupStep::RemoteRuntime),
            ("cloud", SetupStep::RemoteRuntime),
        ];

        for (arg, step) in cases {
            let mut app = test_app();
            let result = SetupCmd::execute(&mut app, Some(arg));
            assert_eq!(
                result.action,
                Some(AppAction::OpenSetupWizardAt { step }),
                "{arg}"
            );
            assert!(result.message.is_none(), "{arg}");
        }
    }

    #[test]
    fn setup_fleet_opens_operate_readiness_step() {
        let mut app = test_app();

        let result = SetupCmd::execute(&mut app, Some("fleet"));

        assert_eq!(
            result.action,
            Some(AppAction::OpenSetupWizardAt {
                step: SetupStep::OperateFleet
            })
        );
        assert!(result.message.is_none());
    }
}
