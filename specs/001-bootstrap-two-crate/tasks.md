---

description: "Task list for the Bootstrap Two-Crate Workspace feature"
---

# Tasks: Bootstrap Two-Crate Workspace

**Input**: Design documents from `specs/001-bootstrap-two-crate/`

**Prerequisites**: `plan.md` and `spec.md`; supporting research, data model, contracts, and
quickstart documents are available in the feature directory.

**Tests**: Required by `FR-010` in `spec.md`. Test tasks are placed before their corresponding
implementation tasks within each user story.

**Clarification precedence**: The clarified `spec.md` is authoritative. Use MIT for every
project component, keep Clap as the only initial third-party CLI dependency, treat the SDK
example function as temporary, and make `microvm` without arguments exit successfully after
showing concise help guidance. These decisions supersede the earlier dual-license assumption in
`research.md`.

**Organization**: Tasks are grouped by user story so each story can be implemented and tested as
an independently verifiable increment after the shared foundation is ready.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish the virtual Cargo workspace and the two package manifests.

- [X] T001 Convert the root `Cargo.toml` into a virtual workspace and remove the obsolete root binary at `src/main.rs`, declaring `crates/sdk` and `crates/cli` as members, resolver `"3"`, Rust 2024 edition, version `0.1.0`, English metadata, and MIT licensing.
- [X] T002 [P] Create the publishable SDK manifest in `crates/sdk/Cargo.toml` with package name `taumaru-microvm`, a library target at `crates/sdk/src/lib.rs`, crates.io-ready metadata, MIT licensing, and no runtime dependencies.
- [X] T003 [P] Create the binary-only CLI manifest in `crates/cli/Cargo.toml` with package name `taumaru-microvm-cli`, `publish = false`, a binary target named `microvm` at `crates/cli/src/main.rs`, a path dependency on `taumaru-microvm`, and Clap 4.6 with the `derive` feature as the only third-party runtime dependency.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Make both package targets compile before story-specific behavior is added.

**⚠️ CRITICAL**: No user story implementation can begin until this phase is complete.

- [X] T004 [P] Add a compileable SDK library target with English module-level Rustdoc and no runtime initialization in `crates/sdk/src/lib.rs`.
- [X] T005 [P] Add a compileable CLI binary entry point with no database, registry, Firecracker, or other runtime-resource initialization in `crates/cli/src/main.rs`.
- [X] T006 Generate the shared workspace lockfile at `Cargo.lock` and verify with `cargo metadata --no-deps --format-version 1` that exactly `taumaru-microvm` and `taumaru-microvm-cli` are workspace packages and that the CLI points to the SDK path dependency.

**Checkpoint**: The virtual workspace and empty package targets compile, and the dependency
direction is established before story work begins.

---

## Phase 3: User Story 1 - Build the Project Foundation (Priority: P1) 🎯 MVP

**Goal**: Deliver independently identifiable SDK and CLI packages with the required publication,
binary, and dependency boundaries.

**Independent Test**: Run workspace metadata and build checks, run the CLI package-boundary test,
and confirm that the SDK is publication-ready while the CLI is excluded from crates.io.

### Tests for User Story 1

- [X] T007 [P] [US1] Add an external-style package-boundary test in `crates/cli/tests/package_boundary.rs` that imports the `taumaru-microvm` SDK dependency and asserts the CLI package identity is `taumaru-microvm-cli` without introducing a reverse dependency.

### Implementation and Verification for User Story 1

- [X] T008 [P] [US1] Verify the package boundary described by `Cargo.toml`, `crates/sdk/Cargo.toml`, `crates/cli/Cargo.toml`, and `Cargo.lock` using `cargo metadata`, `cargo check --workspace`, and `cargo tree -p taumaru-microvm-cli`; correct any package-name, publication, or dependency-direction drift.

**Checkpoint**: User Story 1 is independently complete when both packages build, the CLI can
compile against the SDK, the SDK remains publishable, and the CLI remains `publish = false`.

---

## Phase 4: User Story 2 - Consume the Example SDK API (Priority: P1)

**Goal**: Expose a documented, deterministic, temporary SDK example that external Rust code can
call without output, panic, process control, or hidden state.

