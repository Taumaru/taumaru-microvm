# Snapshot Validation Quickstart

## Prerequisites

- Linux host with loop devices and Device Mapper snapshot support.
- The SDK home contains a complete VM, root disk, cached guest kernel, SSH credentials, and local boot metadata.
- For online capture, the VM is running through its verified per-VM snapshot-origin mapping.
- The caller has permission to manage loop and Device Mapper resources.
- Enough free space exists for the temporary COW file and encrypted output.
- rage, zstd, and GNU tar are available for inspecting a generated archive in manual validation.

## Build and automated validation

From the repository root, run the relevant workspace gates after implementation:

    cargo fmt --all -- --check
    cargo check --all-targets --all-features
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-targets --all-features

The SDK tests should cover both stopped and running source modes, deterministic per-VM resource ownership, output collision, wrong password/tampered/truncated output, COW Invalid or Overflow state, cancellation, cleanup failures, and failure without final-file publication. A privileged Linux integration case should write distinct blocks before and after capture, export the snapshot while the VM keeps running, and verify that the decrypted root disk contains the pre-boundary contents while the live VM contains the later writes. Exercise two VMs to confirm separate COW and mapper identities.

## Manual CLI scenarios

From a terminal:

    microvm snapshot

Choose a VM and enter the password at the hidden prompt. The default output should be ./<NAME>.tmvmsnap and the VM should remain running if it started running.

Select a named VM and default output:

    microvm snapshot <NAME>

Choose an explicit output path:

    microvm snapshot <NAME> /path/to/recovery.tmvmsnap

For script use, specify a password explicitly and choose a non-existing path:

    microvm snapshot <NAME> /path/to/recovery.tmvmsnap --password <PASSWORD>

Verify that an existing destination is refused and is not changed. Verify that invocation without a VM name or password in a non-interactive context exits with an actionable error.

## Inspect the encrypted stream

After successful creation, decrypt and inspect the TAR listing without extracting payloads:

    rage --decrypt <NAME>.tmvmsnap | zstd --decompress --stdout | tar --list

Then extract or stream the archive to a private temporary directory for manual checks. Confirm that the manifest is last, includes the supported version, and lists hashes and sizes that match the payloads. A wrong password or modified/truncated ciphertext must be rejected by age authentication. Delete any manual plaintext extraction after validation.
