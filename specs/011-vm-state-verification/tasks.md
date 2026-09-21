# Tasks: VM State Verification

**Input**: Design documents from `/specs/011-vm-state-verification/`

**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/sdk-state-verification.md, quickstart.md

**Tests**: Included — explicitly requested by the feature specification (SC-003 and SC-004 each state "verified by a test that …") and mandated by the constitution's quality gates. Per the task rules, test tasks come first within each story phase and must FAIL before the implementation they cover lands.

**Organization**: Tasks are grouped by user story. Each story is an independently testable increment with its own checkpoint.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Confirm a clean, green baseline before the breaking change lands.

- [x] T001 Record clean-tree baseline in `specs/011-vm-state-verification/` work (run `git status --short`, then `cargo check --workspace --all-targets --all-features`; proceed only from a clean tree and a passing check)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Persistence without the state column, two-variant vocabulary, completeness gates, shared verify core. No user-story work begins until this phase compiles and its migration test passes.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [x] T002 Create `crates/sdk/migrations/0004_drop_microvm_state.sql` with exactly `DROP INDEX IF EXISTS microvms_state;` then `ALTER TABLE microvms DROP COLUMN state;` (order mandatory), register it as version 4 appended to `MIGRATIONS` in `crates/sdk/src/adapters/persistence/migrations.rs`, and remove `"state"` from the `microvms` entry in `REQUIRED_TABLES` in the same file
- [x] T003 [P] Shrink `MicroVmState` in `crates/sdk/src/domain/lifecycle.rs` to `Running` and `Stopped` only (delete `Creating`, `Configured`, and `parse`; keep `as_str` for `running`/`stopped` used by `Display`, keep `Display`; rewrite enum Rustdoc to state the socket-governed meaning)
- [x] T004 [P] Update `crates/sdk/src/domain/microvm.rs`: delete the `state` field from `MicroVmRecord`; correct `MicroVmCreationResult.state` docs to always `Stopped`; correct `MicroVmStartResult.state` docs to always `Running`; remove the `state` field from `NetworkConfigurationResult` (pinned choice, not a constant); correct `MicroVmSummary.state` docs to "call-time verified state"
- [x] T005 Rewrite the repository trait in `crates/sdk/src/ports/repository.rs`: add `fn list_stored_microvms(&self) -> Result<Vec<StoredMicroVm>, SdkError>;` (all stored VMs ordered by name, children included), delete `update_state`, rename `insert_creating` to `insert_microvm`, and retire `list_microvm_names` after verifying no caller remains (callers today: only the two listings in `crates/sdk/src/manager.rs`)
- [x] T006 Rewrite persistence in `crates/sdk/src/adapters/persistence/sqlite.rs` (depends on T002, T005): drop `state` from the `find_microvm` SELECT, the list query, the insert (renamed to `insert_microvm`), and `record_from_row` (remove the `MicroVmState::parse` gate); delete the `update_state` impl; implement `list_stored_microvms` on one connection ordered by name reusing the existing per-row child loaders in a loop
- [x] T007 Rewrite lifecycle gates and commits in `crates/sdk/src/manager.rs` (depends on T003–T006): replace the `record.state` gates in `create_microvm` repeat, `configure_network`, and `start_microvm` with the completeness rule (a record is complete when all four rows exist: `microvms` + `vm_networks` + `vm_credentials` + `vm_runtime`; incomplete → `LifecycleConflict`); convert the four `update_state` commit sites to runtime-only persists; construct `MicroVmRecord` without `state`; return `Stopped` from creation paths; keep `build_start_result`, `clear_stale_runtime`, `remove_stale_socket`, `cleanup_failed_launch`, and `validate_persisted_files` logic unchanged minus the column
- [x] T008 Fix the `#[cfg(test)]` module in `crates/sdk/src/manager.rs` to compile (depends on T007): drop the `state` parameter from `start_fixture_vm`, remove its `update_state` call, and update existing asserts to the two-variant vocabulary without changing the behavior they pin
- [x] T009 [P] Delete the `Configured` post-check in `crates/cli/src/commands/new.rs` (creation `Ok` implies completeness; the `MicroVmState::Configured` match no longer compiles)
- [x] T010 [P] Update test fixtures in `crates/cli/src/output/human.rs` to the two-variant vocabulary (`Configured` construction becomes `Stopped`; `Running` stays)
- [x] T011 Add a v3→v4 migration test in `crates/sdk/tests/sqlite_persistence.rs` (depends on T002, T006): seed a database at schema version 3 with VM rows, open it through the SDK, assert the `state` column is gone (`SELECT state FROM microvms` fails with "no such column") and all other row data survives

