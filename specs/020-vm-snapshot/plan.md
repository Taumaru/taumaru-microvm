# Implementation Plan: Online Encrypted MicroVM Snapshot

**Branch**: 020-vm-snapshot | **Date**: 2026-09-23 | **Spec**: specs/020-vm-snapshot/spec.md

**Input**: Feature specification from specs/020-vm-snapshot/spec.md

## Summary

Add one SDK operation and one CLI command that export a MicroVM as a password-encrypted, portable archive. For a running VM, the SDK will use the already active per-VM Device Mapper snapshot-origin path, briefly suspend that mapping to add a classic Device Mapper snapshot target, resume it, and stream the immutable snapshot view into an encrypted archive while Firecracker continues running. The source VM remains live; its disk I/O can wait briefly while the capture boundary is installed. A stopped VM can be copied directly after the SDK confirms there is no active writer.

The archive contains the root disk, the exact cached guest kernel, a versioned manifest, portable VM and boot metadata, and the associated SSH key pair. It excludes Firecracker and firectl executables, source-host database contents, absolute paths, process identifiers, and host-owned network resources. The output is a TAR stream compressed with Zstandard and encrypted with the age passphrase format. Temporary snapshot resources are per-VM, verified by deterministic identities, and removed after export. The final file is published atomically without replacing an existing destination.

## Technical Context

**Language/Version**: Rust 2024 workspace.

**Primary Dependencies**: Existing Tokio, SQLite, serde, SHA-256, and host-command abstractions; add the age Rust crate for streaming passphrase encryption, tar for archive framing, and zstd for streaming compression. Keep Firecracker, losetup, and dmsetup operations behind existing SDK runtime boundaries.

**Storage**: Existing SDK home and SQLite inventory remain authoritative for local VM data. Add a read-only repository query for stored distribution boot metadata; no schema migration is needed. Place the temporary COW file in the managed SDK home/tmp directory with owner-only permissions. Write only encrypted archive bytes to a mode-0600 temporary file beside the requested destination.

**Testing**: SDK unit and contract tests for manifest construction, streaming, typed failures, cancellation, and atomic publication; CLI command-surface and prompt/escalation tests; privileged Linux integration coverage for loop and Device Mapper snapshot behavior. The project quality gates remain cargo fmt, cargo check, cargo clippy, and cargo test. These checks are planned for implementation; they are not run during planning.

**Target Platform**: Linux with the Device Mapper snapshot targets, loop devices, dmsetup, losetup, and the permissions required to manage them. Online capture requires the existing VM runtime mapping to be active and verifiable.

**Project Type**: Rust SDK library with a thin Rust CLI.

**Performance Goals**: Stream the disk with a bounded buffer and no second plaintext archive. Compression and encryption run as a pipeline. The VM process and vCPUs stay running; a brief block-I/O delay is expected while the snapshot boundary is installed. No fixed export throughput or completion-time SLA is introduced.

**Constraints**: The snapshot is block-level and crash-consistent, not application-quiesced and not a memory checkpoint. The COW device can fill during high write activity; status and read failures must invalidate the export, and no incomplete archive is published. The persistent COW store needs capacity for changed disk chunks plus Device Mapper metadata. Temporary COW data is plaintext while in use, so its path and file must be private to the SDK owner and removed after detaching its loop and mapper resources. The output must not overwrite an existing file. Passwords and archive contents must not be printed or logged by the SDK or CLI.

**Scale/Scope**: One root disk and one guest kernel per VM. A VM's lifecycle and snapshot work are serialized using its existing lifecycle lock; different VMs may be captured independently. A snapshot is a portable recovery archive, not a restore/import implementation and not a guest-memory image.

## Constitution Check

### Before Phase 0

- **SDK-first shared core: PASS.** Snapshot orchestration and archive creation belong in the SDK. The CLI handles selection, password input, privilege handoff, and presentation.
- **Panic-free, silent SDK boundary: PASS.** Public operations return typed errors and result data. The SDK does not prompt, print, log, or terminate the host process.
- **Explicit local state and lifecycle: PASS.** Read VM identity and configuration from the existing repository, verify live process state, and coordinate with lifecycle operations using the existing per-VM lock. No single-VM assumption is introduced.
- **Closed for modification, open for extension: PASS.** Add an internal archive adapter and extend the runtime-disk and repository boundaries instead of placing host mechanics in the CLI.
- **CLI and documentation language: PASS.** New repository text and user-facing product text will be in English.
- **Host data directory: PASS.** Temporary COW storage uses the managed home/tmp directory with restrictive permissions and deterministic ownership evidence.

### After Phase 1 Design

- **Public API compatibility: PASS.** Add documented SDK types and methods; preserve existing start, stop, rootfs, and VM result contracts.
- **Resource ownership and multiple VMs: PASS.** Device Mapper names and UUIDs derive from the canonical SDK home plus VM identity; loop devices are verified against their exact backing paths. Per-VM locks prevent overlapping snapshot/lifecycle cleanup.
- **Failure safety: PASS.** Do not modify the persistent rootfs. Keep the existing origin mapping in service, validate snapshot status before publication, remove only verified owned resources, and publish the completed encrypted file atomically without replacement.
- **Data and artifact boundaries: PASS.** Snapshot metadata comes from local persisted inventory and verified cached artifacts; it does not require the registry or export the full database.
- **Security and CLI ownership: PASS.** Use age passphrase encryption, owner-only temporary files, hidden interactive input, and no secret-bearing diagnostics. The explicit password flag remains available as specified; command-line arguments may be visible to the local shell/process environment, so the hidden prompt is the safer interactive path.

## Project Structure

### Documentation

    specs/020-vm-snapshot/
    ├── plan.md
    ├── research.md
    ├── data-model.md
    ├── quickstart.md
    └── contracts/
        ├── sdk-snapshot.md
        └── cli-snapshot.md

The tasks.md file is intentionally left for the separate speckit-tasks phase.

### Source and tests

    crates/sdk/src/
    ├── lib.rs
    ├── manager.rs
    ├── error.rs
    ├── domain/
    │   └── snapshot.rs
    ├── ports/
    │   ├── repository.rs
    │   └── runtime_disk.rs
    └── adapters/
        ├── archive/
        │   └── age_tar_zstd.rs
        ├── persistence/
        │   └── sqlite.rs
        └── runtime/
            └── device_mapper.rs

    crates/sdk/tests/
    └── snapshot.rs

    crates/cli/src/
    ├── cli.rs
    ├── privilege.rs
    └── commands/
        ├── mod.rs
        └── snapshot.rs

    crates/cli/tests/
    └── command_surface.rs

**Structure Decision**: Extend the existing two-crate workspace. The manager coordinates the snapshot use case; the runtime-disk port owns snapshot-view setup and cleanup; the SQLite adapter exposes the additional local boot metadata; and a private SDK archive adapter owns TAR, Zstandard, hashing, and age streaming. The CLI remains a thin command module and uses the existing privilege and prompt conventions, with a graceful cancellation path for this long-running operation.

## Complexity Tracking

No constitution violations or additional architectural layers are proposed.
