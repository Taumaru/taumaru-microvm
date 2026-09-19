# Tasks: Image Artifact Download

**Input**: Design documents from `/specs/005-image-artifact-download/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: Included per the constitution's quality gates (unit tests for domain rules and
typed errors, integration/contract tests for the SDK-to-CLI boundary, failure-path tests
proving no panic or unsolicited output, CLI tests for output and exit behavior) and the
spec's Independent Test criteria per story. This is not optional-test padding: every test
task below maps to a spec acceptance scenario or a constitution-mandated gate.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **SDK library**: `crates/sdk/src/`, SDK tests in `crates/sdk/tests/`
- **CLI binary**: `crates/cli/src/`, CLI executable tests in `crates/cli/tests/`
- Feature docs: `specs/005-image-artifact-download/`

---


**Purpose**: Baseline verification and fixture confirmation before any code changes.

- [x] T001 Verify clean baseline gates for the `Cargo.toml` workspace from repository root (`cargo fmt --all -- --check`, `cargo check --all-targets --all-features`)
- [x] T002 [P] Confirm dual-image fixture coverage in `crates/sdk/tests/fixtures/manifest.json` (`alpine-test-minimal` plus `alpine-test-debug` under `alpine-test-1.0`) with servable payload routes in `crates/sdk/tests/support/mod.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Additive single-image SDK operation that MUST be complete before ANY CLI user story work begins. Existing whole-distribution operations stay untouched.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [x] T003 Add `DownloadedDistributionImage { distribution, image, file }` struct with Rustdoc in `crates/sdk/src/domain/artifact.rs`
- [x] T004 Re-export `DownloadedDistributionImage` in `crates/sdk/src/domain/mod.rs` and `crates/sdk/src/lib.rs`
- [x] T005 Implement `download_distribution_image` plus `download_distribution_image_with_cancellation` in `crates/sdk/src/manager.rs`, reusing `distribution_image_member` and `download_member` with a single-member `ProgressTracker` (unknown distribution yields `NotFound { kind: "distribution" }`; image absent from that distribution yields `NotFound { kind: "distribution image" }`; cancellation removes the in-flight temporary file and returns `Cancelled`)
- [x] T006 [P] Add single-image SDK tests in `crates/sdk/tests/download_flow.rs` (requesting only `alpine-test-minimal` stores and verifies just that file with `Downloaded` disposition; repeat yields `SkippedExisting` with no new transfer; whole-distribution download still stores both images; unknown image yields typed `NotFound`; cancellation yields `Cancelled` with the partial file removed)
- [x] T007 [P] Extend type-export assertions in `crates/sdk/tests/public_api.rs` (`DownloadedDistributionImage` exported) and failure-path cases in `crates/sdk/tests/failure_paths.rs` (blank IDs rejected; unknown distribution vs wrong-distribution image distinguished)
- [x] T008 Run focused SDK gates for `crates/sdk/` from repository root (`cargo test -p taumaru-microvm --all-targets --all-features`)

**Checkpoint**: Foundation ready — `download_distribution_image` verified against fixtures; user story implementation can now begin.

---

## Phase 3: User Story 1 - Pick Images From One List (Priority: P1) 🎯 MVP

**Goal**: One image multi-select where every entry names its parent distribution; no
distribution-first step, no kernel choice; sorted review; renamed `artifacts download` entry.

**Independent Test**: Point at the deterministic fixture with several distributions of
several images each, use keyboard-only input to select images across distributions
(including two of one distribution), and verify the review shows only selected images with
resolved default kernels before any transfer.

