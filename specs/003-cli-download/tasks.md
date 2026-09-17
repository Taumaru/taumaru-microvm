---

description: "Actionable task list for the CLI artifact download feature"
---

# Tasks: CLI Artifact Download

**Input**: Design documents from `/specs/003-cli-download/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/cli-download.md](./contracts/cli-download.md), and
[quickstart.md](./quickstart.md)

**Tests**: Test tasks are included because the specification, constitution, and feature request
require unit, failure-path, command-surface, and SDK-to-CLI boundary coverage. Test-first tasks
must be written and observed failing before the corresponding implementation task is completed.

**Organization**: Tasks are grouped by user story so each story has an explicit goal, independent
test criteria, implementation work, and a validation checkpoint.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add the dependencies and CLI module boundaries required by the planned command.

- [ ] T001 Add workspace and crate dependency declarations for `inquire` 0.9.x, `indicatif` 0.18.x, `semver` 1.x, `tokio-util` 0.7.x, and Tokio `macros`, `rt-multi-thread`, and `signal` features in `Cargo.toml`, `crates/cli/Cargo.toml`, and `crates/sdk/Cargo.toml`; update `Cargo.lock` without adding HTTP, SQLite, hashing, or Firecracker dependencies to the CLI.
- [ ] T002 Split the current CLI bootstrap into `crates/cli/src/main.rs`, `crates/cli/src/cli.rs`, `crates/cli/src/context.rs`, `crates/cli/src/error.rs`, `crates/cli/src/commands/mod.rs`, `crates/cli/src/commands/download.rs`, `crates/cli/src/output/mod.rs`, and `crates/cli/src/output/human.rs`, preserving existing help, version, no-argument, and invalid-input behavior.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Establish error, home, cancellation, and test seams before any user story is built.

**CRITICAL**: No user story implementation can begin until this phase is complete.

- [ ] T003 [P] Define typed CLI errors, user-facing what/why/next formatting hooks, and exit-code mapping (`0` success, `1` incomplete/failed plan, `130` cancellation) in `crates/cli/src/error.rs` and `crates/cli/src/main.rs` without `unwrap`, `expect`, panic output, stack traces, or SDK-side output.
- [ ] T004 [P] Implement explicit home resolution and terminal capability detection in `crates/cli/src/context.rs` using `TAUMARU_HOME` when set and `~/.taumaru-microvm` otherwise; pass the resolved path to `MicroVmSdk::new`, detect TTY/`NO_COLOR`, and keep environment lookup out of the SDK.
- [ ] T005 [P] Write failing SDK tests for cooperative cancellation, typed cancellation errors/phases, preservation of already verified members, and cleanup of an interrupted temporary file in `crates/sdk/tests/download_flow.rs` and `crates/sdk/tests/failure_paths.rs`.
- [ ] T006 Implement `DownloadCancellation`, cancellation-aware `download_kernel_with_cancellation`, `download_binary_with_cancellation`, and `download_distribution_with_cancellation` paths in `crates/sdk/src/domain/artifact.rs`, `crates/sdk/src/manager.rs`, `crates/sdk/src/error.rs`, and `crates/sdk/src/lib.rs`; preserve existing uncancelled methods as compatible wrappers, remove temporary files before returning cancellation, never publish partial targets, and add the `tokio-util` dependency in `crates/sdk/Cargo.toml`.
- [ ] T007 Define the CLI-internal SDK-shaped artifact client, progress sink, and cancellation-aware execution seams in `crates/cli/src/commands/download.rs` and `crates/cli/src/output/mod.rs`, keeping production behavior delegated to `MicroVmSdk` and allowing deterministic fakes without a second filesystem, checksum, cache, or SQLite implementation.

**Checkpoint**: CLI modules compile as a wired skeleton, SDK cancellation tests define the safety
contract, and the foundation owns no feature-specific artifact state.

---

## Phase 3: User Story 1 - Build a Download Plan (Priority: P1) 🎯 MVP

**Goal**: Let an operator select one or more host-compatible distributions, choose exactly one
compatible kernel for each, review the automatically selected runtime package, and confirm a
complete plan before any artifact transfer.

**Independent Test**: Against a deterministic catalog, use keyboard input to select at least two
distributions and one compatible kernel for each; verify that the review shows all pairs, runtime
package, image count, and estimated bytes, and that cancelling before confirmation makes zero
download calls.

### Tests for User Story 1

- [ ] T008 [US1] Write failing unit tests for catalog assembly, host-architecture filtering, and rejection of empty or duplicate registry identifiers in `crates/cli/src/commands/download.rs`.
- [ ] T009 [US1] Write failing unit tests for explicit `distribution-id=kernel-id` parsing and validation in `crates/cli/src/commands/download.rs`, covering exactly one mapping per selected distribution, no mapping for an unselected distribution, unavailable kernels, incompatible kernels, and duplicate mappings.
- [ ] T010 [US1] Write failing unit tests for runtime package selection and plan construction in `crates/cli/src/commands/download.rs`, covering the highest valid semantic version, required `firecracker` and `firectl` components, host architecture, deterministic ties, checked byte totals, unique kernel IDs, and deterministic member order.
- [ ] T011 [P] [US1] Extend command-surface tests in `crates/cli/tests/command_surface.rs` for `microvm download`, repeatable `--distribution` and `--kernel` options, `--non-interactive`, help text, and the no-TTY incomplete-selection guard.

### Implementation for User Story 1

- [ ] T012 [US1] Define the Clap root, `Download` subcommand, repeatable distribution/kernel options, and `--non-interactive` flag in `crates/cli/src/cli.rs` and register the command in `crates/cli/src/commands/mod.rs` while retaining the existing top-level command behavior.
- [ ] T013 [US1] Implement `RegistryCatalog`, `DistributionSelection`, `RuntimeBinarySelection`, `DownloadPlan`, and `PlanMember` in `crates/cli/src/commands/download.rs`, including host architecture mapping, supported-kernel intersection, default-kernel marking, runtime package filtering, semantic version ordering, required component checks, checked size totals, unique-kernel deduplication, and deterministic distribution/kernel ordering.
- [ ] T014 [US1] Implement explicit selection parsing and non-interactive validation in `crates/cli/src/commands/download.rs`, rejecting empty selections, duplicate IDs, malformed mappings, incomplete mappings, unselected distribution mappings, unknown IDs, incompatible architectures, and kernels absent from a distribution's supported-kernel set before confirmation or transfer.
- [ ] T015 [US1] Implement the interactive `inquire::MultiSelect`, per-distribution `inquire::Select`, review, and `inquire::Confirm` flow in `crates/cli/src/commands/download.rs` and `crates/cli/src/output/human.rs`, showing focus/selection markers, default-kernel text, distribution/kernel pairs, runtime package, image count, estimated size, and cancellation before transfer.
- [ ] T016 [US1] Wire catalog loading, explicit-versus-interactive selection, plan validation, review, and confirmation into `crates/cli/src/main.rs`, `crates/cli/src/context.rs`, `crates/cli/src/commands/mod.rs`, and `crates/cli/src/commands/download.rs`, guaranteeing that no SDK download operation is called before confirmation and that pre-transfer cancellation maps to exit code `130`.

**Checkpoint**: User Story 1 is independently usable as a safe plan-building flow; it can list,
select, validate, review, confirm, or cancel without transferring artifacts.

---

## Phase 4: User Story 2 - Download the Planned Artifacts (Priority: P1)

**Goal**: Execute a confirmed plan through the SDK in runtime-binary, unique-kernel, and
distribution-image order, downloading each required artifact group exactly as represented by the
plan.

**Independent Test**: Execute a confirmed fake-client plan with multiple distributions sharing a
kernel and with a multi-image distribution; verify one runtime call first, one call per unique
kernel, one call per distribution, and success only after every SDK result is verified.

### Tests for User Story 2

- [ ] T017 [US2] Write failing executor tests with an SDK-shaped fake in `crates/cli/src/commands/download.rs` that record calls and assert runtime binary first, unique kernels once, deterministic kernel/distribution order, and no lifecycle or direct Firecracker invocation.
- [ ] T018 [US2] Write failing success-path tests in `crates/cli/src/commands/download.rs` for multi-image distributions and the invariant that the command reports success only after the runtime package, every unique kernel, and every selected distribution image group returns a verified SDK result.

### Implementation for User Story 2

- [ ] T019 [US2] Implement the generic confirmed-plan executor in `crates/cli/src/commands/download.rs`, executing the runtime binary member first, then each unique kernel, then each selected distribution image group in deterministic order and collecting verified `DownloadedBinary`, `DownloadedKernel`, and `DownloadedDistribution` results.
- [ ] T020 [US2] Implement the production SDK artifact client adapter in `crates/cli/src/commands/download.rs`, forwarding IDs and callbacks to the SDK's cancellation-aware download methods without inspecting paths, hashes, file existence, SQLite, or cache state in the CLI.
- [ ] T021 [US2] Connect the confirmed plan to the executor and final success result in `crates/cli/src/main.rs`, `crates/cli/src/commands/mod.rs`, and `crates/cli/src/commands/download.rs`, returning success only when all planned groups are verified and retaining the runtime prerequisite short-circuit for later failure handling.

**Checkpoint**: A confirmed plan performs the complete happy-path acquisition through the SDK,
deduplicates shared kernels, includes every distribution image group, and cannot report premature
success.

---

## Phase 5: User Story 3 - Reuse Artifacts and Report Progress (Priority: P1)

**Goal**: Make repeated execution visible and efficient by rendering SDK progress and distinguishing
fresh downloads, adopted files, and already available files without duplicating cache logic.

**Independent Test**: Feed the executor downloaded, adopted, and skipped SDK events for a repeated
plan; verify no local cache decision is made by the CLI, all byte counters are truthful, and both
interactive and non-TTY renderers preserve identity and terminal disposition labels.

### Tests for User Story 3

- [ ] T022 [US3] Write failing tests in `crates/cli/src/commands/download.rs` for forwarding `Downloaded`, `AdoptedExisting`, and `SkippedExisting` SDK dispositions as `Downloaded`, `Adopted`, and `Already available` without issuing an extra transfer decision in the CLI.
- [ ] T023 [US3] Write failing progress-normalization tests in `crates/cli/src/commands/download.rs` for artifact/member identity, `Downloading`/`Verifying`/terminal stages, current and expected member bytes, completed-plan offsets, aggregate current/expected bytes, and no fabricated percentage or byte values.
- [ ] T024 [P] [US3] Write failing renderer tests in `crates/cli/src/output/human.rs` for TTY, non-TTY, `NO_COLOR`, and narrow-terminal output, ensuring focus, stage, identity, byte counters, success, failure, and cache dispositions remain distinguishable through text or symbols.

### Implementation for User Story 3

- [ ] T025 [US3] Implement the `indicatif` aggregate/current-member progress renderer and deterministic line renderer in `crates/cli/src/output/human.rs`, using stderr for progress, hiding bars for non-TTY output, respecting `NO_COLOR`, and retaining calm compact labels for narrow terminals.
- [ ] T026 [US3] Map SDK `DownloadProgress` callbacks into `DownloadProgressView` in `crates/cli/src/commands/download.rs`, adding only expected bytes from completed plan members to the SDK operation aggregate and mapping every SDK phase/disposition to explicit text.
- [ ] T027 [US3] Implement review and final result rendering in `crates/cli/src/output/human.rs`, including estimated and verified totals, current group, groups acquired/adopted/already available, successful groups, failed groups, skipped groups, and retry context without changing SDK semantics.

**Checkpoint**: Repeated downloads expose the SDK's intelligent reuse outcome, progress remains
truthful in every terminal mode, and the final output is compact, readable, and script-safe.

---

## Phase 6: User Story 4 - Recover Calmly from Failure (Priority: P2)

**Goal**: Return actionable nonzero outcomes, retain verified work, stop dependent operations at
the right boundary, and handle controlled transfer cancellation safely.

**Independent Test**: Use a fake client that fails the runtime, a selected kernel, and a distribution
image group in separate runs; verify runtime short-circuits all dependent calls, kernel failure
skips only its distribution images while unrelated groups continue, and the final result separates
successful, failed, skipped, and cancelled groups.

### Tests for User Story 4

- [ ] T028 [US4] Write failing tests in `crates/cli/src/commands/download.rs` proving that a runtime-bundle failure produces a nonzero outcome and makes zero kernel or distribution calls.
- [ ] T029 [US4] Write failing tests in `crates/cli/src/commands/download.rs` proving that a selected kernel failure skips only the associated distribution image group, continues unrelated groups, preserves earlier verified results, and separates successful and failed groups in the summary.
- [ ] T030 [US4] Write failing partial-failure tests in `crates/cli/src/commands/download.rs` and `crates/cli/src/output/human.rs` for a multi-image distribution failure, retained successful members, actionable what/why/next text, and exit code `1`.
- [ ] T031 [P] [US4] Write failing cancellation tests in `crates/sdk/tests/download_flow.rs` and `crates/cli/src/commands/download.rs` proving that `Ctrl-C` signals the SDK token, waits for cleanup, publishes no partial artifact, preserves verified groups, starts no subsequent group, reports cancellation, and returns exit code `130`.

### Implementation for User Story 4

- [ ] T032 [US4] Implement `MemberOutcome` and `DownloadOutcome` failure handling in `crates/cli/src/commands/download.rs`, making runtime failure terminal, skipping a distribution's images when its kernel fails, continuing unrelated groups, retaining verified results, and returning nonzero for failed or skipped required groups.
- [ ] T033 [US4] Implement cooperative `Ctrl-C` handling with `tokio::signal::ctrl_c` and the shared `DownloadCancellation` token in `crates/cli/src/main.rs` and `crates/cli/src/commands/download.rs`, awaiting SDK cleanup before returning, stopping current/subsequent plan members, and mapping cancellation to exit code `130`.
- [ ] T034 [US4] Complete typed error formatting and terminal summaries in `crates/cli/src/error.rs` and `crates/cli/src/output/human.rs`, explaining what happened, why preparation is incomplete or cancelled, what to do next, and which groups succeeded, failed, skipped, or were cancelled.

**Checkpoint**: Registry, filesystem, SDK, partial-download, and cancellation failures are calm,
actionable, nonzero outcomes; no invalid partial artifact is published and verified work remains
usable for retry.

---

## Phase 7: User Story 5 - Automate Explicit Selections (Priority: P2)

**Goal**: Run the same validated plan and executor without prompts in scripts or non-interactive
terminals using repeatable distribution IDs and complete distribution-to-kernel mappings.

**Independent Test**: Invoke the command path with explicit valid selections through a deterministic
fake client; verify no prompt is requested, the plan matches interactive selection semantics, output
is deterministic, and invalid mappings make zero download calls.

### Tests for User Story 5

- [ ] T035 [P] [US5] Add parser and process-level tests in `crates/cli/src/cli.rs` and `crates/cli/tests/command_surface.rs` for repeatable `--distribution`, repeatable `--kernel`, `--non-interactive`, no-prompt behavior, and deterministic non-TTY error text when selections are incomplete.
- [ ] T036 [US5] Write failing explicit-mode tests in `crates/cli/src/commands/download.rs` proving valid mappings produce the same `DownloadPlan` as interactive selections and invalid, unknown, incompatible, duplicate, or incomplete mappings start no transfer.

### Implementation for User Story 5

- [ ] T037 [US5] Finish the explicit-selection execution branch in `crates/cli/src/commands/download.rs` and `crates/cli/src/main.rs`, requiring complete mappings, bypassing `inquire` entirely, reusing the same catalog validation and confirmed-plan executor, and rejecting incomplete input before SDK downloads.
- [ ] T038 [US5] Ensure deterministic script output and cross-mode plan equivalence in `crates/cli/src/output/human.rs` and `crates/cli/src/commands/download.rs`, preserving stable ordering, stable status labels, concise summaries, and nonzero exit behavior for every incomplete explicit plan.

**Checkpoint**: Interactive and explicit invocations share one plan validator and executor; valid
automation has no prompts and invalid automation cannot trigger a download.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Document the public boundary, protect package direction, and run the complete quality
gates.

- [ ] T039 [P] Document the public cancellation token, cancellation-aware SDK methods, cancellation error/phase, and CLI command help in `crates/sdk/src/lib.rs`, `crates/sdk/src/domain/artifact.rs`, `crates/sdk/src/error.rs`, and `crates/cli/src/cli.rs` with English Rustdoc/help text and no undocumented compatibility-sensitive behavior.
- [ ] T040 [P] Add regression assertions for the CLI-to-SDK package boundary and direct-dependency policy in `crates/cli/tests/package_boundary.rs` and `crates/cli/Cargo.toml`, ensuring the CLI has no direct HTTP, SQLite, hashing, or Firecracker dependency and does not duplicate lifecycle behavior.
- [ ] T041 Run the focused and workspace checks from `specs/003-cli-download/quickstart.md`, including formatting, CLI tests, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features`; resolve regressions before marking the feature complete.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 Setup**: No dependencies; T001 must complete before code can use the new crates, and T002 establishes the module paths used by later tasks.
- **Phase 2 Foundational**: Depends on T001 and T002; T003, T004, and T005 can run in parallel, T006 depends on T005, and T007 depends on the CLI skeleton plus the SDK cancellation API.
- **Phase 3 US1**: Depends on Phase 2; its tests T008–T011 precede implementation T012–T016.
- **Phase 4 US2**: Depends on US1's confirmed plan and dispatch (T016); tests T017–T018 precede executor work T019–T021.
- **Phase 5 US3**: Depends on the US2 executor (T019–T021); tests T022–T024 precede renderer and callback work T025–T027.
- **Phase 6 US4**: Depends on the executor and progress/result seams (T021 and T025–T027); tests T028–T031 precede failure/cancellation work T032–T034.
- **Phase 7 US5**: Depends on the shared plan/executor and result output (T014, T021, and T027); tests T035–T036 precede explicit-mode work T037–T038.
- **Phase 8 Polish**: Depends on all desired user stories; T041 is the final validation gate.

