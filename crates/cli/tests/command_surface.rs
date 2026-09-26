use std::process::{Command, Output};

unsafe fn libc_geteuid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

fn run_microvm(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(arguments)
        .output()
        .expect("failed to execute microvm")
}

#[test]
fn help_identifies_the_microvm_command() {
    let output = run_microvm(&["--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm"));
    assert!(stdout.contains("Build and manage Firecracker MicroVMs on Linux."));
}

#[test]
fn snapshot_help_exposes_name_and_output_path_without_a_password_option() {
    let output = run_microvm(&["snapshot", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm snapshot"));
    assert!(stdout.contains("[NAME]"));
    assert!(stdout.contains("[OUTPUT_PATH]"));
    assert!(!stdout.contains("--password"));
    assert!(stdout.contains("--address-policy <POLICY>"));
}

#[test]
fn restore_help_exposes_archive_and_non_interactive_options_without_a_password_option() {
    let output = run_microvm(&["restore", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm restore"));
    assert!(stdout.contains("[ARCHIVE_PATH]"));
    assert!(!stdout.contains("--password"));
    assert!(stdout.contains("--non-interactive"));
}

#[test]
fn non_interactive_snapshot_requires_an_explicit_address_policy() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["snapshot", "web-01"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Missing snapshot IPv4 policy"));
    assert!(!stderr.to_ascii_lowercase().contains("password"));
    assert!(!stderr.contains("Choose a MicroVM"));
}

#[test]
fn non_interactive_restore_rejects_a_missing_archive_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["restore", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Missing snapshot archive path"));
    assert!(!stderr.contains("Snapshot archive path"));
}

#[test]
fn non_interactive_restore_does_not_require_a_password() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["restore", "./backup.tmvmsnap", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(!stderr.to_ascii_lowercase().contains("password"));
    let normalized_stderr = stderr.to_ascii_lowercase();
    assert!(
        normalized_stderr.contains("restore failed")
            || normalized_stderr.contains("snapshot archive")
            || normalized_stderr.contains("elevated rights are required")
    );
}

#[test]
fn supplied_restore_password_is_not_echoed_on_failure() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let password = "restore-password-must-not-echo";
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args([
            "restore",
            "./missing.tmvmsnap",
            "--password",
            password,
            "--non-interactive",
        ])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("unexpected argument"));
    assert!(!stdout.contains(password));
    assert!(!stderr.contains(password));
}

#[test]
fn non_interactive_snapshot_rejects_missing_name_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .arg("snapshot")
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Missing machine name"));
    assert!(!stderr.contains("Choose a MicroVM"));
}

#[test]
fn removed_snapshot_password_option_is_rejected_without_echoing_its_value() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["snapshot", "--password", "snapshot-password-must-not-echo"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("unexpected argument"));
    assert!(!stderr.contains("snapshot-password-must-not-echo"));
    assert!(!stderr.contains("elevated rights"));
}

#[test]
fn non_interactive_snapshot_does_not_require_a_password() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["snapshot", "web-01"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Missing snapshot IPv4 policy"));
    assert!(!stderr.to_ascii_lowercase().contains("password"));
    assert!(!stderr.contains("elevated rights"));
}

