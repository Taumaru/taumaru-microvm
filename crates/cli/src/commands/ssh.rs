use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;

use inquire::Select;
use taumaru_microvm::{RunningMicroVm, SshConnectionInfo};

use super::download::prompt_render_config;
use super::new::{resolve_name, validate_name};
use crate::cli::SshArgs;
use crate::context::{CliContext, TerminalCapabilities};
use crate::error::CliError;

#[derive(Clone, Debug)]
struct MachineOption {
    id: String,
    label: String,
}

impl std::fmt::Display for MachineOption {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.label)
    }
}

pub(crate) fn escalated_child_command(name: &str, remote: &[String]) -> Vec<OsString> {
    let mut command = vec![
        OsString::from("ssh"),
        OsString::from(name),
        OsString::from("--non-interactive"),
    ];
    if !remote.is_empty() {
        command.push(OsString::from("--"));
        command.extend(remote.iter().map(OsString::from));
    }
    command
}

fn ssh_prompt_error(error: inquire::InquireError) -> CliError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("cancel") || normalized.contains("interrupt") {
        CliError::ssh_cancelled()
    } else {
        CliError::creation(
            "MicroVM connection selection could not be completed",
            message,
            "Check terminal input and try again",
        )
    }
}

async fn prompt_machine(
    machines: &[(String, taumaru_microvm::MicroVmState)],
    terminal: TerminalCapabilities,
) -> Result<String, CliError> {
    let options: Vec<MachineOption> = machines
        .iter()
        .map(|(name, state)| MachineOption {
            id: name.clone(),
            label: format!("{name} [{state}]"),
        })
        .collect();
    let selected = Select::new("Choose a MicroVM to connect to", options)
        .with_help_message("↑↓ move  ·  enter confirm")
        .with_page_size(10)
        .with_render_config(prompt_render_config(terminal.color))
        .prompt()
        .map_err(ssh_prompt_error)?;
    validate_name(&selected.id)
}

async fn resolve_running_entry(
    context: &CliContext,
    name: &str,
) -> Result<RunningMicroVm, CliError> {
    let running = context.sdk.list_running_microvms().await?;
    if let Some(entry) = running.into_iter().find(|entry| entry.name == name) {
        return Ok(entry);
    }
    let inventory = context.sdk.list_microvms().await?;
    match inventory.into_iter().find(|machine| machine.name == name) {
        Some(known) => Err(CliError::ssh_not_running(name, known.state.to_string())),
        None => Err(CliError::ssh_not_found(name)),
    }
}

fn preflight_key(path: &Path) -> Result<(), CliError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| CliError::ssh_key_unreadable(path))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CliError::ssh_key_unreadable(path));
    }
    std::fs::File::open(path)
        .map(|_| ())
        .map_err(|_| CliError::ssh_key_unreadable(path))
}

fn ssh_argv(ssh: &SshConnectionInfo, remote: &[String]) -> Vec<OsString> {
    let mut argv = Vec::new();
    argv.push(OsString::from("-i"));
    argv.push(ssh.private_key_path.as_os_str().to_owned());
    if ssh.port != 22 {
        argv.push(OsString::from("-p"));
        argv.push(OsString::from(ssh.port.to_string()));
    }
    argv.push(OsString::from(format!("{}@{}", ssh.user, ssh.address)));
    argv.extend(remote.iter().map(OsString::from));
    argv
}

fn build_session_command(ssh_binary: &Path, argv: &[OsString]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(ssh_binary);
    command.args(argv);
    command.stdin(Stdio::inherit());
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());
    command
}

fn map_session_status(status: std::process::ExitStatus) -> u8 {
    status.code().map(|code| code as u8).unwrap_or(130)
}

async fn execute_session(entry: &RunningMicroVm, remote: &[String]) -> Result<u8, CliError> {
    let ssh_binary =
        crate::privilege::backend_path("ssh").ok_or_else(CliError::ssh_client_missing)?;
    preflight_key(&entry.ssh.private_key_path)?;
    let argv = ssh_argv(&entry.ssh, remote);
    let mut child = build_session_command(&ssh_binary, &argv)
        .spawn()
        .map_err(CliError::from)?;
    let status = child.wait().await.map_err(CliError::from)?;
    Ok(map_session_status(status))
}

