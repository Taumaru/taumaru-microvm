# Snapshot Validation Quickstart

## Prerequisites

- Linux host with loop devices and Device Mapper snapshot support.
- The SDK home contains a complete VM, root disk, cached guest kernel, SSH credentials, and local boot metadata.
- For online capture, the VM is running through its verified per-VM snapshot-origin mapping.
- The host exposes loop devices and the Device Mapper `snapshot` target (`dmsetup targets` lists it).
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

    rage --decrypt <NAME>.tmvmsnap | zstd --decompress --stdout | tar --list --file=-

Version 1 contains these TAR members in order:

    payload/rootfs.ext4
    payload/kernel/vmlinux
    payload/ssh/id_ed25519
    payload/ssh/id_ed25519.pub
    manifest.json

The manifest is last and records each payload's byte count and SHA-256 digest, along with portable VM, boot, network-intent, SSH, and compatibility metadata. It excludes host paths, database state, process and socket identity, loop and mapper names, and host network ownership details. The root disk and private SSH key members use mode 0600.

Confirm that a valid password lists all five members and that a wrong password or modified/truncated ciphertext is rejected by age authentication. Avoid extracting plaintext except into a private temporary directory, and delete any manual plaintext after validation.

## Validation record

Validation run for this implementation:

- PASS: `cargo fmt --all -- --check`
- PASS: `cargo check --all-targets --all-features`
- PASS: `cargo clippy --all-targets --all-features -- -D warnings`
- PASS: `cargo test --all-targets --all-features`
- SKIPPED: privileged live Device Mapper capture. `/dev/kvm` and `/dev/mapper/control` exist, but `dmsetup targets` returned `Permission denied` for the current UID (1000), and no active fixture MicroVM was available. Unit tests validate command sequencing and ownership checks, but do not replace a kernel-level live-write test. Run the manual online scenario as root on a host with loop and Device Mapper snapshot permissions.
