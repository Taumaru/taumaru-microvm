---
description: "Task list for online encrypted MicroVM snapshots"
---

# Tasks: Online Encrypted MicroVM Snapshot

**Input**: Design documents from specs/020-vm-snapshot/

**Prerequisites**: plan.md and spec.md; research.md, data-model.md, contracts/, and quickstart.md.

**Tests**: Included because the feature specification defines independent tests, acceptance scenarios, and measurable test outcomes for all three user stories.

**Organization**: Tasks are grouped by user story. US2 and US3 both depend on the SDK capture operation from US1, then may proceed in parallel because the CLI and archive portability work have separate implementation boundaries.

## Format: [ID] [P?] [Story] Description

- [P] means the task can proceed in parallel with other marked tasks because it edits separate files and has no unfinished dependency.
- [Story] labels map to user stories in specs/020-vm-snapshot/spec.md.
- Every task description names its target source, test, or documentation file.

## Phase 1: Setup

**Purpose**: Add the archive libraries needed by the snapshot implementation.

- [ ] T001 Add age, tar, and zstd workspace dependency declarations in Cargo.toml and enable them for the SDK in crates/sdk/Cargo.toml.

---

## Phase 2: Foundational

**Purpose**: Establish the public SDK result/cancellation contract and typed failure surface required by all stories.

**Checkpoint**: User story work can begin after these shared SDK types and errors are available.

- [ ] T002 [P] Define SnapshotResult and SnapshotCancellation with Rustdoc in crates/sdk/src/domain/snapshot.rs, register the module in crates/sdk/src/domain/mod.rs, and re-export the public types from crates/sdk/src/lib.rs.
- [ ] T003 [P] Add typed snapshot output-conflict, invalid-or-overflow-view, capacity, and cancellation errors in crates/sdk/src/error.rs without changing existing error behavior.

---

## Phase 3: User Story 1 - Capture a running VM (Priority: P1) 🎯 MVP

**Goal**: Create an encrypted point-in-time disk export while a running MicroVM stays running, and support a stopped VM without creating an online snapshot layer.

**Independent Test**: Start a VM, write distinct block contents while the snapshot is being streamed, and confirm the archive retains the contents visible at the capture boundary while the running VM contains later writes. Also confirm stopped-VM capture works and failure/cancellation leaves the rootfs and running process intact with no final archive.

### Tests for User Story 1

- [ ] T004 [P] [US1] Add Device Mapper adapter tests for deterministic per-VM snapshot names and UUIDs, verified origin/COW dependencies, suspend-load-resume and suspend-remove-resume sequences, overflow handling, and cleanup that never detaches the rootfs loop in crates/sdk/src/adapters/runtime/device_mapper.rs.
- [ ] T005 [P] [US1] Add SDK snapshot tests for running disk writes across the capture boundary, stopped-disk capture, per-VM lifecycle serialization, cancellation, typed failures, no stdout/stderr, and no final-file publication in crates/sdk/tests/snapshot.rs; gate the privileged Device Mapper case on Linux host prerequisites.

### Implementation for User Story 1

- [ ] T006 [US1] Extend RuntimeDiskController with typed operations for creating, checking, and removing an online snapshot view in crates/sdk/src/ports/runtime_disk.rs, and update its test implementations in crates/sdk/src/manager.rs.
- [ ] T007 [US1] Implement per-VM snapshot COW file allocation, exact loop-backing verification, deterministic Device Mapper identity, PO with P fallback, status checks, stale-resource reconciliation, safe origin suspend/resume, and ordered cleanup in crates/sdk/src/adapters/runtime/device_mapper.rs.
- [ ] T008 [P] [US1] Add the private archive module and implement bounded TAR-to-Zstandard-to-age streaming, payload hashing, finalization, mode-0600 encrypted temporary output, atomic no-clobber publication, and cleanup in crates/sdk/src/adapters/archive/mod.rs, crates/sdk/src/adapters/mod.rs, and crates/sdk/src/adapters/archive/age_tar_zstd.rs.
- [ ] T009 [US1] Implement the SDK create_snapshot operation and cancellation-aware variant in crates/sdk/src/manager.rs; acquire the existing lifecycle and volume locks, verify live-process and mapper ownership, use the snapshot view only for a running VM, copy a stopped VM's stable rootfs directly, and always clean up without changing the public VM/rootfs result contract.