#[test]
fn version_exits_successfully() {
    let output = run_microvm(&["--version"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("0.2.0"));
}

#[test]
fn no_arguments_show_help_and_exit_successfully() {
    let output = run_microvm(&[]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm"));
}

#[test]
fn unknown_command_returns_a_clear_error() {
    let output = run_microvm(&["unknown"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("error"));
    assert!(stderr.contains("unknown"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn invalid_option_returns_a_clear_error() {
    let output = run_microvm(&["--unknown"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("error"));
    assert!(stderr.contains("--unknown"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn artifacts_download_help_exposes_image_selection_options() {
    let output = run_microvm(&["artifacts", "download", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm artifacts download"));
    assert!(stdout.contains("--image <DISTRIBUTION_ID=IMAGE_ID>"));
    assert!(stdout.contains("--non-interactive"));
    assert!(!stdout.contains("--distribution"));
}

#[test]
fn non_interactive_download_rejects_incomplete_selection_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["artifacts", "download", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("selection") || stderr.contains("plan"));
    assert!(!stderr.contains("Choose images"));
}

#[test]
fn non_interactive_download_without_root_reports_privilege_error() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args([
            "artifacts",
            "download",
            "--non-interactive",
            "--image",
            "distro-a=image-a",
        ])
        .env("TAUMARU_HOME", home.path())
        .env_remove("TAUMARU_ESCALATED")
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    assert!(!output.status.success());
    assert!(stderr.to_ascii_lowercase().contains("elevated rights"));
}

#[test]
fn new_help_exposes_creation_options_without_lifecycle_flags() {
    let output = run_microvm(&["new", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm new"));
    assert!(stdout.contains("[NAME]"));
    assert!(stdout.contains("--image <DISTRIBUTION_ID=IMAGE_ID>"));
    assert!(stdout.contains("--disk-gb"));
    assert!(stdout.contains("--memory"));
    assert!(stdout.contains("--vcpus"));
    assert!(stdout.contains("--expose-lan"));
    assert!(stdout.contains("--non-interactive"));
    assert!(!stdout.contains("--kernel"));
    assert!(!stdout.contains("--volume-path"));
    assert!(!stdout.contains("--lan-address"));
}

#[test]
fn trusted_child_skips_catalog_discovery() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args([
            "new",
            "web-01",
            "--non-interactive",
            "--image",
            "distro-a=image-a",
            "--disk-gb",
            "20",
            "--memory",
            "2GB",
            "--vcpus",
            "2",
            "--trusted-values",
        ])
        .env("TAUMARU_HOME", home.path())
        .env("TAUMARU_ESCALATED", "1")
        .env("TAUMARU_NEW_KERNEL", "kernel-a")
        .env("TAUMARU_NEW_KERNEL_SIZE", "13")
        .env("TAUMARU_NEW_RUNTIME", "runtime-new:42")
        .env("TAUMARU_NEW_IMAGE_BYTES", "37")
        .env("TAUMARU_NEW_MIN_MEMORY_MB", "128")
        .env("TAUMARU_NEW_MIN_VCPUS", "1")
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!stderr.contains("Checking artifact registry"));
    assert!(!stderr.contains("Registry ready"));
    assert!(!stdout.contains("Checking artifact registry"));
    assert!(!stdout.contains("Registry ready"));
}

#[test]
fn non_interactive_new_without_root_reports_privilege_error() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args([
            "new",
            "web-01",
            "--non-interactive",
            "--image",
            "distro-a=image-a",
            "--disk-gb",
            "20",
            "--memory",
            "2GB",
            "--vcpus",
            "2",
        ])
        .env("TAUMARU_HOME", home.path())
        .env_remove("TAUMARU_ESCALATED")
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    assert!(!output.status.success());
    assert!(stderr.to_ascii_lowercase().contains("elevated rights"));
}

#[test]
fn non_interactive_new_rejects_missing_values_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["new", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Missing machine name"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn start_help_exposes_name_selection_without_lifecycle_flags() {
    let output = run_microvm(&["start", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm start"));
    assert!(stdout.contains("[NAME]"));
    assert!(stdout.contains("--name"));
    assert!(stdout.contains("--non-interactive"));
    assert!(!stdout.contains("--image"));
    assert!(!stdout.contains("--disk-gb"));
    assert!(!stdout.contains("--memory"));
    assert!(!stdout.contains("--vcpus"));
    assert!(!stdout.contains("--expose-lan"));
}

#[test]
fn non_interactive_start_rejects_missing_name_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["start", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Missing machine name"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn non_interactive_start_without_root_reports_privilege_error() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["start", "web-01", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .env_remove("TAUMARU_ESCALATED")
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    assert!(!output.status.success());
    assert!(stderr.to_ascii_lowercase().contains("elevated rights"));
}

#[test]
fn ssh_help_exposes_name_and_remote_command_without_lifecycle_flags() {
    let output = run_microvm(&["ssh", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm ssh"));
    assert!(stdout.contains("[NAME]"));
    assert!(stdout.contains("[COMMAND]"));
    assert!(stdout.contains("--name"));
    assert!(stdout.contains("--non-interactive"));
    assert!(!stdout.contains("--image"));
    assert!(!stdout.contains("--disk-gb"));
    assert!(!stdout.contains("--memory"));
    assert!(!stdout.contains("--vcpus"));
    assert!(!stdout.contains("--expose-lan"));
}

#[test]
fn non_interactive_ssh_rejects_missing_name_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["ssh", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Missing machine name"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn non_interactive_ssh_without_root_reports_privilege_error() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["ssh", "web-01", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .env_remove("TAUMARU_ESCALATED")
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    assert!(!output.status.success());
    assert!(stderr.to_ascii_lowercase().contains("elevated rights"));
}

#[test]
fn stop_help_exposes_name_selection_without_lifecycle_flags() {
    let output = run_microvm(&["stop", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm stop"));
    assert!(stdout.contains("[NAME]"));
    assert!(stdout.contains("--name"));
    assert!(stdout.contains("--non-interactive"));
    assert!(!stdout.contains("--image"));
    assert!(!stdout.contains("--disk-gb"));
    assert!(!stdout.contains("--memory"));
    assert!(!stdout.contains("--vcpus"));
    assert!(!stdout.contains("--expose-lan"));
}

#[test]
fn non_interactive_stop_rejects_missing_name_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["stop", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Missing machine name"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn non_interactive_stop_without_root_reports_privilege_error() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["stop", "web-01", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .env_remove("TAUMARU_ESCALATED")
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    assert!(!output.status.success());
    assert!(stderr.to_ascii_lowercase().contains("elevated rights"));
}

#[test]
fn ls_help_exposes_flag_only_surface() {
    for spelling in ["ls", "list"] {
        let output = run_microvm(&[spelling, "--help"]);
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success(), "{spelling} help should succeed");
        assert!(stdout.contains("--non-interactive"));
        assert!(!stdout.contains("--image"));
        assert!(!stdout.contains("--disk-gb"));
        assert!(!stdout.contains("--memory"));
        assert!(!stdout.contains("--vcpus"));
        assert!(!stdout.contains("--expose-lan"));
    }
}

#[test]
fn list_alias_matches_ls_help() {
    let ls = run_microvm(&["ls", "--help"]);
    let list = run_microvm(&["list", "--help"]);
    let ls_stdout = String::from_utf8_lossy(&ls.stdout);
    let list_stdout = String::from_utf8_lossy(&list.stdout);

    assert!(ls.status.success());
    assert!(list.status.success());
    for stdout in [&ls_stdout, &list_stdout] {
        assert!(stdout.contains("List all MicroVMs with state and configured capacities"));
        assert!(stdout.contains("--non-interactive"));
    }
}

#[test]
fn ls_rejects_positional_name() {
    let output = run_microvm(&["ls", "web-01"]);
    assert!(!output.status.success());
}

#[test]
fn non_interactive_ls_without_root_reports_privilege_error() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["ls", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .env_remove("TAUMARU_ESCALATED")
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    assert!(!output.status.success());
    assert!(stderr.to_ascii_lowercase().contains("elevated rights"));
}

#[test]
fn prune_help_exposes_flag_only_surface() {
    let output = run_microvm(&["artifacts", "prune", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("--non-interactive"));
    assert!(!stdout.contains("--image"));
    assert!(!stdout.contains("--disk-gb"));
    assert!(!stdout.contains("--memory"));
    assert!(!stdout.contains("--vcpus"));
    assert!(!stdout.contains("--expose-lan"));
}

#[test]
fn prune_rejects_positional_name() {
    let output = run_microvm(&["artifacts", "prune", "web-01"]);
    assert!(!output.status.success());
}

#[test]
fn non_interactive_prune_without_root_reports_privilege_error() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["artifacts", "prune", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .env_remove("TAUMARU_ESCALATED")
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    assert!(!output.status.success());
    assert!(stderr.to_ascii_lowercase().contains("elevated rights"));
}

#[test]
fn delete_help_exposes_name_selection_without_lifecycle_flags() {
    let output = run_microvm(&["delete", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm delete"));
    assert!(stdout.contains("[NAME]"));
    assert!(stdout.contains("--name"));
    assert!(stdout.contains("--non-interactive"));
    assert!(!stdout.contains("--image"));
    assert!(!stdout.contains("--disk-gb"));
    assert!(!stdout.contains("--memory"));
    assert!(!stdout.contains("--vcpus"));
    assert!(!stdout.contains("--expose-lan"));
}

#[test]
fn non_interactive_delete_rejects_missing_name_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["delete", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("Missing machine name"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn non_interactive_delete_without_root_reports_privilege_error() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["delete", "web-01", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .env_remove("TAUMARU_ESCALATED")
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    assert!(!output.status.success());
    assert!(stderr.to_ascii_lowercase().contains("elevated rights"));
}

#[test]
fn autostart_help_lists_crud_subcommands_and_hides_the_boot_runner() {
    let output = run_microvm(&["autostart", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm autostart"));
    for subcommand in ["add", "edit", "rm", "ls"] {
        assert!(
            stdout.contains(&format!("  {subcommand}")),
            "missing {subcommand}"
        );
    }
    assert!(!stdout.contains("  run "));
}

#[test]
fn non_interactive_autostart_commands_require_a_name_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    for subcommand in ["add", "edit", "rm"] {
        let output = Command::new(env!("CARGO_BIN_EXE_microvm"))
            .args(["autostart", subcommand, "--non-interactive"])
            .env("TAUMARU_HOME", home.path())
            .output()
            .expect("failed to execute microvm");
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(output.status.code(), Some(1), "{subcommand}");
        assert!(stderr.contains("Missing machine name"), "{subcommand}");
        assert!(!stderr.contains("Choose a MicroVM"), "{subcommand}");
    }
}

#[test]
fn non_interactive_autostart_edit_requires_a_change() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["autostart", "edit", "web-01", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr.contains("Missing autostart change"));
}

#[test]
fn autostart_rejects_out_of_range_attempts_at_parse_time() {
    let output = run_microvm(&[
        "autostart",
        "add",
        "web-01",
        "--max-attempts",
        "0",
        "--non-interactive",
    ]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--max-attempts"));
}

#[test]
fn non_root_non_interactive_autostart_requires_elevated_rights() {
    if unsafe { libc_geteuid() } == 0 {
        return;
    }
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["autostart", "add", "web-01", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(1));
    assert!(stderr.contains("Elevated rights are required"));
}
