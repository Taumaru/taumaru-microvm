use std::ffi::OsString;

use inquire::Confirm;
use taumaru_microvm::{PruneSummary, SdkError};

use super::download::{prompt_error, prompt_render_config};
use crate::cli::PruneArgs;
use crate::context::CliContext;
use crate::error::CliError;

pub(crate) fn escalated_child_command() -> Vec<OsString> {
    vec![
        OsString::from("artifacts"),
        OsString::from("prune"),
        OsString::from("--non-interactive"),
    ]
}

fn map_prune_error(error: SdkError) -> CliError {
    CliError::prune_failed("Artifact prune failed", &error)
}

fn prune_prompt_error(error: inquire::InquireError) -> CliError {
    match prompt_error(error) {
        CliError::Prompt(message) if message == "cancelled" => CliError::prune_cancelled(),
        CliError::Prompt(message) => CliError::prune_failed(
            "Artifact prune confirmation could not be completed",
            &SdkError::Migration(message),
        ),
        other => other,
    }
}

fn confirm_deletion(terminal: crate::context::TerminalCapabilities) -> Result<bool, CliError> {
    Confirm::new("Delete these artifacts?")
        .with_default(false)
        .with_help_message("Enter deletes the listed artifacts  ·  Ctrl-C cancels")
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(prune_prompt_error)
}

pub(crate) async fn run(context: &CliContext, arguments: PruneArgs) -> Result<u8, CliError> {
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
        return execute_prune(context).await;
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
    let preview = context
        .sdk
        .list_prune_candidates()
        .await
        .map_err(map_prune_error)?;
    if preview.kernels.is_empty() && preview.images.is_empty() {
        crate::output::human::write_prune_empty(context.terminal).map_err(CliError::from)?;
        return Ok(0);
    }
    let images: Vec<(String, String)> = preview
        .images
        .iter()
        .map(|image| (image.distribution_id.clone(), image.image_id.clone()))
        .collect();
    crate::output::human::write_prune_preview(
        &preview.kernels,
        &images,
        preview.estimated_bytes,
        context.terminal,
    )
    .map_err(CliError::from)?;
    if !confirm_deletion(context.terminal)? {
        return Err(CliError::prune_cancelled());
    }
    execute_prune(context).await
}

async fn execute_prune(context: &CliContext) -> Result<u8, CliError> {
    match context.sdk.prune_unused_artifacts().await {
        Ok(summary) => {
            render_summary(context, &summary)?;
            Ok(0)
        }
        Err(SdkError::PruneIncomplete { summary, failures }) => {
            crate::output::human::write_prune_partial(&summary, &failures, context.terminal)
                .map_err(CliError::from)?;
            Ok(1)
        }
        Err(error) => Err(map_prune_error(error)),
    }
}