fn split_request(arguments: &SshArgs) -> Result<(Option<String>, Vec<String>), CliError> {
    if let Some(positional) = arguments.name.as_deref() {
        let name = resolve_name(Some(positional), arguments.explicit_name.as_deref())?;
        return Ok((Some(name), arguments.command.clone()));
    }
    if let Some(explicit) = arguments.explicit_name.as_deref() {
        let name = resolve_name(None, Some(explicit))?;
        return Ok((Some(name), arguments.command.clone()));
    }
    Ok((None, arguments.command.clone()))
}

pub(crate) async fn run(context: &CliContext, arguments: SshArgs) -> Result<u8, CliError> {
    let (named, remote) = split_request(&arguments)?;
    if arguments.non_interactive {
        let name = resolve_name(named.as_deref(), arguments.explicit_name.as_deref()).map_err(
            |error| match error {
                CliError::MissingValue(_) => CliError::missing_value(
                    "machine name",
                    "--name <NAME>",
                    "Run microvm ssh web-01 --non-interactive",
                ),
                other => other,
            },
        )?;
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
        let entry = resolve_running_entry(context, &name).await?;
        return execute_session(&entry, &remote).await;
    }

    if named.is_none() {
        let home = crate::context::resolve_home().ok();
        let command = vec![OsString::from("ssh")];
        if let Some(exit) = crate::privilege::require_privileged(
            &crate::privilege::SystemPrivilege,
            context.terminal,
            false,
            home,
            &[],
            command,
            "Run the same command with sudo or as root",
        )
        .await?
        {
            return Ok(exit);
        }
    }
    if named.is_none() {
        if !context.terminal.interactive {
            return Err(CliError::creation(
                "An interactive terminal is required",
                "no machine name was supplied and input is not interactive",
                "Run microvm ssh <NAME> --non-interactive with root access",
            ));
        }
        let inventory = context.sdk.list_microvms().await?;
        let machines: Vec<(String, taumaru_microvm::MicroVmState)> = inventory
            .into_iter()
            .filter(|machine| machine.state == taumaru_microvm::MicroVmState::Running)
            .map(|machine| (machine.name, machine.state))
            .collect();
        if machines.is_empty() {
            return Err(CliError::ssh_empty());
        }
        let name = prompt_machine(&machines, context.terminal).await?;
        let home = crate::context::resolve_home().ok();
        let command = escalated_child_command(&name, &remote);
        if let Some(exit) = crate::privilege::require_privileged(
            &crate::privilege::SystemPrivilege,
            context.terminal,
            false,
            home,
            &[],
            command,
            "Run the same command with sudo or as root",
        )
        .await?
        {
            return Ok(exit);
        }
        let entry = resolve_running_entry(context, &name).await?;
        return execute_session(&entry, &remote).await;
    }

    let Some(name) = named else {
        unreachable!("split_request only yields none on the selector path above");
    };
    if remote.is_empty() && !context.terminal.interactive {
        return Err(CliError::creation(
            "An interactive terminal is required",
            "an interactive shell needs a terminal for input and output",
            "Run microvm ssh <NAME> -- <command> for scripted use, or run in a terminal",
        ));
    }
    let home = crate::context::resolve_home().ok();
    let command = escalated_child_command(&name, &remote);
    if let Some(exit) = crate::privilege::require_privileged(
        &crate::privilege::SystemPrivilege,
        context.terminal,
        false,
        home,
        &[],
        command,
        "Run the same command with sudo or as root",
    )
    .await?
    {
        return Ok(exit);
    }
    let entry = resolve_running_entry(context, &name).await?;
    execute_session(&entry, &remote).await
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;

    use taumaru_microvm::SshConnectionInfo;

    use super::{build_session_command, escalated_child_command, map_session_status, ssh_argv};
    use super::{preflight_key, resolve_name};

    fn test_ssh(user: &str, port: u16, address: IpAddr) -> SshConnectionInfo {
        SshConnectionInfo {
            user: user.to_owned(),
            port,
            address,
            private_key_path: PathBuf::from("/tmp/vms/web-01/ssh/id_ed25519"),
            public_key_path: PathBuf::from("/tmp/vms/web-01/ssh/id_ed25519.pub"),
        }
    }

    #[test]
    fn positional_and_flag_names_agree_or_abort() {
        assert_eq!(
            resolve_name(Some("web-01"), None).expect("positional name should resolve"),
            "web-01"
        );
        assert_eq!(
            resolve_name(None, Some("web-01")).expect("flag name should resolve"),
            "web-01"
        );
        assert!(resolve_name(Some("web-01"), Some("db-01")).is_err());
    }

    #[test]
    fn invalid_names_abort_with_the_rule() {
        let error = resolve_name(Some("not path friendly"), None).expect_err("invalid name aborts");
        assert!(error.user_message(false).contains("1-64 ASCII"));
    }

    #[test]
    fn escalated_child_carries_name_and_remote_command() {
        let rendered: Vec<String> =
            escalated_child_command("web-01", &["uname".to_owned(), "-a".to_owned()])
                .iter()
                .map(|part| part.to_string_lossy().into_owned())
                .collect();
        assert_eq!(
            rendered,
            ["ssh", "web-01", "--non-interactive", "--", "uname", "-a"]
        );
    }

    #[test]
    fn escalated_child_omits_separator_without_remote_command() {
        let rendered: Vec<String> = escalated_child_command("web-01", &[])
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(rendered, ["ssh", "web-01", "--non-interactive"]);
    }

    #[test]
    fn ssh_argv_matches_manual_form_for_default_port() {
        let ssh = test_ssh("root", 22, IpAddr::V4(Ipv4Addr::new(10, 200, 8, 2)));
        let rendered: Vec<String> = ssh_argv(&ssh, &[])
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            ["-i", "/tmp/vms/web-01/ssh/id_ed25519", "root@10.200.8.2"]
        );
    }

    #[test]
    fn ssh_argv_adds_port_flag_and_verbatim_remote_command() {
        let ssh = test_ssh("root", 2222, IpAddr::V4(Ipv4Addr::new(10, 200, 8, 2)));
        let remote = [
            "echo".to_owned(),
            "hello world".to_owned(),
            "--all".to_owned(),
        ];
        let rendered: Vec<String> = ssh_argv(&ssh, &remote)
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            [
                "-i",
                "/tmp/vms/web-01/ssh/id_ed25519",
                "-p",
                "2222",
                "root@10.200.8.2",
                "echo",
                "hello world",
                "--all"
            ]
        );
    }

    #[test]
    fn ssh_argv_adds_no_options_beyond_identity_port_and_target() {
        for port in [22, 2222] {
            let ssh = test_ssh("root", port, IpAddr::V4(Ipv4Addr::new(10, 200, 8, 2)));
            let rendered: Vec<String> = ssh_argv(
                &ssh,
                &["echo".to_owned(), "StrictHostKeyChecking=no".to_owned()],
            )
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
            let structural: Vec<&str> = rendered
                .iter()
                .take(if port == 22 { 4 } else { 6 })
                .map(String::as_str)
                .collect();
            if port == 22 {
                assert_eq!(structural[0], "-i");
                assert!(structural[2].starts_with("root@"));
                assert!(!structural.contains(&"-o"));
            } else {
                assert_eq!(structural[0], "-i");
                assert_eq!(structural[2], "-p");
                assert_eq!(structural[3], "2222");
                assert!(!structural[..3].contains(&"-o"));
            }
            assert!(
                !rendered
                    .iter()
                    .take(rendered.len() - 2)
                    .any(|part| part == "-o" || part.contains("StrictHostKeyChecking")),
                "the builder adds no host-key options for port {port}"
            );
            assert_eq!(
                rendered[rendered.len() - 2..],
                ["echo".to_owned(), "StrictHostKeyChecking=no".to_owned()],
                "remote words stay verbatim for port {port}"
            );
        }
    }

    #[test]
    fn split_request_keeps_name_and_remote_command() {
        use crate::cli::SshArgs;
        let arguments = SshArgs {
            name: Some("web-01".to_owned()),
            explicit_name: None,
            non_interactive: false,
            command: vec!["uname".to_owned(), "-a".to_owned()],
        };
        let (named, remote) = super::split_request(&arguments).expect("name should split");
        assert_eq!(named.as_deref(), Some("web-01"));
        assert_eq!(remote, ["uname".to_owned(), "-a".to_owned()]);
    }

    #[test]
    fn split_request_keeps_explicit_name_and_remote_command() {
        use crate::cli::SshArgs;
        let arguments = SshArgs {
            name: None,
            explicit_name: Some("web-01".to_owned()),
            non_interactive: true,
            command: vec!["uname".to_owned()],
        };
        let (named, remote) = super::split_request(&arguments).expect("explicit name should split");
        assert_eq!(named.as_deref(), Some("web-01"));
        assert_eq!(remote, ["uname".to_owned()]);
    }

    #[test]
    fn split_request_rejects_mismatched_names() {
        use crate::cli::SshArgs;
        let arguments = SshArgs {
            name: Some("web-01".to_owned()),
            explicit_name: Some("db-01".to_owned()),
            non_interactive: true,
            command: vec![],
        };
        assert!(super::split_request(&arguments).is_err());
    }

    #[test]
    fn split_request_yields_selector_when_no_name_given() {
        use crate::cli::SshArgs;
        let arguments = SshArgs {
            name: None,
            explicit_name: None,
            non_interactive: false,
            command: vec!["uname".to_owned(), "-a".to_owned()],
        };
        let (named, remote) =
            super::split_request(&arguments).expect("nameless input selects first");
        assert!(named.is_none());
        assert_eq!(remote, ["uname".to_owned(), "-a".to_owned()]);
    }

    #[test]
    fn missing_key_maps_to_guided_error_before_any_session() {
        let missing = PathBuf::from("/tmp/definitely-missing-taumaru-key/id_ed25519");
        let error = preflight_key(&missing).expect_err("missing key aborts");
        let message = error.user_message(false);
        assert!(message.contains("SSH key"));
        assert!(message.contains("id_ed25519"));
    }

    #[test]
    fn empty_running_set_points_at_start() {
        let error = crate::error::CliError::ssh_empty();
        assert_eq!(error.exit_code(), 1);
        assert!(error.user_message(false).contains("microvm start"));
    }

    #[test]
    fn unknown_machine_points_at_creation_without_shortcut() {
        let error = crate::error::CliError::ssh_not_found("web-01");
        assert_eq!(error.exit_code(), 1);
        let message = error.user_message(false);
        assert!(message.contains("web-01"));
        assert!(message.contains("microvm new"));
    }

    #[test]
    fn known_but_stopped_machine_points_at_start_with_state() {
        let error = crate::error::CliError::ssh_not_running("web-01", "configured");
        assert_eq!(error.exit_code(), 1);
        let message = error.user_message(false);
        assert!(message.contains("web-01"));
        assert!(message.contains("configured"));
        assert!(message.contains("microvm start web-01"));
    }

    #[test]
    fn ssh_cancellation_reports_exit_130() {
        let error = crate::error::CliError::ssh_cancelled();
        assert_eq!(error.exit_code(), 130);
        assert!(error.user_message(false).contains("No session was opened"));
    }

    #[test]
    fn session_exit_status_propagates_verbatim() {
        let ok = std::process::ExitStatus::from_raw(0);
        assert_eq!(map_session_status(ok), 0);
        let failed = std::process::ExitStatus::from_raw(7 << 8);
        assert_eq!(map_session_status(failed), 7);
        let signaled = std::process::ExitStatus::from_raw(9);
        assert!(signaled.code().is_none());
        assert_eq!(map_session_status(signaled), 130);
    }

    #[test]
    fn session_child_inherits_all_stdio_without_pipes() {
        let command = build_session_command(
            std::path::Path::new("ssh"),
            &[std::ffi::OsString::from("-V")],
        );
        let debug = format!("{command:?}");
        assert!(
            !debug.contains("piped"),
            "session stdio must be inherited, never piped: {debug}"
        );
        assert!(
            !debug.contains("null"),
            "session stdio must reach the terminal: {debug}"
        );
    }

    #[test]
    fn session_success_path_prints_nothing_for_clean_pipes() {
        let ssh = test_ssh("root", 22, IpAddr::V4(Ipv4Addr::new(10, 200, 8, 2)));
        let argv = super::ssh_argv(&ssh, &["uname".to_owned(), "-a".to_owned()]);
        let rendered: Vec<String> = argv
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            [
                "-i",
                "/tmp/vms/web-01/ssh/id_ed25519",
                "root@10.200.8.2",
                "uname",
                "-a"
            ]
        );
    }

    #[test]
    fn running_prefilter_keeps_only_verified_running_machines() {
        use taumaru_microvm::{MicroVmState, MicroVmSummary};
        let inventory = vec![
            MicroVmSummary {
                name: "web-01".to_owned(),
                state: MicroVmState::Running,
            },
            MicroVmSummary {
                name: "db-01".to_owned(),
                state: MicroVmState::Stopped,
            },
        ];
        let machines: Vec<(String, taumaru_microvm::MicroVmState)> = inventory
            .into_iter()
            .filter(|machine| machine.state == taumaru_microvm::MicroVmState::Running)
            .map(|machine| (machine.name, machine.state))
            .collect();
        assert_eq!(
            machines,
            [("web-01".to_owned(), taumaru_microvm::MicroVmState::Running)]
        );
    }

    #[test]
    fn stopped_machines_render_stopped_error_labels() {
        let state = taumaru_microvm::MicroVmState::Stopped;
        assert_eq!(state.to_string(), "stopped");
    }
}