**Checkpoint**: `cargo check --all-targets --all-features` passes; T011 passes. The tree compiles with no persisted state, but listings are not yet verified (that is US1).

---

## Phase 3: User Story 1 — Truthful single-machine state (Priority: P1) 🎯 MVP

**Goal**: Every state surface reports the call-time verified state: `Running` iff the VM's volume-local control socket answers, else `Stopped`. Killed, crashed, or never-started machines report stopped; genuinely live machines report running.

**Independent Test**: Kill a running machine's process outside the CLI, then check its state through the SDK/CLI — it reports `Stopped`, and `start` proceeds as for a stopped machine (spec acceptance scenarios 1–4, SC-001, SC-003, SC-004).

### Tests for User Story 1

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation (T013–T015).**

- [x] T012 [P] [US1] Rewrite `crates/sdk/tests/microvm_listing.rs` with a stateless raw-SQL seeder (no `state` column) plus real `UnixListener` fixtures: answering-200 socket → `Running`, silent socket file → `Stopped`, missing socket → `Stopped`, live non-VM process on the recorded PID with silent socket → `Stopped` (SC-003), empty inventory → empty, name ordering preserved
- [x] T013 [P] [US1] Update export and variant pins in `crates/sdk/tests/public_api.rs` (`MicroVmState::Stopped` presence, `Creating`/`Configured` absence, `MicroVmSummary` export, `MicroVmStartResult` running-state reference)
- [x] T014 [P] [US1] Extend the `#[cfg(test)]` module in `crates/sdk/src/manager.rs` with the socket-governs verdict matrix via the injected `TestRuntime` (socket answers + referencing process → running; socket answers + foreign/missing PID → running; silent socket + referencing process → stopped; silent + foreign → stopped; probe `Err` → stopped; no runtime record → stopped without probing)

### Implementation for User Story 1

- [x] T015 [US1] Implement the private verify core in `crates/sdk/src/manager.rs` (depends on T014): `verify_one` runs `process_references_vm` then `socket_answers` in one blocking closure and returns the verdict plus process evidence; verdict rule is socket-only (answers → `Running`; silent, probe `Err`, timeout, or join failure → `Stopped`); rewire `is_vm_live` to the socket verdict; no writes, no locks, no new dependency
- [x] T016 [US1] Rewrite `list_microvms` in `crates/sdk/src/manager.rs` over `list_stored_microvms` (depends on T015): one entry per stored VM ordered by name with the verified state, sequential probing in this story (fan-out is US2), inventory failures stay typed errors, zero writes
- [x] T017 [US1] Rewrite `list_running_microvms` in `crates/sdk/src/manager.rs` over `list_stored_microvms` (depends on T015): return only socket-answering VMs ordered by name with stored SSH metadata verbatim (paths only); silent/failed probes are omitted, never errors; zero writes
- [x] T018 [P] [US1] Cover verified-state selector labels in `crates/cli/src/commands/start.rs` tests (`name [state]` renders the verified value)
- [x] T019 [P] [US1] Cover the `Running` pre-filter and `resolve_running_entry` error labels in `crates/cli/src/commands/ssh.rs` tests (filter is truthful; stopped machines yield `ssh_not_running` with the verified state string)

**Checkpoint**: US1 acceptance scenarios 1–4 pass; SC-001, SC-003, SC-004 verified by T012/T014; `cargo test -p taumaru-microvm --test microvm_listing` green. MVP shippable: single-machine answers are truthful (bulk is correct but still sequential — US2 makes it fast).

---

## Phase 4: User Story 2 — Fast bulk listing over many machines (Priority: P2)

**Goal**: Bulk verification runs concurrently under a CPU-scaled bound so ~100 machines list in under 10s; one slow or failing probe resolves to `Stopped` without stalling or aborting the listing.

**Independent Test**: Seed ~100 VMs in mixed states and time a full listing — every entry correct, total within SC-002; inject a hung probe and a failing probe — both resolve to `Stopped`, others unaffected (SC-002, SC-005).

### Tests for User Story 2

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation (T021–T022).**

