use std::ffi::OsString;

use taumaru_microvm::SdkError;

use crate::cli::LsArgs;
use crate::context::CliContext;
use crate::error::CliError;

pub(crate) fn escalated_child_command() -> Vec<OsString> {
    vec![OsString::from("ls")]
}

fn map_ls_error(error: SdkError) -> CliError {
    CliError::ls_failed("MicroVM listing failed", &error)
}

pub(crate) async fn run(context: &CliContext, arguments: LsArgs) -> Result<u8, CliError> {
    if arguments.non_interactive {
        if let Some(exit) = crate::privilege::require_privileged(
            &crate::privilege::SystemPrivilege,
            context.terminal,
            true,
            crate::context::resolve_home().ok(),
            &[],
            Vec::new(),
            "Run the same command with sudo or as root",
        )
        .await?
        {
            return Ok(exit);
        }
        return execute_list(context).await;
    }

    let home = crate::context::resolve_home().ok();
    if let Some(exit) = crate::privilege::require_privileged(
        &crate::privilege::SystemPrivilege,
        context.terminal,
        false,
        home,
        &[],
        escalated_child_command(),
        "Run the same command with sudo or as root",
    )
    .await?
    {
        return Ok(exit);
    }
    execute_list(context).await
}

async fn execute_list(context: &CliContext) -> Result<u8, CliError> {
    let listed = context.sdk.list_microvms().await.map_err(map_ls_error)?;
    if listed.is_empty() {
        crate::output::human::write_ls_empty(context.terminal).map_err(CliError::from)?;
    } else {
        crate::output::human::write_ls_table(&listed, context.terminal).map_err(CliError::from)?;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::{escalated_child_command, map_ls_error};

    #[test]
    fn escalated_child_reruns_bare_ls() {
        let rendered: Vec<String> = escalated_child_command()
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, ["ls"]);
    }

    #[test]
    fn listing_failure_maps_to_ls_triplet() {
        use taumaru_microvm::SdkError;
        let error = map_ls_error(SdkError::Migration("inventory is locked".to_owned()));
        let message = error.user_message(false);
        assert!(message.contains("MicroVM listing failed"));
        assert!(message.contains("retry the listing"));
        assert_eq!(error.exit_code(), 1);
    }

    #[test]
    fn empty_report_points_at_creation() {
        let report = crate::output::human::format_ls_empty(crate::context::TerminalCapabilities {
            interactive: false,
            color: false,
            width: Some(120),
        });
        assert!(report.contains("No MicroVMs yet"));
        assert!(report.contains("microvm new"));
    }
}

#[cfg(test)]
mod privilege_tests {
    use std::path::PathBuf;

    use crate::error::CliError;
    use crate::privilege::{EscalationDecision, Privilege};

    struct Unprivileged {
        escalated: bool,
    }

    impl Privilege for Unprivileged {
        fn effective_user_id(&self) -> u32 {
            1000
        }

        fn escalated_marker(&self) -> bool {
            self.escalated
        }

        fn executable_path(&self) -> Result<PathBuf, CliError> {
            Ok(PathBuf::from("/usr/bin/microvm"))
        }

        fn backend_path(&self, program: &str) -> Option<PathBuf> {
            if program == "sudo" {
                return Some(PathBuf::from("/usr/bin/sudo"));
            }
            None
        }

        async fn spawn_elevated(
            &self,
            _plan: &crate::privilege::EscalationPlan,
        ) -> Result<std::process::ExitStatus, CliError> {
            Err(CliError::privileged("retry with elevated rights"))
        }
    }

    #[test]
    fn bare_ls_child_command_has_no_name_to_carry() {
        assert_eq!(
            super::escalated_child_command(),
            vec![std::ffi::OsString::from("ls")]
        );
    }

    #[test]
    fn escalated_child_never_re_escalates() {
        let backend = Unprivileged { escalated: true };
        assert_eq!(
            crate::privilege::decide(&backend, None),
            EscalationDecision::Unavailable
        );
    }

    #[test]
    fn interactive_gate_plans_bare_ls_escalation() {
        let backend = Unprivileged { escalated: false };
        let EscalationDecision::Escalate(plan) =
            crate::privilege::resolve_plan(&backend, None, &[], super::escalated_child_command())
                .expect("escalation should resolve")
        else {
            panic!("unprivileged terminal should escalate");
        };
        let rendered: Vec<String> = plan
            .arguments
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert!(rendered.contains(&String::from("ls")));
        assert!(!rendered.iter().any(|part| part == "--non-interactive"));
    }
}
