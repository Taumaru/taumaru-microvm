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
fn version_exits_successfully() {
    let output = run_microvm(&["--version"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("0.1.0"));
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