**Independent Test**: Run the external-consumer contract test for the SDK and verify the exact
documented result across repeated calls.

### Tests for User Story 2

- [X] T009 [P] [US2] Add external-consumer contract tests in `crates/sdk/tests/public_api.rs` that call `example_message` through `std::panic::catch_unwind`, assert the exact result `taumaru-microvm SDK is ready`, verify repeatability, and keep the invocation free of test-authored output or logging.

### Implementation for User Story 2

- [X] T010 [US2] Implement `pub fn example_message() -> &'static str` in `crates/sdk/src/lib.rs` with English Rustdoc identifying it as a temporary bootstrap API, returning exactly `taumaru-microvm SDK is ready`, and performing no I/O, logging, tracing, environment lookup, persistence, process control, mutation, or panic-prone handling.
- [X] T011 [US2] Run `cargo test -p taumaru-microvm --test public_api` and `cargo doc -p taumaru-microvm --no-deps`, resolving contract or Rustdoc failures only in `crates/sdk/src/lib.rs` and `crates/sdk/tests/public_api.rs`.

**Checkpoint**: User Story 2 is independently complete when an external consumer can import the
SDK function, receive the exact deterministic value, and pass the no-panic and no-unsolicited-
output contract checks.

---

## Phase 5: User Story 3 - Use the `microvm` CLI Foundation (Priority: P1)

**Goal**: Deliver a thin, English, Clap-based CLI foundation with the exact executable name and
predictable help, version, empty-input, and invalid-input behavior.

**Independent Test**: Run the CLI process smoke test and the documented `cargo run` invocations
without Firecracker, KVM, registry access, database state, or an existing MicroVM.

### Tests for User Story 3

- [X] T012 [P] [US3] Add executable smoke tests in `crates/cli/tests/command_surface.rs` using `std::process::Command` and `CARGO_BIN_EXE_microvm` to cover `--help`, `--version`, no arguments, an unknown command, and an invalid option with the required exit statuses, English output, and no panic.

### Implementation for User Story 3

- [X] T013 [US3] Implement the typed Clap parser and thin entry point in `crates/cli/src/main.rs` with an explicit command name `microvm`, package-derived version, English description, standard help/version flags, concise help guidance with exit code `0` for no arguments, non-success Clap parse errors for invalid input, and no runtime-resource initialization or MicroVM lifecycle logic.
- [X] T014 [US3] Run `cargo test -p taumaru-microvm-cli --test command_surface` plus the `cargo run -p taumaru-microvm-cli -- --help`, `--version`, empty-input, and invalid-input commands documented in `specs/001-bootstrap-two-crate/quickstart.md`, correcting only the CLI source or smoke tests as needed.

**Checkpoint**: User Story 3 is independently complete when the installed or built command is
`microvm`, baseline help/version/no-argument paths succeed, invalid input fails clearly, and the
CLI remains a presentation layer over the SDK boundary.

---

## Phase 6: Polish and Cross-Cutting Concerns

**Purpose**: Reconcile design artifacts, audit project conventions, and prove the complete
foundation against the repository quality gates.

- [X] T015 [P] Audit English-only source, identifiers, Rustdoc, CLI text, and test descriptions in `Cargo.toml`, `crates/sdk/Cargo.toml`, `crates/cli/Cargo.toml`, `crates/sdk/src/lib.rs`, `crates/sdk/tests/public_api.rs`, `crates/cli/src/main.rs`, `crates/cli/tests/package_boundary.rs`, and `crates/cli/tests/command_surface.rs`, correcting any non-English project artifact.
- [X] T016 [P] Synchronize the post-plan decisions in `specs/001-bootstrap-two-crate/plan.md`, `specs/001-bootstrap-two-crate/research.md`, `specs/001-bootstrap-two-crate/data-model.md`, `specs/001-bootstrap-two-crate/contracts/sdk.md`, `specs/001-bootstrap-two-crate/contracts/cli.md`, and `specs/001-bootstrap-two-crate/quickstart.md` so they consistently state MIT licensing, the temporary SDK example, Clap-only baseline dependencies, and successful no-argument CLI behavior.
- [X] T017 [P] Audit publication and dependency metadata in `Cargo.toml`, `crates/sdk/Cargo.toml`, `crates/cli/Cargo.toml`, and `Cargo.lock` with `cargo metadata`, `cargo tree`, and `cargo package -p taumaru-microvm --allow-dirty --no-verify`, confirming MIT metadata, SDK publication readiness, CLI `publish = false`, and no unused direct baseline dependencies.
- [X] T018 Run the complete quality gates from `specs/001-bootstrap-two-crate/quickstart.md`: `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo test --workspace`; resolve failures in the affected source, manifest, or test paths before marking the feature complete.
- [X] T019 Run every validation command in `specs/001-bootstrap-two-crate/quickstart.md` from a clean workspace state and confirm the final package boundary, SDK contract, CLI command surface, English-only output, and out-of-scope runtime-resource assumptions.