- [x] T009 [US1] Nest download under an `artifacts` parent subcommand with `DownloadArgs { images: Vec<String>, non_interactive: bool }` in `crates/cli/src/cli.rs`, removing the `distributions`/`kernels` fields and documenting `--image DISTRIBUTION_ID=IMAGE_ID` (repeatable) in help text
- [x] T010 [US1] Route the nested `Command::Artifacts` dispatch to the download flow in `crates/cli/src/commands/mod.rs`
- [x] T011 [US1] Replace `DistributionSelection` with `ImageSelection { distribution, image, kernel, expected_bytes }` plus a flattened host-architecture-filtered image catalog sorted and deduplicated by `(distribution.id, image.id)` with automatic default-kernel resolution and no override path in `crates/cli/src/commands/download.rs` (constraint, quoted from data-model: "The pair `(distribution.id, image.id)` is the plan key; display names never identify a row."; "`kernel` is resolved, never chosen: it always equals `distribution.default_kernel` as published.")
- [x] T012 [US1] Replace the distribution `MultiSelect` plus per-distribution kernel `Select` prompts with a single image `MultiSelect` whose rows carry image identity, parent distribution, variant or capabilities, and size in `crates/cli/src/commands/download.rs`
- [x] T013 [P] [US1] Render per-image review rows ordered by distribution then image with parent distribution, resolved default kernel, runtime bundle, and selected-only byte totals in `crates/cli/src/output/human.rs`
- [x] T014 [US1] Cover catalog flattening, host filtering, `(distribution.id, image.id)` sort order, repeat-pair dedup to one entry, default-kernel resolution, and missing/incompatible-default rejection in the tests module of `crates/cli/src/commands/download.rs`
- [x] T015 [P] [US1] Assert the renamed `artifacts download` help path and rejection of removed `--distribution`/`--kernel` flags with guidance toward `--image` in `crates/cli/tests/command_surface.rs`

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently (selection through review with zero transfers started before confirmation).

---

## Phase 4: User Story 2 - Download Only Selected Images With Default Kernels (Priority: P1)

**Goal**: Confirmed plan fetches runtime bundle first, then each unique resolved default
kernel once, then exactly the selected images — never unselected siblings.

**Independent Test**: Confirm a fixture plan with two images of one distribution plus one
image of another, and verify every selected image is acquired and verified, no unselected
image is stored, and each unique default kernel is requested at most once.

- [x] T016 [US2] Replace `PlanMember::DistributionImages` with per-image `PlanMember::DistributionImage { distribution_id, image_id, expected_bytes }` and runtime-first, unique-kernels-second, images-last ordering with checked byte totals covering selected artifacts only in `crates/cli/src/commands/download.rs` (constraint, quoted from data-model: "Each image member maps to exactly one `download_distribution_image(distribution_id, image_id)` call.")
- [x] T017 [US2] Swap `ArtifactClient::download_distribution` for `download_distribution_image(distribution_id, image_id)` plus a per-image `VerifiedArtifact` variant holding `DownloadedDistributionImage` in `crates/cli/src/commands/download.rs`
- [x] T018 [US2] Execute images sorted by distribution then image independent of selection order, with per-image byte accounting and kernel-failure skipping only dependent images in `crates/cli/src/commands/download.rs`
- [x] T019 [P] [US2] Render `distribution/{distro}/{image}` progress labels and per-image summary groups in `crates/cli/src/output/human.rs`
- [x] T020 [US2] Cover sorted execution order, shared-default-kernel-once, unselected-images-untouched, and two-images-one-distribution plans with the `RecordingClient` fake in the tests module of `crates/cli/src/commands/download.rs`

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently (select, review, transfer exactly the selection with creation-satisfiable default kernels).

---

## Phase 5: User Story 3 - Reuse Artifacts and Report Progress (Priority: P1)

**Goal**: Repeated runs reuse verified files without re-transfer, repair invalid cache
through SDK behavior, and show truthful per-image progress with reuse distinctly labeled.

**Independent Test**: Run the same confirmed image plan twice, corrupt one cached image
between runs, and verify reused/repaired/failed statuses plus transfer counts.

- [x] T021 [US3] Verify aggregate plan accounting (`completed_plan_bytes` plus SDK counters via checked addition) and `Downloaded`/`Adopted`/`Already available` disposition passthrough with no progress-shape change in `crates/cli/src/commands/download.rs` (constraint, quoted from data-model: "Aggregate progress derives only from SDK counters plus plan metadata — never from elapsed time or assumed rate.")
- [x] T022 [US3] Cover verified-reuse without second transfer, missing/corrupt repair before readiness, and per-image identity in progress events in the tests module of `crates/cli/src/commands/download.rs`
- [x] T023 [P] [US3] Cover non-color and narrow-terminal status rendering (text/symbol state never color-only) in the tests module of `crates/cli/src/output/human.rs`

**Checkpoint**: All P1 stories independently functional (select, transfer, reuse, truthful progress).

---

## Phase 6: User Story 4 - Recover Calmly From Failure (Priority: P2)

**Goal**: Runtime failure stops dependents; kernel failure skips only dependent images;
independent groups continue; verified work is retained; interrupt returns `130` with a
cancelled/successful/failed summary.