**Checkpoint**: The SDK can capture a stable running or stopped root disk into a completed encrypted output and preserve the source VM on failure.

---

## Phase 4: User Story 2 - Create a snapshot from the CLI (Priority: P1)

**Goal**: Expose the SDK snapshot operation through interactive and scripted command forms without duplicating lifecycle behavior.

**Independent Test**: Verify microvm snapshot, microvm snapshot NAME, and microvm snapshot NAME OUTPUT_PATH call the SDK with the resolved VM and output path; verify hidden password input and actionable non-interactive errors.

### Tests for User Story 2

- [ ] T010 [P] [US2] Add command-surface tests for all three positional forms, --password parsing, VM selection when interactive, missing-name/password errors without a TTY, default output path, destination collision, password redaction, and success path reporting in crates/cli/src/cli.rs, crates/cli/src/commands/snapshot.rs, and crates/cli/tests/command_surface.rs.

### Implementation for User Story 2

- [ ] T011 [US2] Add SnapshotArgs and the Snapshot command variant, including optional name, optional output path, and --password, and register its dispatcher in crates/cli/src/cli.rs and crates/cli/src/commands/mod.rs.
- [ ] T012 [US2] Implement VM selection, default path construction, hidden password prompt, non-interactive validation, SDK invocation, calm errors, and final-path reporting in crates/cli/src/commands/snapshot.rs.
- [ ] T013 [US2] Route snapshot privilege escalation through the child process, prompt only after elevation when no password flag is supplied, propagate cancellation to SnapshotCancellation, and wait for cleanup before forced child termination in crates/cli/src/commands/snapshot.rs and crates/cli/src/privilege.rs.

**Checkpoint**: All specified CLI forms work over the SDK, refuse overwrites, and keep passwords out of user-facing output.

---

## Phase 5: User Story 3 - Keep the archive portable and protected (Priority: P1)

**Goal**: Complete the encrypted archive with all guest and portable configuration data needed for bootable recovery on a compatible host, without source-host database or runtime dependencies.

**Independent Test**: Decrypt an archive with the correct password and verify its manifest, rootfs, exact kernel, SSH public/private keys, file modes, sizes, and SHA-256 hashes. Verify wrong passwords, tampering, truncation, and source-host-only metadata are rejected or absent.

### Tests for User Story 3

- [ ] T014 [P] [US3] Add archive tests for manifest-last ordering, required member names and modes, streaming payload hashes, successful age decryption, wrong-password rejection, tamper/truncation rejection, and absence of a plaintext archive temporary file in crates/sdk/src/adapters/archive/age_tar_zstd.rs.
- [ ] T015 [P] [US3] Add a portability contract test that checks required rootfs/kernel/SSH members and asserts that absolute source paths, database rows, process/socket IDs, mapper/loop names, and host network resources are absent from the decrypted archive in crates/sdk/tests/snapshot_portability.rs.
- [ ] T016 [P] [US3] Add SQLite tests for retrieving the VM's stored distribution boot arguments and artifact provenance without registry access or schema changes in crates/sdk/src/adapters/persistence/sqlite.rs.

### Implementation for User Story 3

- [ ] T017 [US3] Add a typed snapshot metadata projection and read method for stored distribution boot data and provenance in crates/sdk/src/ports/repository.rs.
- [ ] T018 [US3] Implement the read-only snapshot metadata query using the existing distribution, image, kernel, and boot-argument tables without adding a migration in crates/sdk/src/adapters/persistence/sqlite.rs.
- [ ] T019 [US3] Build portable archive input from the stored VM record, local verified kernel artifact, SSH key files, and filtered logical network configuration in crates/sdk/src/manager.rs; reject missing, non-regular, or metadata-inconsistent source files without fetching the registry.
- [ ] T020 [US3] Complete the versioned manifest and TAR payload set in crates/sdk/src/adapters/archive/age_tar_zstd.rs with rootfs, exact kernel, SSH public/private keys, boot/resource/provenance metadata, compatibility requirements, and SHA-256/size records, while excluding source-host paths, runtime binaries, database state, and network ownership data.

