use std::process::{Command, Output};

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
fn download_help_exposes_repeatable_selection_options() {
    let output = run_microvm(&["download", "--help"]);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("Usage: microvm download"));
    assert!(stdout.contains("--distribution <DISTRIBUTION_ID>"));
    assert!(stdout.contains("--kernel <DISTRIBUTION_ID=KERNEL_ID>"));
    assert!(stdout.contains("--non-interactive"));
}

#[test]
fn non_interactive_download_rejects_incomplete_selection_without_prompting() {
    let home = tempfile::tempdir().expect("temporary home should be created");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_microvm"))
        .args(["download", "--non-interactive"])
        .env("TAUMARU_HOME", home.path())
        .output()
        .expect("failed to execute microvm");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("selection") || stderr.contains("plan"));
    assert!(!stderr.contains("Select distributions"));
}