fn render_summary(context: &CliContext, summary: &PruneSummary) -> Result<(), CliError> {
    if summary.removed_kernels.is_empty() && summary.removed_images.is_empty() {
        crate::output::human::write_prune_empty(context.terminal).map_err(CliError::from)?;
    } else {
        crate::output::human::write_prune_result(summary, context.terminal)
            .map_err(CliError::from)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::escalated_child_command;

    #[test]
    fn escalated_child_reruns_non_interactive_prune() {
        let rendered: Vec<String> = escalated_child_command()
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, ["artifacts", "prune", "--non-interactive"]);
    }

    #[test]
    fn declined_confirmation_maps_to_prune_cancelled() {
        let error = crate::error::CliError::prune_cancelled();
        assert_eq!(error.exit_code(), 130);
        assert!(
            error
                .user_message(false)
                .contains("No artifact was deleted")
        );
    }

    #[test]
    fn empty_summary_renders_nothing_to_prune() {
        let summary = taumaru_microvm::PruneSummary {
            removed_kernels: Vec::new(),
            removed_images: Vec::new(),
            skipped_artifact_keys: Vec::new(),
            freed_bytes_kernels: 0,
            freed_bytes_images: 0,
            freed_bytes_total: 0,
        };
        assert!(summary.removed_kernels.is_empty() && summary.removed_images.is_empty());
        let report =
            crate::output::human::format_prune_empty(crate::context::TerminalCapabilities {
                interactive: false,
                color: false,
                width: Some(120),
            });
        assert!(report.contains("Nothing to prune"));
        assert!(!report.contains("removed"));
    }

    #[test]
    fn incomplete_maps_to_partial_with_exit_one() {
        let error = crate::error::CliError::prune_failed(
            "Artifact prune failed",
            &taumaru_microvm::SdkError::PruneIncomplete {
                summary: taumaru_microvm::PruneSummary {
                    removed_kernels: vec!["linux-6.18".to_owned()],
                    removed_images: Vec::new(),
                    skipped_artifact_keys: Vec::new(),
                    freed_bytes_kernels: 10,
                    freed_bytes_images: 0,
                    freed_bytes_total: 10,
                },
                failures: vec![taumaru_microvm::PruneFailure {
                    artifact_key: "kernel:stuck".to_owned(),
                    reason: "permission denied".to_owned(),
                }],
            },
        );
        assert_eq!(error.exit_code(), 1);
        assert!(error.user_message(false).contains("Artifact prune failed"));
    }

    #[test]
    fn generic_failure_maps_to_prune_triplet() {
        let error = crate::error::CliError::prune_failed(
            "Artifact prune failed",
            &taumaru_microvm::SdkError::Migration("inventory is locked".to_owned()),
        );
        let message = error.user_message(false);
        assert!(message.contains("Artifact prune failed"));
        assert!(message.contains("retry the prune"));
        assert_eq!(error.exit_code(), 1);
    }

    #[test]
    fn interactive_gate_plans_prune_escalation() {
        struct Unprivileged;
        impl crate::privilege::Privilege for Unprivileged {
            fn effective_user_id(&self) -> u32 {
                1000
            }
            fn escalated_marker(&self) -> bool {
                false
            }
            fn executable_path(&self) -> Result<std::path::PathBuf, crate::error::CliError> {
                Ok(std::path::PathBuf::from("/usr/bin/microvm"))
            }
            fn backend_path(&self, program: &str) -> Option<std::path::PathBuf> {
                if program == "sudo" {
                    return Some(std::path::PathBuf::from("/usr/bin/sudo"));
                }
                None
            }
            async fn spawn_elevated(
                &self,
                _plan: &crate::privilege::EscalationPlan,
            ) -> Result<std::process::ExitStatus, crate::error::CliError> {
                Err(crate::error::CliError::privileged(
                    "retry with elevated rights",
                ))
            }
        }
        let backend = Unprivileged;
        let crate::privilege::EscalationDecision::Escalate(plan) =
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
        assert!(rendered.contains(&String::from("prune")));
        assert!(rendered.contains(&String::from("--non-interactive")));
        assert_eq!(
            crate::privilege::decide(&UnprivilegedEscalated, None),
            crate::privilege::EscalationDecision::Unavailable
        );
    }

    struct UnprivilegedEscalated;
    impl crate::privilege::Privilege for UnprivilegedEscalated {
        fn effective_user_id(&self) -> u32 {
            1000
        }
        fn escalated_marker(&self) -> bool {
            true
        }
        fn executable_path(&self) -> Result<std::path::PathBuf, crate::error::CliError> {
            Ok(std::path::PathBuf::from("/usr/bin/microvm"))
        }
        fn backend_path(&self, _program: &str) -> Option<std::path::PathBuf> {
            None
        }
        async fn spawn_elevated(
            &self,
            _plan: &crate::privilege::EscalationPlan,
        ) -> Result<std::process::ExitStatus, crate::error::CliError> {
            Err(crate::error::CliError::privileged(
                "retry with elevated rights",
            ))
        }
    }
}
