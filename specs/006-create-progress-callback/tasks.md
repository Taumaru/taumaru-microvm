# Tasks: MicroVM Creation Progress Callback

**Input**: Design documents from `/specs/006-create-progress-callback/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: Included — each user story's spec defines an explicit Independent Test, and the constitution mandates unit, contract, and failure-path tests for SDK contract changes.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Baseline verification before touching the SDK contract

- [ ] T001 Verify baseline gates pass from the repository root (`cargo test -p taumaru-microvm --all-targets --all-features`) and enumerate all in-repo `create_microvm` call sites via grep over `crates/` for migration tracking

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Public progress types that MUST exist before any emission logic or story work

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T002 Add `CreationStage` (6 variants: `Validation`, `PrerequisiteResolution`, `VolumePreparation`, `CredentialSetup`, `NetworkConfiguration`, `Finalization`), `CreationOutcome` (`Completed`, `AlreadyConfigured`, `Failed { stage }`), `CreationProgress` (`stage: CreationStage`, `completed_steps: u64`, `total_steps: u64`, `bytes_completed: Option<u64>`, `expected_bytes: Option<u64>`, `outcome: Option<CreationOutcome>`), and `TOTAL_CREATION_STEPS: u64 = 6` in `crates/sdk/src/domain/microvm.rs` with Rustdoc, `Clone + Copy + Debug + Eq + PartialEq` derives, and `Display` plus `pub(crate) as_str`/`parse` following the `MicroVmState`/`NetworkMode` pattern in `crates/sdk/src/domain/lifecycle.rs`
- [ ] T003 Re-export `CreationStage`, `CreationOutcome`, `CreationProgress` (and `TOTAL_CREATION_STEPS` if a const re-export is used) from `crates/sdk/src/domain/mod.rs` and `crates/sdk/src/lib.rs` (depends on T002)
- [ ] T004 [P] Extend export assertions in `crates/sdk/tests/public_api.rs` to reference the three new types (e.g. `size_of::<CreationProgress>()`) so the public facade is contract-tested (depends on T003)

**Checkpoint**: Foundation ready - user story implementation can now begin in parallel

---

## Phase 3: User Story 1 - Observe Creation Stages Live (Priority: P1) 🎯 MVP

**Goal**: `create_microvm` accepts an optional observer and emits 6 ordered stage events plus one `completed` terminal on success

**Independent Test**: Run creation with a recording observer against fixture artifacts; assert 7 events in stage order ending in `completed`, and returned VM metadata field-identical to a no-observer run

### Tests for User Story 1

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [ ] T005 [US1] Add success-stream test in `crates/sdk/src/manager.rs` tests module asserting exactly 7 events (`Validation` 1/6 through `Finalization` 6/6 in order, then `completed` terminal at 6/6, `outcome.is_none()` on the first 6) using a recording closure over the injected fakes (`TestStorage`, `TestCredentials`, `TestNetwork`, `TestRuntime`, `TestArtifactSource`)

### Implementation for User Story 1

- [ ] T006 [US1] Change `create_microvm` signature in `crates/sdk/src/manager.rs` to `pub async fn create_microvm<F>(&self, request: CreateMicroVmRequest, on_progress: Option<F>) -> Result<MicroVmCreationResult, SdkError> where F: FnMut(CreationProgress) + Send`, add a private `emit` helper taking `Option<&mut F>`, and emit `Validation` 1/6 (after the existing-VM lookup returns `None`) and `PrerequisiteResolution` 2/6 (after `insert_creating` succeeds)
- [ ] T007 [US1] Thread `Option<&mut F>` into `create_claimed_microvm` in `crates/sdk/src/manager.rs` and emit `VolumePreparation` 3/6 (after `prepare_rootfs`, with byte fields set per US3 rule), `CredentialSetup` 4/6 (after `inject_public_key`), `NetworkConfiguration` 5/6 (after guest-config writes), `Finalization` 6/6 (after `update_state(Configured)`), then the `completed` terminal at 6/6 (depends on T006, same file — sequential)
- [ ] T008 [US1] Migrate every in-repo `create_microvm(...)` call site to the new signature in `crates/sdk/src/manager.rs` tests module (~9 sites, recording closure or `None::<fn(CreationProgress)>`) and in `crates/sdk/tests/public_api.rs` (1 site) (depends on T006)

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently (run T005 plus `cargo test -p taumaru-microvm --all-targets --all-features`)

---

## Phase 4: User Story 2 - Same Results Through the New Signature (Priority: P1)

**Goal**: Creation with no observer produces results, errors, idempotency, conflicts, and rollback identical to the old behavior

**Independent Test**: Run the full existing creation suite (success, idempotent repeat, conflict, failure paths) through the new signature with `None` and verify zero behavior differences apart from the new argument

### Implementation for User Story 2

- [ ] T009 [US2] Add no-observer equivalence test in `crates/sdk/src/manager.rs` tests module creating the same request twice (once with a recording observer, once with `None`) and asserting field-identical `MicroVmCreationResult` values
- [ ] T010 [US2] Verify all pre-existing creation scenarios in `crates/sdk/src/manager.rs` tests module (idempotent repeat, immutable-field conflict, injected runtime verification failure, LAN variants) pass with `None` attached and produce unchanged typed errors and rollback (no new code expected — verification task; fix any divergence found)

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently

---

## Phase 5: User Story 3 - Truthful Measurable Progress (Priority: P2)

**Goal**: Step counters monotonic and bounded; byte counters honest — `Some(disk, disk)` on exactly the volume event, `None` elsewhere

**Independent Test**: Record the event stream of a successful observed creation; assert step counters never decrease and never exceed 6, and byte totals match the requested `disk_size_bytes` only on the volume event

### Implementation for User Story 3

- [ ] T011 [US3] Add counter-truthfulness test in `crates/sdk/src/manager.rs` tests module asserting monotonic `completed_steps` 1–6 with `total_steps == 6` on every event, `bytes_completed == expected_bytes == Some(disk_size_bytes)` on exactly the `VolumePreparation` event, and `None`/`None` byte fields on all other 6 events (covers FR-005 byte rule and the "both Some or both None — never mixed" invariant from `specs/006-create-progress-callback/data-model.md`)
- [ ] T012 [P] [US3] Add `Display` label test in a new `#[cfg(test)]` module in `crates/sdk/src/domain/microvm.rs` asserting the six snake-case strings (`validation`, `prerequisite_resolution`, `volume_preparation`, `credential_setup`, `network_configuration`, `finalization`) from `specs/006-create-progress-callback/contracts/sdk-create-progress.md`

