# Tasks: Unencrypted MicroVM Snapshots and Restore

**Input**: Design documents from `specs/022-remove-snapshot-encryption/`

**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`

**Tests**: Included because the project constitution requires tests for SDK and CLI behavior changes, and the feature contracts define archive integrity, compatibility, and atomicity cases.

**Organization**: Tasks are grouped by user story. User Story 2 consumes the version 3 archive defined and emitted by User Story 1.

## Phase 1: Setup

**Purpose**: Prepare project structure and tooling.

The Rust workspace, SDK, CLI, archive adapter, and test targets already exist. No setup changes are required.

---

## Phase 2: Foundational

**Purpose**: Complete prerequisites that block all user stories.

The existing SDK archive boundary, restore journal, CLI wiring, and workspace are sufficient foundations. No shared infrastructure changes are required before the story-specific work.

---

## Phase 3: User Story 1 - Create an unencrypted snapshot (Priority: P1) 🎯 MVP

**Goal**: Produce a portable Zstandard-compressed TAR snapshot without an encryption layer, password input, or password prompt while preserving capture consistency and operating-system file access defaults.

**Independent Test**: Snapshot running and stopped VMs through the SDK and CLI; read the resulting archive without a secret, verify manifest version 3 and integrity checks, confirm the source lifecycle state is preserved, and confirm output permissions follow the process umask and destination directory policy.

### Tests for User Story 1

> Write these tests first and confirm they fail against the current password-based behavior.

- [ ] T001 [P] [US1] Cover password-free snapshot method signatures and `SnapshotResult.archive_size_bytes` in `crates/sdk/tests/public_api.rs`.
- [ ] T002 [P] [US1] Cover readable version 3 Zstandard/TAR output, frame and payload checksums, running/stopped source state, and default output permissions in `crates/sdk/tests/snapshot.rs`.
- [ ] T003 [P] [US1] Cover snapshot help and parsing without `--password`, no secret echo, interactive and non-interactive operation, and the plaintext contents disclosure in `crates/cli/tests/command_surface.rs`.

### Implementation for User Story 1

- [ ] T004 [P] [US1] Remove password parameters from public snapshot methods and rename `encrypted_size_bytes` to `archive_size_bytes` in `crates/sdk/src/manager.rs` and `crates/sdk/src/domain/snapshot.rs`.
- [ ] T005 [P] [US1] Set the snapshot manifest format version to 3 while retaining the existing portable manifest fields in `crates/sdk/src/adapters/archive/manifest.rs`.
- [ ] T006 [US1] Rename `crates/sdk/src/adapters/archive/age_tar_zstd.rs` to `crates/sdk/src/adapters/archive/tar_zstd.rs`, update module wiring in `crates/sdk/src/adapters/archive/mod.rs`, and write TAR through Zstandard level 3 with the frame checksum and existing bounded buffers; remove age encryption and the explicit owner-only staging mode so the process umask and destination default ACL determine access, while preserving synchronization, no-overwrite publication, cleanup, cancellation, and progress behavior.
- [ ] T007 [US1] Remove snapshot password arguments, prompts, non-interactive validation, and password forwarding from the CLI definitions and command flow in `crates/cli/src/cli.rs` and `crates/cli/src/commands/snapshot.rs`.
- [ ] T008 [US1] Update snapshot completion and progress wording to identify the archive as unencrypted, disclose that it contains the VM disk and SSH credentials, and report `archive_size_bytes` in `crates/cli/src/output/human.rs`.

**Checkpoint**: Snapshot creation works from the SDK and CLI without a password; the produced archive is readable, integrity-checked, and published with the selected file-access policy.

---

## Phase 4: User Story 2 - Restore a VM without a password (Priority: P1)

**Goal**: Restore supported version 3 snapshots without a password, reject previous age-encrypted archives before staging, and preserve atomic restore behavior.

**Independent Test**: Restore a supported unencrypted archive through interactive and non-interactive CLI flows, confirm the reconstructed VM is discoverable and stopped, and confirm legacy, corrupt, truncated, and unsupported archives fail without publishing partial state.

### Tests for User Story 2

> Write these tests first and confirm they fail against the current password-required behavior.

- [ ] T009 [P] [US2] Cover a password-free `RestoreRequest` and public restore contract in `crates/sdk/tests/public_api.rs`.
- [ ] T010 [P] [US2] Cover successful stopped-VM restoration, legacy age-header rejection before staging, version and checksum validation, corruption cleanup, and unchanged destination state in `crates/sdk/tests/restore.rs`.
- [ ] T011 [P] [US2] Cover restore help and parsing without `--password`, no secret echo or prompt, non-interactive operation, and actionable legacy-format errors in `crates/cli/tests/command_surface.rs`.

### Implementation for User Story 2

- [ ] T012 [P] [US2] Remove the password field from `RestoreRequest` and remove password inputs from public restore operations in `crates/sdk/src/domain/restore.rs` and `crates/sdk/src/manager/restore.rs`.
- [ ] T013 [P] [US2] Replace archive decryption with plain Zstandard/TAR reading, detect the legacy age header before staging, accept only manifest version 3, and retain strict member, size, digest, SSH-key, checksum, cleanup, and atomic-commit validation in `crates/sdk/src/adapters/archive/restore.rs`; update the legacy compatibility wording in `crates/sdk/src/error.rs`.
- [ ] T014 [US2] Remove restore password options, prompts, non-interactive password checks, and password forwarding from `crates/cli/src/cli.rs` and `crates/cli/src/commands/restore.rs`.
- [ ] T015 [US2] Update restore progress and error presentation to describe reading, verifying, and installing archives without encryption or decryption claims in `crates/cli/src/output/human.rs`.

**Checkpoint**: Supported snapshots restore without passwords and remain stopped; legacy encrypted and invalid archives fail with an actionable error and no partial destination state.

---

## Phase 5: Polish & Cross-Cutting Concerns

**Purpose**: Complete the breaking API migration, remove obsolete dependencies, and validate the full workspace flow.

- [ ] T016 Remove the `age` dependency from `Cargo.toml`, `crates/sdk/Cargo.toml`, and `Cargo.lock` after compatibility tests use a static legacy-header fixture in `crates/sdk/tests/restore.rs`.
- [ ] T017 Bump the workspace package version to 0.2.0 in `Cargo.toml` and document removed password inputs, the `archive_size_bytes` rename, and the need to recreate legacy archives in `crates/sdk/src/lib.rs`.
- [ ] T018 Run `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` for `Cargo.toml`, `crates/sdk/Cargo.toml`, and `crates/cli/Cargo.toml`, then validate the manual compatibility and permissions flow in `specs/022-remove-snapshot-encryption/quickstart.md`.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No changes are required; the existing workspace is ready.
- **Foundational (Phase 2)**: No changes are required; existing SDK and CLI boundaries unblock story work.
- **User Stories (Phase 3+)**: User Story 1 defines and writes the version 3 archive. User Story 2 follows it and implements the matching reader and legacy-format rejection.
- **Polish (Phase 5)**: Depends on both stories so obsolete dependencies can be removed and the completed workspace can be validated.

### User Story Dependencies

- **User Story 1 (P1)**: Starts immediately because the existing archive adapter and workspace provide its prerequisites.
- **User Story 2 (P1)**: Follows User Story 1 because restore must consume the plain version 3 format that snapshot creation emits.

### Within Each User Story

- Write SDK and CLI contract tests before implementation and confirm they fail against the current behavior.
- Complete the SDK archive behavior before the corresponding CLI command wiring and presentation.
- Keep the existing SDK lifecycle, integrity, cancellation, and atomicity boundaries intact.

### Parallel Opportunities

- User Story 1 test tasks T001, T002, and T003 touch separate files and can run in parallel.
- User Story 1 API and manifest tasks T004 and T005 touch separate files and can run in parallel; T006 depends on the version 3 manifest contract from T005.
- User Story 2 test tasks T009, T010, and T011 touch separate files and can run in parallel after User Story 1 establishes the archive contract.
- User Story 2 SDK request and reader tasks T012 and T013 touch separate files and can run in parallel; CLI integration T014 follows the API change.

---

## Parallel Example: User Story 1

```text
Task: T001 password-free SDK snapshot contract tests in crates/sdk/tests/public_api.rs
Task: T002 archive format and snapshot behavior tests in crates/sdk/tests/snapshot.rs
Task: T003 snapshot CLI surface tests in crates/cli/tests/command_surface.rs
```

## Parallel Example: User Story 2

```text
Task: T009 password-free SDK restore contract tests in crates/sdk/tests/public_api.rs
Task: T010 restore compatibility and failure-path tests in crates/sdk/tests/restore.rs
Task: T011 restore CLI surface tests in crates/cli/tests/command_surface.rs
```

---

## Implementation Strategy

### MVP First (User Story 1)

1. Complete the existing-workspace setup and foundation checkpoints.
2. Complete User Story 1 and validate unencrypted snapshot creation independently.
3. Treat User Story 2 as required before release because the requested passwordless workflow includes restoration.

### Incremental Delivery

1. Implement snapshot creation and its SDK/CLI coverage.
2. Implement matching passwordless restore and legacy archive rejection.
3. Remove the age dependency, publish the 0.2.0 migration guidance, and run the full quickstart and quality gates.

## Notes

- `[P]` marks tasks in separate files that have no unfinished-task dependency.
- `[US1]` and `[US2]` map each task to the corresponding feature story.
- Restore compatibility tests use a static legacy header so the removed age dependency is not retained for test generation.
- Archive checksums detect accidental corruption; they do not provide authenticity or confidentiality.