### User Story Dependencies

```text
Foundational
    -> US1: Build a Download Plan (MVP)
        -> US2: Download the Planned Artifacts
            -> US3: Reuse Artifacts and Report Progress
                -> US4: Recover Calmly from Failure
                -> US5: Automate Explicit Selections
                    -> Polish
```

US4 and US5 can be developed in parallel after US3 if separate contributors work on the failure
policy and automation path, but both must use the completed US1 plan model and US2 executor.

### Within Each User Story

- Write the story's tests first and confirm they fail for the missing behavior.
- Implement pure models and validators before interactive or execution wiring.
- Keep all registry, filesystem, checksum, cache, persistence, and binary-resolution decisions in
  the SDK.
- Complete the story checkpoint before treating the next story as independently deliverable.

## Parallel Execution Examples

### Foundation

```text
T003: Define CLI errors and exit mapping in crates/cli/src/error.rs and crates/cli/src/main.rs
T004: Implement home and terminal context in crates/cli/src/context.rs
T005: Write SDK cancellation tests in crates/sdk/tests/download_flow.rs and failure_paths.rs
```

These tasks touch separate files and can begin after T001/T002. T006 must wait for the cancellation
tests and T007 must wait for the SDK contract.

### User Story 1

```text
T008: Catalog and identifier tests in crates/cli/src/commands/download.rs
T009: Explicit mapping tests in crates/cli/src/commands/download.rs
T010: Runtime selection and plan tests in crates/cli/src/commands/download.rs
T011: Command-surface tests in crates/cli/tests/command_surface.rs
```

