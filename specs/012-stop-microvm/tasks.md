---

description: "Actionable task list for stopping a running MicroVM"

---

# Tasks: Stop Running MicroVM

**Input**: Design documents from `/specs/012-stop-microvm/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/sdk-stop.md](./contracts/sdk-stop.md), and
[quickstart.md](./quickstart.md)

**Tests**: Included because the constitution requires tests for domain rules, state transitions,
typed errors, and failure paths, and the plan's test design mandates lifecycle, idempotency,
forced-escalation, PID-reuse, race, concurrency, failure-path, and silence coverage.

**Organization**: Tasks are grouped by the three user stories (two P1, one P2). Shared domain and
runtime-port work is completed before story-specific implementation. No migration, no CLI, and no
other lifecycle operation are part of these tasks.

## Path Conventions

- SDK production code: `crates/sdk/src/`
- SDK integration and public-contract tests: `crates/sdk/tests/`
- Feature design documents: `specs/012-stop-microvm/`
- The CLI and files from `main` are not modified by these tasks.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish the test doubles and scaffolding the stop work builds on.

- [ ] T001 Extend the manager unit-test fakes with a controllable stop double (shutdown-delivery verdict per socket, wait-for-stop verdict sequence, process-reference verdict, terminate-record list) in `crates/sdk/src/manager.rs`.
- [ ] T002 [P] Add a running/stopped VM fixture builder (record with volume/socket paths, persisted runtime with `process_id`/`process_state`, complete network plus credential rows) reusable by all stop tests in `crates/sdk/src/manager.rs`.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core types and port boundaries that MUST be complete before ANY user story can be implemented.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T003 [P] Add the `MicroVmStopResult` type with the constraint "`state` is always `MicroVmState::Stopped` on success; `socket_path` is always inside the volume and silent at return; `forced` is `true` only when SIGKILL was delivered" in `crates/sdk/src/domain/microvm.rs`, re-export it from `crates/sdk/src/domain/mod.rs`, and re-export it from `crates/sdk/src/lib.rs` with Rustdoc.
- [ ] T004 [P] Extend the crate-internal `RuntimeController` in `crates/sdk/src/ports/runtime.rs` with `request_shutdown` (HTTP 204 delivered / already-silent / delivery-error) and `wait_for_stop` (200 ms poll until socket silence plus recorded-process exit, `None` PID waits on socket alone) operations, keeping `validate_host`, `verify_stopped`, `process_references_vm`, `socket_answers`, `launch_detached`, and `wait_for_socket` unchanged, and updating the `terminate_spawned` docs to cover the stop SIGKILL escalation path.
- [ ] T005 Update the existing test runtime doubles to implement the extended `RuntimeController` in `crates/sdk/src/manager.rs` test module.

**Checkpoint**: The SDK has the public result type, the extended runtime port, and controllable doubles. User-story work may begin.

---

## Phase 3: User Story 1 - Gracefully stop a running machine (Priority: P1) 🎯 MVP

**Goal**: Stop one running VM by name with a single graceful shutdown request through its control socket, observe the exit within the 60-second bound, persist the stopped runtime row, and return `Stopped` with `forced: false`.

**Independent Test**: With a running VM fixture (answering socket, referencing process), call `stop_microvm(name)` with a cooperative guest and verify `Stopped` state, a silent volume-local socket, settled runtime refs, `forced: false`, and no session held open.

### Tests for User Story 1

- [ ] T006 [P] [US1] Add public-contract coverage for `MicroVmStopResult`, `MicroVmState::Stopped`, volume-local silent socket placement, and `forced: false` on the graceful path in `crates/sdk/tests/public_api.rs`.
- [ ] T007 [P] [US1] Add deterministic preflight coverage for name validation with the constraint "1–64 ASCII characters; first character alphanumeric; remaining alphanumeric, `-`, or `_`", `NotFound` on unknown names, `LifecycleConflict` on incomplete creation, and no host mutation on preflight failure in `crates/sdk/src/manager.rs` unit tests.

### Implementation for User Story 1

- [ ] T008 [P] [US1] Implement the `FirecrackerRuntime` graceful shutdown delivery (`PUT /actions` with `{ "action_type": "SendCtrlAltDel" }` over UDS, bounded read/write timeouts, HTTP 204 delivered / already-silent / delivery-error) in `crates/sdk/src/adapters/runtime/firecracker.rs`.
- [ ] T009 [P] [US1] Implement the `FirecrackerRuntime` exit wait poll (200 ms interval, exit means socket silence AND recorded process no longer referencing the VM, `None` PID waits on socket silence alone) in `crates/sdk/src/adapters/runtime/firecracker.rs`.
- [ ] T010 [US1] Implement the stop coordinator graceful path in `crates/sdk/src/manager.rs`: `validate_vm_name`, per-name plus volume lock pair, `find_microvm`, `NotFound`, `require_complete`, socket-decided liveness, one `request_shutdown` call, 60-second `wait_for_stop`, owned stale-socket removal only at exactly the persisted socket path after proving no listener, stopped commit via `reset_runtime_to_stopped`, return `Stopped` with `forced: false` (depends on T008, T009).
- [ ] T011 [US1] Add deterministic US1 lifecycle coverage for graceful delivery, exit within the bound, silent socket and settled refs afterwards, and operation silence in `crates/sdk/src/manager.rs` unit tests and `crates/sdk/tests/lifecycle.rs`.

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently: a cooperative running VM stops gracefully and reports `forced: false`.

---

## Phase 4: User Story 2 - Stopping an already-stopped machine succeeds (Priority: P1)

**Goal**: Make stop idempotent: a silent socket returns success with `forced: false` without shutdown, signal, or host change (beyond an optional ref-settling write), including the delivery-race where the socket goes silent before the request lands.

**Independent Test**: Call `stop_microvm(name)` against stopped, never-started, and externally-killed rows; every call returns `Stopped` with `forced: false`, sends no shutdown, signals no process, and concurrent same-name stops yield one shutdown sequence with one consistent stopped result.

### Tests for User Story 2

- [ ] T012 [P] [US2] Add idempotency coverage across stopped, never-started, and externally-killed rows (success `Stopped` with `forced: false`, at most a ref-settling write, no shutdown request, no process signal) in `crates/sdk/src/manager.rs` unit tests.
- [ ] T013 [P] [US2] Add delivery-race coverage (socket silent at delivery time re-verifies to success with `forced: false`) in `crates/sdk/src/manager.rs` unit tests.

### Implementation for User Story 2

- [ ] T014 [US2] Wire the already-stopped early return in `crates/sdk/src/manager.rs`: silent socket settles refs via `clear_stale_runtime` when needed, removes the owned stale socket only at exactly the persisted socket path after proving no listener, and returns `Stopped` with `forced: false` with no shutdown and no signal.
- [ ] T015 [US2] Add concurrency coverage proving concurrent same-name stops run one shutdown sequence with one consistent stopped result through the held lock pair in `crates/sdk/src/manager.rs` unit tests.
- [ ] T016 [US2] Add failure-path coverage for preflight errors (unknown, malformed, and incomplete-record names return typed errors with no host mutation and operation silence) in `crates/sdk/tests/failure_paths.rs`.

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently: graceful stop and idempotent already-stopped both return the correct `forced` flag with no stray host effects.

---

## Phase 5: User Story 3 - Unresponsive machine is forced to stop (Priority: P2)

**Goal**: Escalate a machine that stays running after the 60-second wait to immediate SIGKILL of the re-verified recorded PID, re-verify the stop, and report `forced: true`; every unforceable or still-running outcome is a typed error that never claims stopped.

**Independent Test**: With a running VM whose guest ignores the shutdown request, call `stop_microvm(name)` and verify the machine reaches stopped with `forced: true`; repeat with no usable PID, a recycled PID, an undeliverable request, and a kill-resistant process to verify typed errors with no false stopped claims and no unrelated process signaled.

### Tests for User Story 3

- [ ] T017 [P] [US3] Add escalation coverage (60-second expiry, SIGKILL to the re-verified PID, success with `forced: true`, natural-exit race between expiry and SIGKILL returns `forced: false`) in `crates/sdk/src/manager.rs` unit tests.
- [ ] T018 [P] [US3] Add failure-path coverage for undeliverable graceful request while answering (typed error, no signal, no state write), unforceable still-running machine with no usable PID (typed error, never success), PID-reuse safety (unrelated process never signaled), and still-running after SIGKILL re-verify (typed error, never claims stopped) in `crates/sdk/tests/failure_paths.rs`.

### Implementation for User Story 3

- [ ] T019 [US3] Wire forced escalation in `crates/sdk/src/manager.rs`: on 60-second expiry with the machine still running require a usable recorded PID, re-verify it with `process_references_vm`, deliver SIGKILL via `terminate_spawned`, re-verify with `wait_for_stop` up to 10 seconds, commit the stopped row on success and return `Stopped` with `forced: true`, otherwise return the typed error without claiming stopped.
- [ ] T020 [US3] Add deterministic US3 lifecycle coverage for forced stop end-to-end (silent socket, settled refs, `forced: true` observable from the result alone) in `crates/sdk/src/manager.rs` unit tests and `crates/sdk/tests/lifecycle.rs`.

**Checkpoint**: All user stories should now be independently functional: graceful, idempotent, and forced stops each behave per the liveness decision table with PID-reuse protection intact.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Contract consistency, documentation, and quality gates across all stories.

- [ ] T021 [P] Add Rustdoc for `stop_microvm` and `MicroVmStopResult` covering the name-only input, socket-decided liveness, `SendCtrlAltDel` graceful path, 60-second wait, SIGKILL-only escalation, the `forced` flag meaning, and volume-local silent socket placement in `crates/sdk/src/manager.rs` and `crates/sdk/src/domain/microvm.rs`.
- [ ] T022 [P] Verify no CLI source changes and no new public error variant were introduced, and that all repository text is English, by reviewing the final diff.
- [ ] T023 Run `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` and fix all findings.
- [ ] T024 Run the `quickstart.md` validation scenarios (graceful stop by name, already-stopped repeat, forced escalation, failure handling) against the implemented SDK.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - User stories can then proceed in parallel (if staffed)
  - Or sequentially in priority order (US1 → US2 → US3)
- **Polish (Final Phase)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 2 (P1)**: Can start after Foundational (Phase 2) - Builds on the US1 coordinator's liveness and lock wiring (T014 extends T010); independently testable via doubles
- **User Story 3 (P2)**: Can start after Foundational (Phase 2) - Builds on the US1 coordinator's shutdown/wait wiring (T019 extends T010); independently testable via doubles

### Within Each User Story

- Tests (included per constitution) are written alongside implementation using deterministic doubles
- Domain/port work (Phase 2) before coordinator work (US1)
- Coordinator skeleton (T010) before idempotency wiring (T014) and escalation wiring (T019)
- Core implementation before failure-path and concurrency coverage
- Story complete before moving to next priority

### Parallel Opportunities

- All Setup tasks marked [P] can run in parallel
- T003 and T004 can run in parallel (different files: domain vs ports)
- T008 and T009 can run in parallel (different concerns in the same adapter file - coordinate edits)
- T006 and T007 can run in parallel (different test targets)
- T012 and T013 can run in parallel (different test coverage in the same module - coordinate edits)
- T017 and T018 can run in parallel (different test targets)
- Once Foundational phase completes, all user stories can start in parallel (if team capacity allows)
- T021 and T022 can run in parallel (docs vs diff review)

---

## Parallel Example: User Story 1

```bash
# Launch shutdown delivery and exit-wait work together (coordinate same-file edits):
Task: "Implement FirecrackerRuntime graceful shutdown delivery in crates/sdk/src/adapters/runtime/firecracker.rs"
Task: "Implement FirecrackerRuntime exit wait poll in crates/sdk/src/adapters/runtime/firecracker.rs"

# Launch contract and preflight tests together:
Task: "Add public-contract coverage in crates/sdk/tests/public_api.rs"
Task: "Add deterministic preflight coverage in crates/sdk/src/manager.rs unit tests"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently (cooperative VM stops gracefully, socket silent, refs settled, `forced: false`)
5. Deploy/demo if ready

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Test independently → Deploy/Demo (MVP!)
3. Add User Story 2 → Test independently → Deploy/Demo
4. Add User Story 3 → Test independently → Deploy/Demo
5. Each story adds value without breaking previous stories

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: User Story 1 (shutdown delivery, exit wait, coordinator graceful path)
   - Developer B: User Story 2 (idempotency, delivery race, concurrency)
   - Developer C: User Story 3 (forced escalation, unforceable/PID-reuse failures)
3. Stories complete and integrate independently

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Avoid: vague tasks, same file conflicts, cross-story dependencies that break independence