- [x] T020 [P] [US2] Add bulk tests to the `#[cfg(test)]` module in `crates/sdk/src/manager.rs`: mixed-state bulk correctness with per-VM injected probe outcomes (extend `TestRuntime` with per-socket answers; KVM-free), failure isolation (one `Err` + one hang → both `Stopped`, rest correct, no panic), and timeout containment through the private timeout-parameterized helper with a short test bound (never wait the production 12s)
- [x] T021 [P] [US2] Add a bulk scale test in `crates/sdk/tests/microvm_listing.rs`: 8 never-responding listeners plus 92 silent VMs complete with all-correct verdicts in under 30s (sequential execution would exceed ~40s on the 5s read timeout alone; typical hosts finish in one ~5s wave)

### Implementation for User Story 2

- [x] T022 [US2] Implement the fan-out in `crates/sdk/src/manager.rs` and wire both listings through it (depends on T020–T021): permits `(std::thread::available_parallelism * 4).clamp(8, 32)` with a hardcoded fallback, `PROBE_TIMEOUT_SECS = 12`, one `spawn_blocking` per VM with cloned `Arc<dyn RuntimeController>` and owned `PathBuf`s wrapped in `tokio::time::timeout` and collected via `JoinSet` preserving name order; any per-VM outcome other than "socket answers" resolves to `Stopped`; reads take no `target_lock` and perform no writes; `wait_for_socket` stays out of read paths

**Checkpoint**: US2 acceptance scenarios pass; SC-002 and SC-005 verified by T020–T021. At this point US1 and US2 work together (verified + fast).

---

## Phase 5: User Story 3 — Safe start over a stale runtime reference (Priority: P2)

**Goal**: `start_microvm` branches on socket-governed liveness plus process evidence: live + referencing → idempotent return; live + missing/foreign PID → `TemporaryRuntime` refusal (no duplicate, no adoption, no signalling); silent → stale recovery and fresh launch.

**Independent Test**: With a stale runtime reference and no live process, `start` boots fresh; repeated start while live returns the same identity; with a foreign-live socket, `start` refuses instead of duplicating (quickstart scenarios 1 and 4).

### Tests for User Story 3

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation (T024–T025).**

- [x] T023 [P] [US3] Add start-branch tests to the `#[cfg(test)]` module in `crates/sdk/src/manager.rs`: idempotent live return (same PID, nothing launched), stale recovery (silent socket → fresh launch with new PID), foreign-live refusal (`TemporaryRuntime` with `stopped: false`, zero launches, recorded PID never signalled), incomplete record → `LifecycleConflict`
- [x] T024 [P] [US3] Cover the refused-foreign path rendering in `crates/cli/src/commands/start.rs` tests (`TemporaryRuntime` maps to the calm `start_failed` error; no new copy)

### Implementation for User Story 3

- [x] T025 [US3] Implement socket-governed start branching in `crates/sdk/src/manager.rs` (depends on T023–T024 and the US1 verify core): replace the liveness fast path with the three-way branch (idempotent / refuse-foreign / stale-recovery-then-launch); keep `build_start_result`'s PID-equality check as the idempotent-path enforcement; keep locks, network reconciliation, stale-socket removal, and launch-failure cleanup unchanged

**Checkpoint**: All three user stories functional; quickstart scenarios 1 and 4 verified; `ssh`/`start` pre-steps behave for stopped machines.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Compatibility documentation, end-to-end validation, full gates.