**Checkpoint**: All P1 stories plus truthful counters independently functional

---

## Phase 6: User Story 4 - Failure Remains Visible and Safe (Priority: P2)

**Goal**: Every failure path ends with exactly one `failed` terminal naming the failed stage; typed errors and rollback unchanged; idempotent/conflict paths emit terminal-only streams

**Independent Test**: Inject a prerequisite failure with an observer; verify stream ends with terminal `failed` naming the stage, typed error matches no-observer error, no partial VM remains

### Tests for User Story 4

- [ ] T013 [US4] Add failure-terminal tests in `crates/sdk/src/manager.rs` tests module covering: invalid request → single `failed` at `Validation` 0/6; missing prerequisite → finished-stage events plus single `failed` at `PrerequisiteResolution` 1/6 with unchanged typed error; idempotent repeat → single `already-configured` terminal 0/6 with no stage events and no host changes; conflicting settings → single `failed` at `Validation` 0/6 with unchanged conflict error

### Implementation for User Story 4

- [ ] T014 [US4] Wire terminal-`failed` emission at every stage error site in `crates/sdk/src/manager.rs` (match-emit-return replacing bare `?` where an emission is owed: validation/conflict/idempotent branch, prerequisites, volume, credentials, network, finalization), keeping `rollback_creation` emission-free and all `SdkError` variants and messages unchanged (depends on T013 test-first ordering; same file as T006/T007 — sequential)
- [ ] T015 [P] [US4] Create `crates/sdk/tests/creation_progress.rs` integration test proving the silent/typed-error surface with an observer attached (invalid request yields typed `InvalidRequest` with a single `failed` terminal; no stdout/stderr assertion possible — assert error type and stream shape only) using the fixture registry server pattern from `crates/sdk/tests/download_flow.rs`

**Checkpoint**: All user stories should now be independently functional

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Contract alignment, per-call isolation proof, and full gates

