# Implementation Plan: Portable MicroVM Restore

**Branch**: 021-vm-restore | **Date**: 2026-09-24 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification for snapshot address portability and SDK/CLI restore in [spec.md](spec.md).

## Summary

Extend the encrypted snapshot format with an explicit IPv4 address policy and implement the paired SDK restore operation and CLI command. With **preserve IPv4**, the archive records the guest address, prefix, gateway, optional LAN address, network mode, LAN exposure, and MAC; restore recreates those values and fails atomically if they cannot be used. With **regenerate IPv4**, the archive records only the policy and LAN exposure, removes source addressing from the Taumaru-managed guest network file in a private snapshot copy, and restore allocates destination-local IPv4/network identity and writes it into the recovered disk.

For a running VM, the existing Device Mapper snapshot remains the immutable block-level, crash-consistent point-in-time parent; it does not coordinate guest applications or pause the VM. The regenerate path creates an independently owned writable CoW child over that capture, replays committed ext4 journal transactions in the child, and removes only the managed network file there. A stopped VM uses a read-only loop view and the same private-child flow. The source root disk and running guest are never edited. The archive reader stages fixed members, validates manifest and payload integrity plus authenticated EOF, then restores files, destination network resources, and inventory as one recoverable operation. The restored VM is discoverable only after complete publication and remains stopped.

## Technical Context

**Language/Version**: Rust, workspace edition and toolchain.

**Primary Dependencies**: Existing SDK dependencies: age, tar, zstd, serde/serde_json, sha2, rusqlite, tokio, tokio-util, and fs4. Existing host tools include Device Mapper/loop control and ext4 tooling used by the current storage adapter. Any required host command is capability-checked and reported as a typed error. CLI uses existing clap, inquire, indicatif, and SDK dependencies.

**Storage**: SQLite local inventory; VM files under SDK home at `vms/<name>`; reusable kernels under `artifacts/kernels/<kernel-id>/<filename>`; private operation staging under `tmp`. Snapshot and restore journals track only operation-owned host state.

**Testing**: Plan SDK unit and public-contract tests for both address policies, archive validation and authentication, ext4 sanitization, database transaction, restore recovery, and failure cleanup. Plan CLI tests for interactive policy warning, non-interactive policy requirement, password handling, progress, and restore output. Plan a privileged Linux integration check proving that the supported classic Device Mapper child snapshot can read a read-only capture, accept private writes, preserve its parent and origin, and clean up without affecting another VM.

**Target Platform**: Linux hosts supported by the existing SDK and Firecracker runtime, with compatible architecture, KVM/runtime prerequisites, writable SDK storage, Device Mapper, loop control, and ext4 tools required by the selected snapshot policy.

**Performance Goals**: Stream disk and archive data using bounded buffers. Do not pause a running VM for the backup copy. Report progress by actual payload bytes and distinct preparation/sanitization stages. Monitor parent and child CoW validity/overflow through archive completion. Avoid a second full-disk copy when the writable child mapping is supported; a private regular-file copy remains the fallback if nested classic snapshots fail a verified capability check.

**Constraints**: Manifest version 2 only; reject v1 and unsupported versions. IPv4 only. Source database rows, host interfaces, host-side addresses, routes, leases, ownership records, absolute paths, process IDs, sockets, loop/Device Mapper names, and execution binaries are not portable. The SDK remains silent and returns typed errors. Restore never starts the VM.

**Scale/Scope**: Multiple VMs and concurrent operations are first-class. Serialize operations targeting one VM name/volume. Give each temporary loop and Device Mapper layer an identity derived from the VM identity plus an operation identity, so cleanup and discovery do not depend on persistent database state.

## Constitution Check

*Gate before research and design: PASS.*

