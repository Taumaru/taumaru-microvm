---

description: "Actionable task list for starting a configured MicroVM"

---

# Tasks: Start Configured MicroVM

**Input**: Design documents from `/specs/008-start-microvm/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/sdk-start.md](./contracts/sdk-start.md), and
[quickstart.md](./quickstart.md)

**Tests**: Included because the constitution requires tests for domain rules, state transitions,
typed errors, and failure paths, and the plan's test design mandates lifecycle, idempotency,
stale-recovery, repair, concurrency, failure-path, and silence coverage.

**Organization**: Tasks are grouped by the three user stories (two P1, one P2). Shared domain and
runtime-port work is completed before story-specific implementation. No migration, no CLI, and no
other lifecycle operation are part of these tasks.

## Path Conventions

- SDK production code: `crates/sdk/src/`
- SDK integration and public-contract tests: `crates/sdk/tests/`
- Feature design documents: `specs/008-start-microvm/`
- The CLI and files from `main` are not modified by these tasks.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish the test doubles and scaffolding the start work builds on.

- [ ] T001 Extend the manager unit-test fakes with a controllable runtime double (recorded-process liveness verdict, socket-probe verdict, spawned-PID script, readiness verdict) in `crates/sdk/src/manager.rs`.
- [ ] T002 [P] Extend the manager unit-test fakes with a configurable network double (skipped/applied script per mode) in `crates/sdk/src/manager.rs`.
- [ ] T003 [P] Add a `Configured` VM fixture builder (record, persisted network both modes, credential, runtime refs) reusable by all start tests in `crates/sdk/src/manager.rs`.

---

## Phase 2: Foundational (Blocking Prerequisites)

- [ ] T004 [P] Add the `MicroVmStartResult` type with the constraint "`state` is always `MicroVmState::Running` on success; `socket_path` is always inside `volume_path`; `ssh` carries the private-key path, never key contents" in `crates/sdk/src/domain/microvm.rs`, re-export it from `crates/sdk/src/domain/mod.rs`, and re-export it from `crates/sdk/src/lib.rs` with Rustdoc.
- [ ] T005 [P] Extend the crate-internal `RuntimeController` in `crates/sdk/src/ports/runtime.rs` with process-liveness, control-socket probe, detached-launch, and bounded-readiness operations, keeping `validate_host` and `verify_stopped` unchanged.
- [ ] T006 Update the existing test runtime doubles to implement the extended `RuntimeController` in `crates/sdk/src/manager.rs` test module.
- [ ] T007 Add the start-specific persisted-data validation helper in `crates/sdk/src/manager.rs` enforcing "`volume_path` absolute real directory; `rootfs_path` equals `{volume_path}/rootfs.ext4`; `socket_path` equals `{volume_path}/firecracker.sock`; rootfs regular file of exactly `disk_size_bytes`; credential paths/modes/metadata match the contract", without reusing the creation assertion that the runtime is stopped and no socket file exists.
- [ ] T008 Add boot-artifact reverification against the artifact inventory (kernel path from `kernel_id`, Firecracker/`firectl` paths from package IDs, executable and host-compatible) in `crates/sdk/src/manager.rs`, returning `ArtifactPrerequisite` naming kind, registry ID, local path, and reason.

**Checkpoint**: The SDK has the public result type, the extended runtime port, start validation,
and controllable doubles. User-story work may begin.

---

## Phase 3: User Story 1 - Start a Stopped, Configured MicroVM (Priority: P1) 🎯 MVP

**Goal**: Start one stopped VM by name with its full persisted configuration as a detached
background process, persist `Running` with PID and volume-local socket after the control channel
answers, and return the running identity.

**Independent Test**: With a `Configured` VM fixture (valid volume, rootfs, keys, boot artifacts,
correct network), call `start_microvm(name)` and verify `Running` state, the socket inside the
volume answering, the recorded PID, the full persisted configuration applied, and the caller
regaining control immediately with no session held open.

### Tests for User Story 1

- [ ] T009 [P] [US1] Add public-contract coverage for `MicroVmStartResult`, `MicroVmState::Running`, volume-local socket placement, and private-key-path-only SSH metadata in `crates/sdk/tests/public_api.rs`.
- [ ] T010 [P] [US1] Add deterministic start lifecycle coverage for name validation, `NotFound` on unknown names, `Creating` rejection with `LifecycleConflict`, persisted-data validation errors, and no host mutation on preflight failure in `crates/sdk/src/manager.rs` unit tests.

### Implementation for User Story 1

- [ ] T011 [P] [US1] Implement the `FirecrackerRuntime` control-socket probe (`tokio::net::UnixStream` connect plus minimal `GET /machine-config` read; file existence alone never counts) in `crates/sdk/src/adapters/runtime/firecracker.rs`.
- [ ] T012 [P] [US1] Implement the `FirecrackerRuntime` process-liveness check (read `/proc/<pid>`, verify the command line still references this VM's socket or runtime binary path; mismatched or unreadable command line is not-live; never signal the process) in `crates/sdk/src/adapters/runtime/firecracker.rs`.
- [ ] T013 [US1] Implement the `firectl` argv builder from validated persisted values (Firecracker binary, kernel path, VM-local rootfs with `:rw`, vCPU count, checked effective memory MiB, persisted TAP/MAC, distribution `kernel_args` plus persisted `desired_boot_parameters`, volume-local socket) as typed argument vectors in `crates/sdk/src/adapters/runtime/firecracker.rs`.
- [ ] T014 [US1] Implement detached spawn (stdin null, stdout/stderr appended to a VM-local log file in the volume, new process group, `kill_on_drop(false)`, forgotten handle after PID capture, passive reaper only) plus the bounded socket-readiness poll in `crates/sdk/src/adapters/runtime/firecracker.rs`.
- [ ] T015 [US1] Implement the start coordinator in `crates/sdk/src/manager.rs`: `validate_vm_name`, per-name plus volume lock pair, `find_microvm`, `NotFound`, `Creating` gate with `LifecycleConflict` naming name/state/operation, start-specific validation, boot-artifact reverification, network reconciliation with `update_network`, launch, readiness, and the atomic `persist_runtime` plus `update_state(Running)` commit in one repository closure.
- [ ] T016 [US1] Implement launch-failure cleanup in `crates/sdk/src/manager.rs` (terminate only the just-spawned child, remove the owned socket only if this attempt created it, reset runtime refs to stopped without claiming `Running`, keep repaired network items persisted, return the typed launch error with no orphan process).
- [ ] T017 [US1] Add deterministic US1 lifecycle coverage for full-configuration launch, volume-local socket readiness, PID persistence, atomic `Running` commit, launch-failure cleanup with repaired network kept, and operation silence in `crates/sdk/src/manager.rs` unit tests and `crates/sdk/tests/lifecycle.rs`.

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently:
a stopped VM starts, stays running after return, and failures leave it stopped with no orphan.

---

## Phase 4: User Story 2 - Repeated Start Is Safe and Sees Reality (Priority: P2, P1-grade safety)

**Goal**: Make repeated and concurrent starts safe through live verification: genuinely running
returns the current identity with no second process; any process/socket mismatch is stale and
starts fresh; concurrent same-name calls yield one process and one observed identity.

**Independent Test**: Start a VM, start again while live (same identity, one process), kill the
process externally and start again (fresh process, `Running`), exercise each mismatch direction
and PID reuse (each treated as stale, never duplicated), and run concurrent same-name starts
(one process, one identity).

### Tests for User Story 2

- [ ] T018 [P] [US2] Add idempotency coverage (live process plus answering socket returns current identity, launches nothing, changes no persisted configuration) in `crates/sdk/src/manager.rs` unit tests.
- [ ] T019 [P] [US2] Add stale-recovery coverage for the liveness decision table (process-live/socket-silent, process-dead/socket-answering, PID reuse with foreign command line, orphaned socket file) in `crates/sdk/src/manager.rs` unit tests.

### Implementation for User Story 2

- [ ] T020 [US2] Wire live-check-first idempotency in `crates/sdk/src/manager.rs`: when both signals agree the VM is running, return the current running identity from persisted refs without launching, repairing, or mutating configuration.
- [ ] T021 [US2] Wire stale-ref recovery in `crates/sdk/src/manager.rs`: on any signal mismatch, reset runtime refs to stopped via one repository closure, remove the stale socket file only at exactly the persisted socket path after proving no listener, and continue with a fresh launch.
- [ ] T022 [US2] Add concurrency coverage proving concurrent same-name starts produce one process and one observed identity through the held lock pair in `crates/sdk/src/manager.rs` unit tests.
- [ ] T023 [US2] Add failure-path coverage for stale handling (never signal a previously recorded PID, never delete outside the persisted socket path, never claim `Running` on mismatch) in `crates/sdk/tests/failure_paths.rs`.

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently: start, repeat,
external kill, mismatch, PID reuse, and concurrency all resolve to exactly one live machine or a
clean fresh start.

---

## Phase 5: User Story 3 - Start Repairs Host Network State Lost Since Configuration (Priority: P2)

**Goal**: Repair host network items lost since configuration (for example after a host reboot)
during start: keep correct items, recreate only missing or stale ones, keep the persisted network
identity unchanged, and fail typed with the VM stopped when repair is impossible.

**Independent Test**: With a `Configured` VM, remove its host network items outside the SDK, call
`start_microvm(name)`, and verify only the missing/stale items are recreated, correct items are
reported skipped, the persisted mode/addresses are unchanged, and the machine reaches `Running`;
repeat for both host-only and LAN modes.

### Tests for User Story 3

- [ ] T024 [P] [US3] Add per-mode repair coverage (wiped host-only items recreated with identity unchanged, wiped LAN items recreated with identity unchanged, correct items skipped) in `crates/sdk/src/manager.rs` unit tests.

### Implementation for User Story 3

- [ ] T025 [US3] Wire persisted-mode validation plus `NetworkController::configure` reuse in `crates/sdk/src/manager.rs` (mode derived from `expose_on_lan` must match the persisted mode; persisted network passed as `existing`; host-only and LAN used-address lists loaded; reconciled attachment persisted with `update_network`), returning the typed `Network` error with no process launched when repair is impossible.
- [ ] T026 [US3] Add failure-path coverage for unrepairable network (VM stays `Configured`, no duplicate or half-started process, actionable typed error) in `crates/sdk/tests/failure_paths.rs`.

**Checkpoint**: All user stories should now be independently functional: start, safe repeat, and
repair-on-start all work per mode with identity preserved.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Contract consistency, documentation, and quality gates across all stories.

- [ ] T027 [P] Add Rustdoc for `start_microvm` and `MicroVmStartResult` covering the name-only input, volume-from-record rule, both-signals-agree liveness, socket-preferred control, PID fallback, and volume-local socket placement in `crates/sdk/src/manager.rs` and `crates/sdk/src/domain/microvm.rs`.
- [ ] T028 [P] Verify no CLI source changes and no new public error variant were introduced, and that all repository text is English, by reviewing the final diff.
- [ ] T029 Run `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` and fix all findings.
- [ ] T030 Run the `quickstart.md` validation scenarios (start by name, repeat while live, external-kill recovery, per-mode repair, failure handling) against the implemented SDK.

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
- **User Story 2 (P1-grade safety)**: Can start after Foundational (Phase 2) - Builds on the US1 coordinator's live-check wiring (T020/T021 extend T015); independently testable via doubles
- **User Story 3 (P2)**: Can start after Foundational (Phase 2) - Builds on the US1 coordinator's network wiring (T025 extends T015); independently testable per mode via doubles

### Within Each User Story

- Tests (included per constitution) are written alongside implementation using deterministic doubles
- Domain/port work (Phase 2) before coordinator work (US1)
- Coordinator skeleton (T015) before idempotency/staleness wiring (T020/T021) and repair wiring (T025)
- Core implementation before failure-path and concurrency coverage
- Story complete before moving to next priority

### Parallel Opportunities

- All Setup tasks marked [P] can run in parallel
- T004 and T005 can run in parallel (different files: domain vs ports)
- T011 and T012 can run in parallel (different concerns in the same adapter file - coordinate edits)
- T009 and T010 can run in parallel (different test targets)
- T018 and T019 can run in parallel (different test coverage in the same module - coordinate edits)
- Once Foundational phase completes, all user stories can start in parallel (if team capacity allows)
- T027 and T028 can run in parallel (docs vs diff review)

---

## Parallel Example: User Story 1

```bash
# Launch probe and liveness work together (coordinate same-file edits):
Task: "Implement FirecrackerRuntime control-socket probe in crates/sdk/src/adapters/runtime/firecracker.rs"
Task: "Implement FirecrackerRuntime process-liveness check in crates/sdk/src/adapters/runtime/firecracker.rs"

# Launch contract and validation tests together:
Task: "Add public-contract coverage in crates/sdk/tests/public_api.rs"
Task: "Add deterministic start lifecycle coverage in crates/sdk/src/manager.rs unit tests"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently (start reaches `Running`, socket answers in the volume, failures stay stopped with no orphan)
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
   - Developer A: User Story 1 (coordinator, launch, readiness, cleanup)
   - Developer B: User Story 2 (idempotency, stale recovery, concurrency)
   - Developer C: User Story 3 (repair wiring per mode)
3. Stories complete and integrate independently

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Avoid: vague tasks, same file conflicts, cross-story dependencies that break independence