**Checkpoint**: A correct password yields a complete portable recovery archive; incorrect passwords and modified/truncated streams are rejected.

---

## Phase 6: Polish and Cross-Cutting Concerns

**Purpose**: Align documentation and user-facing details with the implemented format, then validate the complete feature.

- [ ] T021 Update specs/020-vm-snapshot/quickstart.md with the final implemented archive member names, CLI examples, Linux prerequisites, and any changed validation commands.
- [ ] T022 Run cargo fmt --all -- --check, cargo check --all-targets --all-features, cargo clippy --all-targets --all-features -- -D warnings, cargo test --all-targets --all-features, and the privileged/manual scenarios in specs/020-vm-snapshot/quickstart.md; record outcomes and host-dependent skips in that quickstart.

---

## Dependencies and Execution Order

### Phase dependencies

- Setup (Phase 1): T001 has no dependencies.
- Foundational (Phase 2): T002 and T003 depend on T001 and block all user stories.
- User Story 1 (Phase 3): T004-T009 depend on the foundational phase; T006 precedes T007, T007 and T008 precede T009.
- User Story 2 (Phase 4): T010-T013 depend on the SDK operation from T009; tests precede CLI implementation.
- User Story 3 (Phase 5): T014-T020 depend on the snapshot archive path from T009; tests precede metadata and archive implementation. T017 precedes T018; T018 precedes T019; T019 precedes T020.
- Polish (Phase 6): T021 follows the implemented interface and archive format; T022 follows all desired stories.

### User story dependencies

- **US1 (P1)**: Starts after Phase 2; no other user-story dependency.
- **US2 (P1)**: Depends on US1 because the CLI delegates to the SDK create_snapshot operation.
- **US3 (P1)**: Depends on US1's capture and archive pipeline. After T009, US2 and US3 can proceed in parallel because the CLI surface and portable archive metadata are separate work areas.

### Parallel opportunities

- T002 and T003 touch separate SDK files and can proceed in parallel.
- T004 and T005 are independent test work in separate SDK files.
- T007 and T008 can proceed in parallel after T006 because Device Mapper management and archive streaming have separate files.
- After T009, T010 and the US3 test tasks T014-T016 can proceed in parallel; subsequent CLI implementation tasks and US3 repository/archive implementation tasks can also proceed in parallel after their own tests and dependencies are satisfied.

### Parallel Example: after User Story 1

    Developer A: T010, then T011-T013 in crates/cli/src/
    Developer B: T014-T016, then T017-T020 in crates/sdk/src/

---

## Implementation Strategy

### MVP First

1. Complete Phase 1 and Phase 2.
2. Complete Phase 3 (US1) to prove online and stopped disk capture, encrypted streaming, and safe cleanup.
3. Validate US1 independently with the live-write integration scenario and failure/cancellation paths.
4. Treat US1 as the technical MVP only; complete US3 before presenting the archive as portable recovery.
5. Add US2 for the requested CLI entry point. US2 and US3 may proceed in parallel after the SDK API is stable.

### Incremental Delivery

1. Add shared dependencies and SDK contracts.
2. Implement and validate stable disk capture and encrypted publication.
3. Add the CLI command using the SDK.
4. Complete and validate portable manifest, kernel, credentials, and cross-host metadata.
5. Run the workspace gates and quickstart scenarios.

### Notes

- Tests are included because the feature spec explicitly defines independent test criteria and measurable outcomes.
- [P] means different files and no dependency on unfinished tasks.
- Each task includes a concrete repository path and uses the required checkbox, task ID, optional parallel marker, and story label format.

