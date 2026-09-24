# Quickstart: Validate Unencrypted Snapshot and Restore

## Prerequisites

- Linux host with a compatible KVM and Firecracker setup.
- Rust workspace dependencies available.
- A managed MicroVM and enough free space for an archive and a separate restore home.
- A previous password-encrypted .tmvmsnap file for the compatibility check.
- An output directory whose system defaults and access policy can be inspected.

## Automated validation

Run the workspace quality gates after implementing the plan:

    cargo fmt --all -- --check
    cargo check --all-targets --all-features
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets --all-features

The SDK snapshot and restore tests must exercise the plain Zstandard/TAR stream, manifest version 3, no-password restore, legacy age rejection, corruption handling, cleanup, and output permissions. CLI command-surface tests must cover help, removed-option rejection, no secret echo, non-interactive behavior, and completion/progress wording.

## Manual flow

1. Confirm password flags are absent from help:

       cargo run -p taumaru-microvm-cli -- snapshot --help
       cargo run -p taumaru-microvm-cli -- restore --help

   Neither command should document --password.

2. Create a snapshot without a password. Supply the existing non-interactive snapshot inputs, including its address policy:

       cargo run -p taumaru-microvm-cli -- snapshot <VM_NAME> /tmp/<VM_NAME>.tmvmsnap --address-policy regenerate

   The command should finish without password input and state that the archive is unencrypted and contains the VM disk and SSH credentials.

3. Inspect the resulting file mode and owner using the host's file-inspection tools. The mode must follow the effective process umask and destination directory access policy. The current privilege flow may make the elevated process the file owner.

4. Restore the archive into a clean destination home without a password:

       TAUMARU_HOME=/tmp/taumaru-plain-restore cargo run -p taumaru-microvm-cli -- restore /tmp/<VM_NAME>.tmvmsnap --non-interactive

   Restore should report success, the VM should appear under its archived name, and its state should be stopped.

5. Attempt to restore a previous age-encrypted archive:

       cargo run -p taumaru-microvm-cli -- restore /tmp/legacy-encrypted.tmvmsnap --non-interactive

   The command should report that the old encrypted format is unsupported and recommend creating a new snapshot. It must not prompt for a password or create a VM.

6. Confirm a supplied removed option is rejected without echoing the supplied value:

       cargo run -p taumaru-microvm-cli -- snapshot <VM_NAME> /tmp/rejected.tmvmsnap --password sensitive-value --address-policy regenerate

   The command should fail at argument parsing, and neither output stream should contain sensitive-value.