T011 can run in parallel with the command-module tests because it uses the executable boundary;
T008–T010 should be sequenced if they share the same test module to avoid edit conflicts.

### User Story 3

```text
T022: SDK disposition mapping tests in crates/cli/src/commands/download.rs
T023: Progress normalization tests in crates/cli/src/commands/download.rs
T024: Terminal rendering tests in crates/cli/src/output/human.rs
```

T024 is independently parallelizable with the command-module tests. T025 follows the renderer
tests, while T026 follows the executor and progress test contracts.

### User Story 4 and User Story 5

```text
T028/T029/T030: Failure-policy tests in crates/cli/src/commands/download.rs and output/human.rs
T031: SDK/CLI cancellation tests in crates/sdk/tests/download_flow.rs and commands/download.rs
T035: CLI parser and command-surface automation tests in cli.rs and tests/command_surface.rs
T036: Explicit-plan equivalence tests in commands/download.rs
```

T031 and T035 can run in parallel with the other story tests because they cover separate boundary
files; implementation tasks remain ordered behind their respective failing tests.

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 Setup.
2. Complete Phase 2 Foundational, including the SDK cancellation contract even though transfer
   execution is delivered later.
3. Complete Phase 3 User Story 1.
4. Stop and validate keyboard selection, compatibility, review, confirmation gating, and cancel
   behavior using the independent test criteria.