- [x] T026 Verify breaking-change documentation renders correctly: Rustdoc on `MicroVmState`, `MicroVmSummary.state`, and both result types states the new invariants (`cargo doc -p taumaru-microvm --no-deps` builds without warnings); all user-facing text English; CLI changes keep calm `running`/`stopped` labels with no color-only signaling
- [x] T027 Run the `specs/011-vm-state-verification/quickstart.md` validation scenarios 1–6 end to end and record outcomes (killed-machine listing, stale file, recycled PID, foreign-live refusal, bulk timing, column-removal SQL check)
- [x] T028 Run the full repository gates and fix all fallout: `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — starts immediately.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories. T005 precedes T006; T007–T008 follow T003–T006; T009–T010 need only T003–T004; T011 needs T002 + T006.
- **User Stories (Phases 3–5)**: All depend on Foundational completion.
  - US1 first (MVP; owns the verify core T015 that US2/US3 build on).
  - US2 depends on US1 (fan-out wires the US1 listings through the shared helper).
  - US3 depends on US1 (start branching consumes US1 process evidence); US3 touches `crates/sdk/src/manager.rs` like US2, so serialize US2 → US3 manager edits or coordinate explicitly — do not parallelize T022 and T025.
- **Polish (Phase 6)**: Depends on all desired user stories being complete.

### User Story Dependencies

- **User Story 1 (P1)**: After Foundational only. No dependencies on other stories.
- **User Story 2 (P2)**: After US1 (needs `verify_one` + verified listings to parallelize).
- **User Story 3 (P2)**: After US1 (needs verify evidence); sequence after US2 for same-file safety.

### Within Each User Story

- Tests FIRST and FAIL before implementation (T012–T014 before T015–T017; T020–T021 before T022; T023–T024 before T025).
- Shared helper before callers (T015 before T016–T017; fan-out T022 before relying on its timing in T021's scale run — write T021 first, run it green after T022).
- Story checkpoint green before the next story starts.

### Parallel Opportunities

- T003 + T004 (different domain files, no mutual dependency).
- T009 + T010 (different CLI files).
- T012 + T013 + T014 (different test files/scopes; T014's matrix asserts the T015 contract, written first).
- T018 + T019 (different CLI test modules).
- T020 + T021 (different test files).
- T023 + T024 (different test scopes).
- US2 test-writing may overlap US3 test-writing once US1 is green (different test blocks), but T022 and T025 implementation must serialize on `crates/sdk/src/manager.rs`.

---

## Parallel Example: User Story 1

```bash
# Launch all US1 tests together (fail-first):
Task: "Rewrite microvm_listing.rs with stateless seeder + UnixListener fixtures in crates/sdk/tests/microvm_listing.rs"
Task: "Update export and variant pins in crates/sdk/tests/public_api.rs"
Task: "Extend manager test module with socket-governs verdict matrix in crates/sdk/src/manager.rs"

# Then implementation, shared helper first:
Task: "Implement verify core in crates/sdk/src/manager.rs"
# Then in parallel with each other (same file — serialize edits, or split author/reviewer):
Task: "Rewrite list_microvms over bulk load in crates/sdk/src/manager.rs"
Task: "Rewrite list_running_microvms over bulk load in crates/sdk/src/manager.rs"

# CLI test modules together:
Task: "Cover selector labels in crates/cli/src/commands/start.rs tests"
Task: "Cover running-filter + error labels in crates/cli/src/commands/ssh.rs tests"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001).
2. Complete Phase 2: Foundational (T002–T011) — migration, vocabulary, completeness gates compile green.
3. Complete Phase 3: User Story 1 (T012–T019).
4. **STOP and VALIDATE**: kill a live VM's process out-of-band, confirm every surface reports `Stopped` and `start` recovers; run `cargo test -p taumaru-microvm --test microvm_listing`.
5. Shippable MVP: truthful single-machine state (bulk correct, sequential).

### Incremental Delivery

1. Setup + Foundational → persistence without state, gates on completeness, green `cargo check`.
2. Add US1 → verified listings → validate independently (MVP).
3. Add US2 → concurrent fan-out → validate SC-002 timing + SC-005 isolation.
4. Add US3 → safe start branching → validate stale recovery + foreign refusal.
5. Polish → docs, quickstart run, full gates.

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together (T002–T011; T003/T004 and T009/T010 pair up).
2. Once Foundational is done: Developer A owns US1 (verify core + listings + tests).
3. After US1: Developer A takes US2 (fan-out), Developer B writes US3 tests (T023–T024) against the frozen US1 evidence contract, then implements T025 after T022 lands (serialize manager.rs).

---

## Notes

- [P] tasks = different files/scopes, no execution dependency (same-file [P] pairs still serialize edits).
- [Story] label maps each story-phase task to its user story for traceability.
- Each story phase is independently completable and testable per its checkpoint.
- Test tasks fail first: run the new test against unimplemented code, observe failure, then implement.
- Commit after each task or logical group; stop at any checkpoint to validate independently.
- Avoid: vague tasks, same-file parallel edits (T022 vs T025), cross-story behavior dependencies beyond the stated US1→US2/US3 order.
- No new dependency is introduced in any task; `tokio` `sync`/`rt`/`time`/`macros` features already enabled cover the fan-out.
- Verbatim contracts enforced in tasks: migration body order (T002), `(available_parallelism * 4).clamp(8, 32)` permits and 12s probe bound (T022), completeness = four rows present (T007), socket-answers-iff-running verdict (T015).