- **I. SDK-First Shared Core**: Snapshot and restore orchestration belongs in the SDK; the CLI only gathers choices and presents progress/results.
- **II. Panic-Free, Silent SDK Boundary**: Invalid policy, malformed archives, conflicts, I/O, host-tool, network, and database failures return typed errors. The SDK does not prompt, print, or log.
- **III. Explicit Local State and Lifecycle**: Publish a complete stopped VM after payloads and destination resources are ready. Reconcile interrupted work from a durable journal.
- **IV. Closed for Modification, Open for Extension**: Keep archive, storage sanitization, runtime snapshot, network allocation, and repository persistence behind focused adapters and ports.
- **V. Calm CLI and English Artifacts**: Follow existing prompts, progress, diagnostics, and deterministic non-interactive behavior; keep repository artifacts in English.

*Gate after design: PASS.* The design adds explicit snapshot policy, a private disk transformation path, and a journaled restore flow without moving lifecycle logic into the CLI.

## Project Structure

**Documentation**: Keep all feature artifacts in `specs/021-vm-restore/`: this plan, `research.md`, `data-model.md`, `quickstart.md`, existing restore contracts, new snapshot SDK/CLI contracts, and the requirements checklist. Do not edit another feature directory.

**SDK changes**:
- `crates/sdk/src/domain/snapshot.rs`: address policy and sanitization progress stages; expose a typed policy on snapshot requests.
- `crates/sdk/src/domain/restore.rs`: restore request/result/progress/stage and policy-aware manifest types; export via `domain/mod.rs` and `lib.rs`.
- `crates/sdk/src/manager.rs`: snapshot orchestration for preserve/regenerate and restore orchestration with per-name locking and durable journal.
- `crates/sdk/src/adapters/runtime/device_mapper.rs`: private child snapshot mapping, independent COW loop/file ownership, validity/overflow checks, and dependency-ordered cleanup.
- `crates/sdk/src/adapters/storage/guest_fs.rs` and `crates/sdk/src/ports/storage.rs`: narrow operation to remove the fixed Taumaru-managed network unit from a private ext4 view and a typed writer for the final destination IPv4 configuration.
- `crates/sdk/src/adapters/archive/age_tar_zstd.rs` and a restore reader alongside it: version 2 policy manifest, encrypted stream reading, staging, integrity, and authenticated EOF.
- `crates/sdk/src/ports/network.rs` and `crates/sdk/src/adapters/network/linux.rs`: exact-address restore input and conflict validation for preserve policy; ordinary destination-local allocation for regenerate policy.
- `crates/sdk/src/ports/repository.rs` and `crates/sdk/src/adapters/persistence/sqlite.rs`: one transaction to publish restored VM, network, credential, stopped runtime, and imported kernel inventory.
- SDK tests under `crates/sdk/src` and `crates/sdk/tests` for policy validation, CoW isolation, archive contract, network conflict, atomic persistence, cancellation, and journal recovery.

**CLI changes**:
- `crates/cli/src/commands/snapshot.rs`: ask whether to include source IPv4 settings, explain restore conflict risk, pass an explicit policy to the SDK, and require a policy in non-interactive use.
- `crates/cli/src/cli.rs`: add a deterministic explicit snapshot-policy argument using the existing CLI conventions; settle its exact spelling while implementing the command surface.
- `crates/cli/src/commands/restore.rs` and `commands/mod.rs`: add restore path/password prompts and SDK call.
- `crates/cli/src/output/human.rs`: render policy, sanitization, snapshot/restore progress, success, and typed failures using existing output patterns.
- CLI tests under `crates/cli/tests` for the interactive and scripted flows.

**Structure Decision**: Keep policy and archive validation in SDK domain/adapters. Use the existing online read-only Device Mapper capture as the immutable parent. Apply network-file sanitization only to a separate writable child CoW view (or an isolated full-copy fallback after capability detection). Restore uses a fixed-member staging reader, destination-specific network allocation, guest config rewrite, a compound SQLite commit, and a durable journal for cross-resource recovery.

## Complexity Tracking

No constitution violations. The feature coordinates an encrypted archive, a writable child CoW layer, offline ext4 journal/config handling, destination network resources, kernel cache inventory, and SQLite/filesystem state. The design bounds that complexity through explicit policy variants, owned operation identities, narrow storage operations, transactional database publication, and a durable cleanup journal. The nested classic snapshot capability check and its full-copy fallback are explicit portability work, not an assumption that every kernel accepts arbitrary snapshot depth.