---

## Dependencies and Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: T001 must complete before T002 and T003; T002 and T003 can then run in parallel.
- **Foundational (Phase 2)**: T004 and T005 depend on the manifests from Phase 1 and can run in parallel; T006 depends on T004 and T005 and blocks all user stories.
- **User Stories (Phases 3-5)**: T007/T008, T009, and T012 can begin in parallel after T006 because they touch separate test or verification surfaces. Each story's implementation follows its own tests.
- **Polish (Phase 6)**: T015-T017 can run in parallel after the three story checkpoints; T018 and T019 run after those audits.

### User Story Dependencies

- **User Story 1 (P1)**: Depends only on Phase 2 and has no dependency on the other stories.
- **User Story 2 (P1)**: Depends only on Phase 2; its SDK contract is independently testable.
- **User Story 3 (P1)**: Depends only on Phase 2; its CLI process behavior is independently testable and uses the already-declared SDK path dependency without adding lifecycle logic.
- All three stories have the same priority and may proceed in parallel after Phase 2 when separate files are assigned.

### Within Each User Story

- Write the story's tests before its implementation tasks.
- Keep package boundaries and public contracts explicit in the paths named by each task.
- Run the story checkpoint before starting cross-cutting polish.

## Parallel Execution Examples

### After the Foundational Phase

```text
T007 [US1] package-boundary test in crates/cli/tests/package_boundary.rs
T008 [US1] package metadata verification using Cargo.toml and package manifests
T009 [US2] SDK contract test in crates/sdk/tests/public_api.rs
T012 [US3] CLI process smoke test in crates/cli/tests/command_surface.rs
```

These tasks use separate files or read-only verification surfaces and can be prepared in parallel
after T006.

### User Story 1

```text
T007 and T008 can run in parallel after T006.
```

### User Story 2

```text
T009 must complete before T010; T011 follows the implementation.
No additional intra-story parallel split is useful because the API and its contract share one
small public surface.
```

### User Story 3

```text
T012 must complete before T013; T014 follows the implementation.
The process test and parser entry point intentionally remain sequential within this story.
```

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational.
3. Complete Phase 3: User Story 1.
4. Stop and validate both package boundaries independently.
5. Use the resulting workspace as the MVP foundation for the SDK and CLI increments.

### Incremental Delivery

1. Add User Story 2 after the workspace boundary is proven; validate the temporary public SDK API.
2. Add User Story 3 after the workspace boundary is proven; validate the `microvm` command surface.
3. Run Phase 6 only after all desired story checkpoints pass.
4. Keep Firecracker, registry, database, persistence, and production VM commands out of this feature.

### Parallel Team Strategy

1. One contributor completes T001-T006 and establishes the shared foundation.
2. After T006, contributors can work in parallel on US1 boundary checks, US2 SDK tests/API, and US3 CLI tests/parser.
3. A final contributor performs T015-T019 after the story checkpoints are complete.

## Notes

- `[P]` means the task can run in parallel because it uses a different file or a read-only verification surface and has no dependency on incomplete work.
- `[US1]`, `[US2]`, and `[US3]` map tasks to the user stories in `spec.md`.
- Every task includes a concrete repository path and follows the required checkbox, sequential ID, optional parallel marker, optional story label, and action-description format.
- Do not add Firecracker, `firectl`, registry, database, async, logging, table, progress, JSON, or color dependencies until a later feature uses the capability.
