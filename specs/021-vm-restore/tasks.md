---
description: "Dependency-ordered implementation tasks for portable MicroVM snapshots and restore"
---

# Tasks: Portable MicroVM Restore

**Input**: Design documents from `specs/021-vm-restore/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, and `contracts/`

**Tests**: Included because the specification defines independent test criteria for each user story and the project constitution requires relevant unit, integration, contract, and failure-path coverage. Test tasks are listed before implementation in each story and should demonstrate the expected failure before their implementation tasks begin.

**Organization**: Tasks are grouped by the four user stories in `spec.md`. All stories have priority P1. The shared v2 manifest foundation comes first; restore safety and CLI follow; the snapshot producer is implemented after restore so its round-trip tests can exercise the completed restore contract.

## Phase 1: Setup

**Purpose**: Project initialization.

No project setup task is required. The SDK/CLI workspace and dependencies already exist; this feature extends the current crates and adapters without adding a new crate or dependency.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Establish shared archive policy, manifest, and typed errors before implementing producer or consumer behavior.

- [ ] T001 Add the public `SnapshotAddressPolicy` type with the required typed values for preserving source IPv4 or regenerating destination IPv4, document it with Rustdoc, and re-export it in `crates/sdk/src/domain/snapshot.rs`, `crates/sdk/src/domain/mod.rs`, and `crates/sdk/src/lib.rs`.
- [ ] T002 Define the version 2 manifest and network-policy variants in `crates/sdk/src/adapters/archive/manifest.rs` and `crates/sdk/src/adapters/archive/mod.rs`; preserve requires guest IPv4, prefix, optional gateway, optional LAN IPv4, mode, LAN exposure, and guest MAC, while regenerate stores only the policy and LAN exposure; include kernel ID, name, display name, version, architecture, registry path and URL, filename, format, MIME type, and modified time, with payload byte count/SHA-256 authoritative; reject version 1, unknown versions, IPv6, and inconsistent fields.
- [ ] T003 Add typed SDK error variants and Rustdoc for unsupported archive versions/policies, invalid or conflicting network values, sanitization and snapshot capability failures, CoW overflow, and restore recovery failures in `crates/sdk/src/error.rs`.

**Checkpoint**: Shared policy and manifest types are available to snapshot and restore work; SDK failures have typed representations.

---

## Phase 3: User Story 1 — Restore a MicroVM from an archive (Priority: P1)

**Goal**: Restore the archived disk, kernel, credentials, configuration, and selected network policy into a discoverable local VM that remains stopped.

**Independent Test**: Restore valid encrypted version 2 fixtures for both address policies into a clean SDK home. Verify disk/kernel/key contents, manifest metadata, destination-local paths, guest network settings, and stopped inventory state.

### Tests for User Story 1

- [ ] T004 [P] [US1] Add archive-reader contract tests for valid preserve and regenerate version 2 archives, fixed members, payload sizes/digests, kernel metadata, and authenticated EOF in `crates/sdk/src/adapters/archive/restore.rs`.
- [ ] T005 [P] [US1] Add an SDK restore integration test using encrypted version 2 fixtures to verify restored root disk, exact embedded kernel registration, SSH public/private keys and username/port/type/fingerprint metadata, portable VM settings, both network policy outcomes, destination paths, stopped state, and resolution through the normal start path in `crates/sdk/tests/restore.rs`.

### Implementation for User Story 1

- [ ] T006 [US1] Add documented `RestoreRequest`, `RestoreResult`, progress, and lifecycle-stage types, then export them through `crates/sdk/src/domain/restore.rs`, `crates/sdk/src/domain/mod.rs`, and `crates/sdk/src/lib.rs`; the request accepts an archive path and one non-empty password.
- [ ] T007 [P] [US1] Implement the age → Zstandard → TAR restore reader with a fixed member allowlist, SDK-selected staging paths, reject duplicate/missing/extra/unsafe members and non-regular entries, validate archived VM names and safe single-component kernel filenames, verify sizes and SHA-256 values, validate SSH key/fingerprint correspondence, bound archive/member sizes and accepted age scrypt work factor, and check authenticated EOF in `crates/sdk/src/adapters/archive/restore.rs` and `crates/sdk/src/adapters/archive/mod.rs`.
- [ ] T008 [P] [US1] Extend `GuestStorage` with a typed writer for the final IPv4 guest configuration, including exact prefix, optional guest-visible gateway, optional LAN address, and route, in `crates/sdk/src/ports/storage.rs`, `crates/sdk/src/adapters/storage/ext4.rs`, and `crates/sdk/src/adapters/storage/guest_fs.rs`.
- [ ] T009 [P] [US1] Extend the internal network request to accept exact archived guest IPv4/prefix/gateway, optional LAN IPv4, mode, exposure, and MAC for preserve policy, while using normal destination allocation from LAN exposure for regenerate policy, in `crates/sdk/src/ports/network.rs` and `crates/sdk/src/adapters/network/linux.rs`.
- [ ] T010 [P] [US1] Add one repository operation that atomically persists the restored VM, destination network record, credential, stopped runtime, and verified kernel inventory metadata in `crates/sdk/src/ports/repository.rs` and `crates/sdk/src/adapters/persistence/sqlite.rs`.
- [ ] T011 [US1] Orchestrate restore in `crates/sdk/src/manager.rs`: validate destination prerequisites, stage and verify all archive members, resolve runtime locally, install files without replacement at the documented modes (root disk/private key 0600; kernel/public key 0644), configure destination networking, write the guest network file, register the exact embedded kernel, and publish the VM only after every required record is ready.
- [ ] T012 [US1] Expose the documented SDK restore operation and any progress variant through `crates/sdk/src/manager.rs` and `crates/sdk/src/lib.rs`; guarantee that success returns the archived VM identity and stopped state without launching Firecracker.

**Checkpoint**: SDK callers can restore a valid version 2 archive into a complete stopped VM without source-host database or runtime files.

---

## Phase 4: User Story 2 — Reject conflicts and invalid archives safely (Priority: P1)

**Goal**: Reject untrusted archives, duplicate names, unavailable exact network values, and interrupted operations without altering existing VMs or publishing partial state.

**Independent Test**: Exercise duplicate-name/path races, wrong password, malformed/truncated archives, unsupported versions, unsafe or mismatched members, IPv6, exact IP/MAC conflicts, cancellation, and simulated restart recovery. Verify typed errors, no partial records/files, and preservation of pre-existing VMs.

### Tests for User Story 2

- [ ] T013 [P] [US2] Add failure-path tests for wrong password, v1/unknown version, missing or inconsistent policy fields, IPv6, unsafe/duplicate/extra archive members, corrupt hashes, truncation, and authenticated-stream failure in `crates/sdk/tests/restore_archive_failures.rs`.
- [ ] T014 [P] [US2] Add conflict tests for an existing VM name, occupied managed path, exact guest/LAN IPv4 or MAC conflict, insufficient storage, and unavailable destination runtime; assert no pre-existing state changes in `crates/sdk/tests/restore_conflicts.rs`.
- [ ] T015 [P] [US2] Add cancellation and interrupted-restore tests that restart the manager, reconcile journaled operation-owned resources, and allow a safe retry without deleting pre-existing data in `crates/sdk/tests/restore_recovery.rs`.

### Implementation for User Story 2

- [ ] T016 [P] [US2] Serialize concurrent restores by archived VM name and enforce no-clobber checks for inventory and destination paths before installation in `crates/sdk/src/manager.rs`.
- [ ] T017 [P] [US2] Validate preserve-policy guest/LAN IPv4 and MAC against persisted and live destination allocations, and roll back only network resources created by the failed restore in `crates/sdk/src/adapters/network/linux.rs`.
- [ ] T018 [US2] Add durable restore-journal persistence for operation identity, affected VM, staging/final paths, imported kernel entries, network resources, and progress state in `crates/sdk/src/ports/repository.rs` and `crates/sdk/src/adapters/persistence/sqlite.rs`.
- [ ] T019 [US2] Reconcile incomplete journal entries before retrying the affected VM name and remove only resources recorded as owned by the interrupted operation in `crates/sdk/src/manager.rs`.
- [ ] T020 [US2] Complete rollback for errors and cancellation across staged/installed files, kernel cache entries, credentials, and destination network resources; enforce private SSH key mode `0600` and prevent publication of partial inventory in `crates/sdk/src/manager.rs` and `crates/sdk/src/adapters/archive/restore.rs`.

**Checkpoint**: Invalid or conflicting restores return typed errors and leave existing machines unchanged; interrupted operations can be reconciled safely.

---

## Phase 5: User Story 3 — Restore interactively or from a script (Priority: P1)

**Goal**: Expose restore through a CLI that prompts only for missing inputs, hides the password, reports progress, and delegates lifecycle behavior to the SDK.

**Independent Test**: Exercise `microvm restore`, `microvm restore <ARCHIVE_PATH>`, and `microvm restore <ARCHIVE_PATH> --password <PASSWORD>`; verify exactly one hidden password prompt, no confirmation, immediate non-interactive errors for missing inputs, progress, and stopped-state output.

### Tests for User Story 3

- [ ] T021 [P] [US3] Add CLI command-surface tests for missing/present archive paths, one masked password prompt without confirmation, supplied password not echoed, non-interactive validation, progress, and successful stopped-state output in `crates/cli/tests/command_surface.rs`.

### Implementation for User Story 3

- [ ] T022 [US3] Add the `restore` subcommand with an optional archive path and `--password` argument, and register dispatch through `crates/cli/src/cli.rs` and `crates/cli/src/commands/mod.rs`.
- [ ] T023 [P] [US3] Implement `crates/cli/src/commands/restore.rs` to prompt for a missing path, request a missing password once through the existing masked `inquire` password input, reject missing non-interactive values, call the SDK, and forward progress without printing the password.
- [ ] T024 [P] [US3] Render restore progress, archived VM name, destination-local identity, stopped state, and calm cause/impact/next-step diagnostics through `crates/cli/src/output/human.rs`.

**Checkpoint**: Interactive and scripted restore use the same SDK operation and never expose the password.

---

## Phase 6: User Story 4 — Choose address portability when creating a snapshot (Priority: P1)

**Goal**: Let snapshot callers choose exact IPv4 preservation or address regeneration, produce a version 2 archive with the matching metadata, and sanitize only a private disk copy for the regenerate policy.

**Independent Test**: Create and restore snapshots with each policy. Verify preserve conflicts fail safely; regenerate manifests omit source network assignments, the archived Taumaru-managed network file has no source addresses, destination restore writes newly allocated guest settings, the source disk is unchanged, and guest applications were never paused.

### Tests for User Story 4

- [ ] T025 [P] [US4] Add writer/manifest tests for version 2 kernel metadata and both policy shapes; preserve includes guest IPv4, prefix, optional gateway/LAN address, mode, exposure, and MAC, while regenerate stores only the policy and LAN exposure and omits all source assignment fields, in `crates/sdk/src/adapters/archive/age_tar_zstd.rs`.
- [ ] T026 [P] [US4] Add a privileged Linux integration test proving a writable classic Device Mapper child over the read-only capture accepts private writes without changing its parent/origin, detects overflow, and cleans up in dependency order in `crates/sdk/tests/snapshot_cow.rs`.
- [ ] T027 [P] [US4] Add ext4 image tests that sanitize only `/etc/systemd/network/10-taumaru.network`, preserve unrelated guest files, and leave the source image unchanged in `crates/sdk/tests/snapshot_sanitization.rs`.
- [ ] T028 [P] [US4] Add CLI tests for the yes/no IPv4 prompt, the preserve-conflict warning, and failure when non-interactive snapshot creation omits the policy in `crates/cli/tests/command_surface.rs`.

### Implementation for User Story 4

- [ ] T029 [P] [US4] Add the independently named writable child snapshot, COW loop/file ownership, status/overflow checks, and dependency-ordered cleanup to `crates/sdk/src/adapters/runtime/device_mapper.rs`; add a verified nested-snapshot capability probe and use the full-copy fallback only when the probe reports unsupported.
- [ ] T030 [P] [US4] Extend `GuestStorage` with a fixed-path private-view preparation operation that replays committed ext4 journal transactions and removes only the Taumaru-managed network file; reject missing/non-regular files and never accept an arbitrary guest path in `crates/sdk/src/ports/storage.rs`, `crates/sdk/src/adapters/storage/ext4.rs`, and `crates/sdk/src/adapters/storage/guest_fs.rs`.
- [ ] T031 [US4] Add an exact-length private root-disk copy fallback that checks available storage, uses unique restrictive temporary files, reports copied bytes, does not resize the image, and cleans up on failure in `crates/sdk/src/ports/storage.rs` and `crates/sdk/src/adapters/storage/ext4.rs`.
- [ ] T032 [P] [US4] Update the encrypted archive writer to emit version 2, serialize policy-specific network fields and kernel ID, name, display name, version, architecture, registry path and URL, filename, format, MIME type, and modified time; keep payload byte count/SHA-256 authoritative and hash the actual root-disk bytes after any sanitization in `crates/sdk/src/adapters/archive/age_tar_zstd.rs`.
- [ ] T033 [US4] Update snapshot orchestration to require an explicit SDK policy, capture a running VM through the stable read-only parent, use the private child or verified full-copy fallback for regenerate, keep stopped-source reads serialized, report preparation/sanitization/archive progress, clean up before atomic output publication, and never mutate the source disk in `crates/sdk/src/manager.rs` and `crates/sdk/src/domain/snapshot.rs`.
- [ ] T034 [P] [US4] Add the CLI `--address-policy <preserve|regenerate>` option, interactive choice with the restore-conflict warning, and deterministic non-interactive failure when the option is absent in `crates/cli/src/cli.rs` and `crates/cli/src/commands/snapshot.rs`.
- [ ] T035 [US4] Render snapshot policy, private-copy sanitization, real payload byte progress, completion, and actionable failures through `crates/cli/src/output/human.rs`.

**Checkpoint**: The SDK can produce both policy variants; restore round-trips either variant and configures the guest disk according to the destination policy.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Complete documentation and validate the full implementation against project quality gates and Linux snapshot behavior.

- [ ] T036 [P] Update `specs/021-vm-restore/quickstart.md` with the implemented policy flag, interactive prompts, restore commands, crash-consistency boundary, host-tool requirements, and full-copy fallback behavior.
- [ ] T037 [P] Complete public Rustdoc and compatibility notes for the new snapshot policy and restore request/result/error surfaces in `crates/sdk/src/lib.rs`, `crates/sdk/src/domain/snapshot.rs`, and `crates/sdk/src/domain/restore.rs`.
- [ ] T038 Run `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` from the workspace root `Cargo.toml`.
- [ ] T039 Run the privileged nested-snapshot integration test on each supported kernel/filesystem configuration and verify the fallback path on a host where classic nested snapshots are unavailable using `crates/sdk/tests/snapshot_cow.rs` and `crates/sdk/tests/snapshot_sanitization.rs`.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No changes required; the existing workspace is ready.
- **Foundational (Phase 2)**: Must complete before all user stories.
- **User Story 1 (Phase 3)**: Depends on shared version 2 manifest types; valid archives can be supplied by fixtures while the producer remains version 1.
- **User Story 2 (Phase 4)**: Depends on the restore operation from US1.
- **User Story 3 (Phase 5)**: Depends on the public restore API from US1.
- **User Story 4 (Phase 6)**: Depends on the shared policy model and US1 restore API for end-to-end round-trip acceptance. US4 completes the producer side of the paired format.
- **Polish (Phase 7)**: Depends on all four stories.

### User Story Dependencies

- **US1 (P1)**: Can start after Phase 2; independent tests use authenticated version 2 fixtures.
- **US2 (P1)**: Requires US1 restore orchestration so failure and recovery tests exercise the real path.
- **US3 (P1)**: Requires the US1 public SDK operation; CLI restore presentation can proceed after that API stabilizes.
- **US4 (P1)**: Requires Phase 2 policy types and US1 restore for a full snapshot → restore round trip. Snapshot manifest and CoW work can be implemented against the shared contract.

### Parallel Opportunities

- In **US1**, T004 and T005 can be written in parallel; after T006, T007, T008, T009, and T010 can proceed in parallel, followed by manager integration T011 and facade completion T012.
- In **US2**, T013, T014, and T015 can be written in parallel; T016, T017, and T018 work in separate files, followed by journal reconciliation and cleanup.
- In **US3**, the command-surface tests T021 can be drafted before T022–T024; T023 and T024 can proceed in parallel after the argument/result types are fixed.
- In **US4**, T025–T028 can be drafted in parallel; implement the Device Mapper child (T029), storage sanitizer (T030), writer (T032), and CLI policy input (T034) in parallel. T031 uses the same storage files as T030 and follows it. Integrate the snapshot manager in T033, then finish progress rendering T035.

## Parallel Execution Examples

### User Story 1

- Start T004 and T005 together.
- After T006, implement archive reading (T007), guest network writing (T008), network request support (T009), and repository publication (T010) in parallel.
- Integrate the restore flow in T011, then expose the public API in T012.

### User Story 2

- Start T013, T014, and T015 together.
- After tests are in place, implement name/path locking (T016), network conflict rollback (T017), and journal persistence (T018) in parallel.
- Complete manager recovery (T019) and failure cleanup (T020) afterward.

### User Story 3

- Draft T021 first. After the public restore API is stable, add CLI parsing (T022), then implement command behavior (T023) and output rendering (T024) in parallel.

### User Story 4

- Start T025–T028 together.
- Implement the Device Mapper child (T029), ext4 preparation/sanitization (T030), private copy fallback (T031), and CLI policy input (T034) in parallel.
- Integrate the writer (T032) and SDK snapshot pipeline (T033), then finish human progress output (T035).

---

## Implementation Strategy

### MVP

Deliver **US1 + US2** first: restore a supported version 2 archive and prove that invalid input, conflicts, cancellation, and interruption cannot damage existing state. The producer remains a separate increment, so initial SDK acceptance uses authenticated version 2 fixtures. For a complete local snapshot-and-restore workflow, add **US4** before calling the feature end-to-end. **US3** adds the guided and scripted restore CLI.

### Incremental Delivery

1. Complete the shared policy/manifest foundation.
2. Deliver restore success (US1) and safe rejection/recovery (US2).
3. Add interactive and non-interactive restore (US3).
4. Add snapshot address choice, private sanitization, and round-trip support (US4).
5. Run the full quality gates and privileged Device Mapper integration checks.

### Notes

- Every task uses the required checkbox/ID format; [P] marks tasks that can proceed in parallel without editing the same file or waiting on unfinished work.
- User-story labels map to the four stories in `spec.md`.
- SDK tests assert typed errors and no unsolicited output; CLI tests cover prompts, progress, and diagnostics.
- The snapshot remains block-level crash-consistent; it does not quiesce guest applications or coordinate database transactions.