**Independent Test**: Fail one image or kernel transfer in a fixture and verify exit
status, separated group summary, retained artifacts, retry guidance, and `130` with no
partial artifact on interrupt.

- [x] T024 [US4] Implement runtime-terminal rule, kernel-failure skips-only-dependents continuation, and the remaining-images cancellation tail (`append_cancelled_after_image`) in `crates/cli/src/commands/download.rs` (constraint, quoted from data-model: "`Succeeded` is possible only when every runtime package, every unique resolved default kernel, and every selected image returns a verified SDK result.")
- [x] T025 [P] [US4] Keep three-part failure messages (what happened, why preparation is incomplete, what to do next) and exit codes `0`/`1`/`130` in `crates/cli/src/error.rs`
- [x] T026 [US4] Cover runtime-terminal stop, kernel-failure dependent-skip with independent continuation, and `Ctrl-C` returning `130` with preserved groups and no published partial in the tests module of `crates/cli/src/commands/download.rs`

**Checkpoint**: Failure and cancellation behavior independently verified.

---

## Phase 7: User Story 5 - Automate Explicit Image Selections (Priority: P2)

**Goal**: Repeatable `--image DISTRIBUTION=IMAGE` selections build the same plan as the
interactive flow with identical validation, deterministic non-interactive output, and
rejection of removed flags.

**Independent Test**: Invoke non-interactively with explicit image selections spanning two
images of one distribution and verify no prompts, same plan semantics, and deterministic
status.

- [x] T027 [US5] Parse repeatable `--image DISTRIBUTION=IMAGE` values with scoped validation (distribution present and host-compatible; image published by its named distribution; repeats deduplicate, never error) in `crates/cli/src/commands/download.rs`
- [x] T028 [US5] Gate explicit versus interactive modes with non-TTY guidance naming the `--image` form and removed-flag migration help in `crates/cli/src/commands/download.rs`
- [x] T029 [P] [US5] Cover explicit multi-image plans, malformed/unavailable/mismatched scoped IDs starting no transfer, and parser-level rejection of `--distribution`/`--kernel` in `crates/cli/tests/command_surface.rs` and the parser tests of `crates/cli/src/cli.rs`

**Checkpoint**: All user stories independently functional over both interactive and scripted input.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Dead-code removal, contract conformance, and full-gate validation.