- [ ] T016 [P] Add per-call isolation test in `crates/sdk/src/manager.rs` tests module running two concurrent creations for different VM names with separate recording observers and asserting each stream contains only its own 7 events in order
- [ ] T017 Verify `specs/006-create-progress-callback/quickstart.md` snippets against the implemented API (observer call convention `Some(|event: CreationProgress| events.push(event))` and `None::<fn(CreationProgress)>` form both compile) and confirm no CLI source under `crates/cli/src/` was touched and no SQLite migration was added
- [ ] T018 Run full gates from the repository root (`cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`) and record results

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - User stories can then proceed in priority order (US1 → US2 → US3 → US4); US2/US3/US4 build on the US1 signature and emission points, so they run sequentially after US1
  - Within-story test tasks are written FIRST and must FAIL before the story's implementation tasks
- **Polish (Final Phase)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories; delivers the MVP (observable creation)
- **User Story 2 (P1)**: Depends on US1 signature change (T006) - Verifies no-observer equivalence through the new signature
- **User Story 3 (P2)**: Depends on US1 emissions (T006/T007) - Proves counter truthfulness of the emitted stream
- **User Story 4 (P2)**: Depends on US1 emissions (T006/T007) - Adds failure-terminal wiring and terminal-path tests

### Within Each User Story

- Tests MUST be written and FAIL before implementation (T005 before T006/T007; T013 before T014)
- Same-file `manager.rs` tasks (T006 → T007 → T014) are strictly sequential - never parallel
- Story complete (tests green) before moving to next priority

### Parallel Opportunities

- T004 (public_api.rs exports) can run in parallel with US1 manager.rs work (different files, after T003)
- T012 (Display labels) can run in parallel with T011 (different file or independent test block)
- T015 (new integration file `creation_progress.rs`) can run in parallel with T014 (different files)
- T016 (isolation test) can run in parallel with T017 (docs verification)
- All Polish [P] tasks can run in parallel with each other

---

## Parallel Example: User Story 4

```bash
# T014 (manager.rs failure wiring) and T015 (creation_progress.rs) touch different
# files and can run in parallel once T013 defines the expected terminals:
Task: "Wire terminal-failed emission at every stage error site in crates/sdk/src/manager.rs"
Task: "Create crates/sdk/tests/creation_progress.rs integration test with fixture registry server"
```

## Parallel Example: Cross-Story (after US1 complete)

```bash
# US3 counter tests and US4 terminal tests both live in the manager.rs tests module
# but in independent test functions - run implementation in sequence, verification in parallel:
Task: "Add counter-truthfulness test in crates/sdk/src/manager.rs tests module"
Task: "Verify quickstart.md snippets in specs/006-create-progress-callback/quickstart.md"
```

---

## Implementation Strategy

### MVP First (User Stories 1 + 2 Only)

1. Complete Phase 1: Setup (T001 baseline + call-site inventory)
2. Complete Phase 2: Foundational (T002–T004 public types + exports)
3. Complete Phase 3: User Story 1 (T005–T008 signature + success emissions + migration)
4. Complete Phase 4: User Story 2 (T009–T010 equivalence proof)
5. **STOP and VALIDATE**: Observed creation yields 7 ordered events; no-observer runs identical to old behavior; full gates green
6. Deploy/demo if ready (CLI can now build live progress on the stream)

### Incremental Delivery

1. Complete Setup + Foundational → public types available, nothing emits yet
2. Add US1 → observable success path (MVP core)
3. Add US2 → equivalence proof for the signature break (MVP complete)
4. Add US3 → counter-truthfulness proof (CLI can trust fractions)
5. Add US4 → failure/idempotent/conflict terminals (UI can stop indicators deterministically)
6. Polish → isolation proof, doc alignment, full gates

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together (T001–T004)
2. Developer A: US1 (T005–T008, owns `manager.rs` — exclusive write access)
3. Once US1 merges: Developer A continues US4 failure wiring (T013–T014, same file), Developer B takes US2/US3 tests (T009–T012) and T015 integration file
4. Stories integrate in priority order; `manager.rs` is never edited concurrently

---

## Notes

- [P] tasks = different files, no dependencies (T004 vs US1; T012 vs T011; T015 vs T014; T016 vs T017)
- [Story] label maps task to specific user story for traceability (US1–US4 from spec.md)
- `manager.rs` tasks are sequential by construction (T006 → T007 → T009/T010/T011/T013/T014/T016 all append to the same tests module or flow — coordinate, don't parallelize same-file edits)
- Spec's `specs/003-create-microvm/*` docs are historical records — never rewritten; only the new contract references them
- No SQLite migration, no new dependency, no CLI change, no `SdkError` variant in any task
- Commit after each task or logical group; stop at any checkpoint to validate the story independently