The MVP is a safe, reviewable plan builder. It does not claim artifacts are available until the
download stories are implemented.

### Incremental Delivery

1. Add US1 to build and confirm plans.
2. Add US2 to execute all planned SDK operations.
3. Add US3 to expose intelligent reuse and truthful progress.
4. Add US4 to make partial failure and transfer cancellation safe and actionable.
5. Add US5 to make the same behavior deterministic for automation.
6. Run Polish and the complete workspace gates.

### Parallel Team Strategy

1. Complete Setup and Foundational together.
2. Assign one contributor to US1 planning and another to SDK cancellation tests/implementation only
   after the shared module skeleton is stable.
3. Once US2 and US3 boundaries are stable, assign US4 failure handling and US5 automation in
   parallel, avoiding simultaneous edits to the same command-module sections.
4. Integrate and run the final quality gates from T041.

## Notes

- `[P]` means the task can be performed in parallel without depending on incomplete work in the
  same files.
- `[US1]` through `[US5]` map tasks to the corresponding specification user stories.
- Every task names the concrete file or files it changes or tests.
- The CLI must never calculate SHA-256, compare artifact sizes, inspect cache paths, remove invalid
  artifacts, mutate SQLite, or invoke Firecracker directly.
- No migration or SQLite schema task is present because the existing SDK database is the sole
  inventory owner for this feature.