- [x] T030 Remove superseded distro-first code (`parse_kernel_mappings`, kernel `Select` flow, `DistributionSelection`, group-level `DistributionImages` member and its helpers) in `crates/cli/src/commands/download.rs`
- [x] T031 [P] Validate English-only user-facing strings and old-`download` to `artifacts download` migration guidance across `crates/cli/src/cli.rs`, `crates/cli/src/error.rs`, and `crates/cli/src/output/human.rs`
- [x] T032 Run the validation guide in `specs/005-image-artifact-download/quickstart.md` (interactive flow, explicit flow, failure checks) followed by full workspace gates from repository root (`cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately.
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories (CLI
  work needs the new SDK operation compiled and fixture-tested first).
- **User Stories (Phases 3–7)**: All depend on Foundational completion; run sequentially
  in priority order (US1 → US2 → US3 → US4 → US5) because Phases 3–6 and 7 share
  `crates/cli/src/commands/download.rs` and same-file edits must not run in parallel.
- **Polish (Phase 8)**: Depends on all user story phases being complete.

### User Story Dependencies

- **User Story 1 (P1)**: Starts after Foundational — no dependencies on other stories.
- **User Story 2 (P1)**: Depends on US1 plan/selection types (`ImageSelection`, sorted
  catalog) but is independently testable via the `RecordingClient` fake.
- **User Story 3 (P1)**: Depends on US2 execution loop; independently testable through
  disposition/progress assertions.
- **User Story 4 (P2)**: Depends on US2 execution loop; independently testable through
  failure-injection tests.
- **User Story 5 (P2)**: Depends on US1 catalog validation and US2 plan shape; independently
  testable through explicit-mode tests.

### Within Each User Story

- Tests live in the same module/file as the behavior they cover (repo convention:
  unit tests close to the module); write the test first, watch it FAIL, then implement.
- Plan/catalog types before prompts; prompts before review; review before execution.
- Core implementation before failure-continuation wiring.
- Story checkpoint validated before moving to the next priority.

### Parallel Opportunities

- T002 can run alongside any Setup/Foundational work (read-only fixture check).
- T006 and T007 can run in parallel (different SDK test files, same contract).
- T013 can run alongside T011/T012 (different file: `human.rs` vs `download.rs`).
- T015 can run alongside US1 implementation (different file: `command_surface.rs`).
- T019 can run alongside T016–T018 (different file: `human.rs` vs `download.rs`).
- T023 can run alongside T021–T022 (different file: `human.rs` tests vs `download.rs`).
- T025 can run alongside T024/T026 (different file: `error.rs` vs `download.rs`).
- T029 can run alongside T027/T028 (different file: `command_surface.rs`/`cli.rs` vs `download.rs`).
- T031 can run alongside T030 (different files).
- NEVER parallelize two tasks editing `crates/cli/src/commands/download.rs`
  simultaneously (T011/T012, T014, T016/T017/T018, T020, T021/T022, T024, T026, T027/T028, T030).

---

## Parallel Example: User Story 1

```bash
# T011+T012 sequential (same file), T013 in parallel with them (different file):
Task: "Replace DistributionSelection with ImageSelection and sorted catalog in crates/cli/src/commands/download.rs"   # T011, then T012 same file
Task: "Render per-image review rows in crates/cli/src/output/human.rs"                                                # T013 [P] alongside

# Tests across files in parallel:
Task: "Cover flatten/sort/dedup and default-kernel resolution in crates/cli/src/commands/download.rs tests module"    # T014
Task: "Assert renamed help path and removed-flag rejection in crates/cli/tests/command_surface.rs"                    # T015 [P] alongside T014
```

## Parallel Example: Foundational SDK

```bash
# Sequential (type dependency chain):
Task: "Add DownloadedDistributionImage struct in crates/sdk/src/domain/artifact.rs"                                    # T003
Task: "Re-export in crates/sdk/src/domain/mod.rs and crates/sdk/src/lib.rs"                                          # T004, after T003
Task: "Implement download_distribution_image pair in crates/sdk/src/manager.rs"                                       # T005, after T003

# Then in parallel (different test files, same contract):
Task: "Add single-image SDK tests in crates/sdk/tests/download_flow.rs"                                              # T006 [P]
Task: "Extend export and failure-path assertions in crates/sdk/tests/public_api.rs + failure_paths.rs"               # T007 [P]
```

---

## Implementation Strategy

### MVP First (User Story 1 + Foundational)

1. Complete Phase 1: Setup (baseline + fixture confirmation).
2. Complete Phase 2: Foundational (SDK single-image operation fixture-tested).
3. Complete Phase 3: User Story 1 (select + review under the renamed entry, no transfers yet wired per-image).
4. **STOP and VALIDATE**: selection, dedup, ordering, default-kernel review, and renamed help independently.
5. Deploy/demo if ready (review-only confidence before touching execution).

### Incremental Delivery

1. Setup + Foundational → SDK single-image ready, whole-distribution preserved.
2. Add US1 → select and review images (MVP!).
3. Add US2 → transfer exactly the selection (operational payoff).
4. Add US3 → reuse/repair display and truthful progress.
5. Add US4 → calm failure continuation and `130` cancellation.
6. Add US5 → scripted `--image` automation with removed-flag guidance.
7. Polish → dead-code removal, string audit, full gates.
8. Each increment adds value without breaking previous stories.

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together (T003→T004→T005 chain, then T006/T007
   split across two developers).
2. Single developer takes Phases 3–7 sequentially (shared `download.rs` forbids
   splitting by story across developers without merge conflicts).
3. A second developer can own `human.rs` tasks (T013, T019, T023), `error.rs` (T025),
   and `command_surface.rs`/`cli.rs` tasks (T015, T029, T031) in parallel with the
   `download.rs` owner, integrating at each story checkpoint.

---

## Notes

- [P] tasks = different files, no same-file conflicts.
- [Story] label maps each task to its spec user story for traceability.
- Each user story is independently completable and testable via its Independent Test.
- Verify tests fail before implementing (TDD per repo convention).
- Commit after each task or logical group.
- Stop at any checkpoint to validate the story independently.
- Avoid: vague tasks, same-file parallel edits, cross-story dependencies that break independence.
- Data-model constraints quoted verbatim in T011, T016, T021, T024 — implementation must not reinterpret them.
- Constitution trace: T003–T008 (Principles I–IV: SDK-first, silent/typed, explicit state, additive-only); T009–T032 (Principle V: calm/accessible CLI, English-only strings in T031); T032 (quality gates: fmt, check, clippy `-D warnings`, full test suite).
