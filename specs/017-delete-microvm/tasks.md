---

description: "Actionable task list for deleting a MicroVM"

---

# Tasks: Delete MicroVM

**Input**: Design documents from `/specs/017-delete-microvm/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/sdk-delete.md](./contracts/sdk-delete.md), and
[quickstart.md](./quickstart.md)

**Tests**: Included because the constitution requires tests for domain rules, state transitions,
typed errors, and failure paths, and the plan's test design mandates lifecycle, refusal,
retry, scoping, concurrency, failure-path, and silence coverage.

**Organization**: Tasks are grouped by the three user stories (two P1, one P2). Shared domain and
port work is completed before story-specific implementation. No migration, no CLI, and no
other lifecycle operation are part of these tasks.

## Path Conventions

- SDK production code: `crates/sdk/src/`
- SDK integration and public-contract tests: `crates/sdk/tests/`
- Feature design documents: `specs/017-delete-microvm/`
- The CLI and files from `main` are not modified by these tasks.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish the test doubles and fixture variations the delete work builds on.

- [X] T001 Extend the manager unit-test `TestNetwork` fake with a scriptable `cleanup_for_delete` double (per-test outcome plus call counter, mirroring the existing `cleanup` fake) in `crates/sdk/src/manager.rs`.
- [X] T002 Add delete fixture variations reusable by all delete tests (incomplete record with missing child rows via `delete_child_rows`, partially removed volume, persisted volume outside the SDK home) in `crates/sdk/src/manager.rs`.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core types and port boundaries that MUST be complete before ANY user story can be implemented.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [X] T003 [P] Add the `MicroVmDeleteResult` type with the constraint "`name` is the stable identifier of the deleted machine; on success the record is gone, the whole volume directory is gone, owned host network items are released, shared artifacts are intact" in `crates/sdk/src/domain/microvm.rs`, re-export it from `crates/sdk/src/domain/mod.rs`, and re-export it from `crates/sdk/src/lib.rs` with Rustdoc.
- [X] T004 [P] Extend the crate-internal `NetworkController` in `crates/sdk/src/ports/network.rs` with `cleanup_for_delete` (release exactly the owned items, skip already-absent items as removed, typed error for present-but-unreleasable items), keeping `detect_uplink`, `select_lan_offer`, `configure`, and strict `cleanup` unchanged.
- [X] T005 Implement the scripted `cleanup_for_delete` on the manager unit-test `TestNetwork` fake in `crates/sdk/src/manager.rs` (depends on T004).

**Checkpoint**: The SDK has the public result type, the extended network port, and a controllable double. User-story work may begin.

---

## Phase 3: User Story 1 - Delete a stopped machine and all its owned traces (Priority: P1) 🎯 MVP

**Goal**: Delete one stopped VM by name with host-network release, whole volume-directory removal, and inventory-record deletion committed last, returning the deleted name; already-absent owned items converge and a repeat delete reports not-found.

**Independent Test**: With a stopped VM fixture (silent socket, complete rows), call `delete_microvm(name)` and verify the result carries the name, the name resolves to not-found, the whole volume directory is gone, owned network items are released, shared kernel/image rows are untouched, and a second delete returns `NotFound`.

### Tests for User Story 1

- [X] T006 [P] [US1] Add public-contract coverage for `MicroVmDeleteResult`, the deleted-name result value, and the re-export chain in `crates/sdk/tests/public_api.rs`.
- [X] T007 [P] [US1] Add deterministic preflight coverage for name validation with the constraint "1–64 ASCII characters; first character alphanumeric; remaining alphanumeric, `-`, or `_`", `NotFound` on unknown names including already-deleted names, and no host mutation on preflight failure in `crates/sdk/tests/public_api.rs` and `crates/sdk/tests/failure_paths.rs`.

### Implementation for User Story 1

- [X] T008 [US1] Implement the volume containment plus removal helper in `crates/sdk/src/manager.rs`: verify the volume is absolute, strictly below the SDK home, and not the home itself, verify `rootfs_path` is `{volume}/rootfs.ext4`, `socket_path` is `{volume}/firecracker.sock`, and present credential paths are `{volume}/ssh/id_ed25519` and `{volume}/ssh/id_ed25519.pub` (violations are `StorageConflict`/`Credential` with the record kept), then `remove_dir_all` with `NotFound` mapped to success and other I/O failures to `Filesystem` with the record kept (depends on T005).
- [X] T009 [US1] Implement the delete coordinator in `crates/sdk/src/manager.rs`: `validate_vm_name`, per-name plus volume lock pair, `find_microvm`, `NotFound`, no `require_complete` gate, socket probe `Ok(false)` proceeds, `cleanup_for_delete` with the in-memory network clone (skipped when no network row), volume helper from T008, record deletion via `delete_microvm(vm_id)`, return `MicroVmDeleteResult { name }` (depends on T008).
- [X] T010 [US1] Add deterministic US1 lifecycle coverage for stopped delete end-to-end (result name, unresolvable afterwards, whole volume gone, owned network released, shared kernel/image rows untouched), absent-owned-files convergence, incomplete-record delete, repeat-delete `NotFound`, and operation silence in `crates/sdk/src/manager.rs` unit tests (depends on T009).
- [X] T011 [US1] Add US1 failure-path coverage for containment refusal (typed error, nothing removed, record kept), volume-removal failure (typed error, record kept, retry succeeds), and unknown plus malformed names (typed errors, no host mutation) in `crates/sdk/tests/failure_paths.rs` and `crates/sdk/src/manager.rs` unit tests (depends on T009).

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently: a stopped VM deletes completely with a retryable failure contract.

---

## Phase 4: User Story 2 - Deleting a running machine is refused with a stop-first error (Priority: P1)

**Goal**: Refuse deletion of a running machine with a stop-first `LifecycleConflict` and zero host changes, propagate unprobable-socket probe errors without deleting, and serialize concurrent same-name deletes to one deletion sequence.

**Independent Test**: With a running VM fixture (answering socket), call `delete_microvm(name)` and verify the stop-first error with owned files, record, and network attachment unchanged; stop the machine, delete again, and verify success; concurrent same-name deletes yield one consistent outcome.

### Tests for User Story 2

- [X] T012 [P] [US2] Add refusal failure-path coverage (answering socket returns stop-first `LifecycleConflict` with byte-for-byte unchanged files, record, and attachment; stop-then-delete succeeds; unprobable probe error propagates with no deletion) in `crates/sdk/src/manager.rs` unit tests and `crates/sdk/tests/failure_paths.rs`.

### Implementation for User Story 2

- [X] T013 [US2] Wire the running-refusal plus probe-error branch in the delete coordinator in `crates/sdk/src/manager.rs`: probe `Ok(true)` returns `LifecycleConflict` with state `running` directing stop-before-delete before any host work, probe `Err` propagates unchanged without deleting, only `Ok(false)` reaches the US1 deletion steps (depends on T009).
- [X] T014 [US2] Add concurrency coverage proving concurrent same-name deletes run one deletion sequence with one consistent outcome through the held lock pair in `crates/sdk/src/manager.rs` unit tests (depends on T013).
- [X] T015 [US2] Add deterministic US2 lifecycle coverage for refusal-then-stop-then-delete end-to-end in `crates/sdk/src/manager.rs` unit tests (depends on T013).

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently: full stopped deletion and safe running refusal with no stray host effects.

---

## Phase 5: User Story 3 - Host network cleanup is scoped to exactly this machine (Priority: P2)

**Goal**: Release only the deleted VM's recorded owned host network items with absent-tolerance in the Linux adapter, keep the record on unreleasable-item failure for retry, and leave other VMs plus unowned host configuration untouched.

**Independent Test**: With two networked VM fixtures, delete one and verify its owned host resources are released while the surviving VM's connectivity, addresses, and routes are unchanged; repeat with a failing release and verify the record is kept and a retry succeeds.

### Implementation for User Story 3

- [X] T016 [P] [US3] Implement `cleanup_for_delete` on `LinuxNetworkController` in `crates/sdk/src/adapters/network/linux.rs`: reuse the ownership-scoped tap plus iptables removal core, add existence probes before host-route and proxy-neighbour removals so absent items skip instead of failing, keep sysctl restorations strict, leave the existing strict `cleanup` used by creation rollback unchanged.
- [X] T017 [US3] Add US3 coverage for present-but-unreleasable failure with record kept plus retry success, and multi-VM isolation with shared kernel plus image preservation in `crates/sdk/src/manager.rs` unit tests (depends on T009).

**Checkpoint**: All user stories should now be independently functional: full deletion, safe refusal, and scoped absent-tolerant network release each behave per the state machine with retry convergence intact.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Contract consistency, documentation, and quality gates across all stories.

- [X] T018 [P] Add Rustdoc for `delete_microvm` and `MicroVmDeleteResult` covering the name-only input, socket-decided liveness with stop-first refusal, network-first then volume then record ordering, whole-directory removal, absent-as-removed convergence, keep-record retry, and the deleted-name result in `crates/sdk/src/manager.rs` and `crates/sdk/src/domain/microvm.rs`.
- [X] T019 [P] Verify no CLI source changes and no new public error variant were introduced, and that all repository text is English, by reviewing the final diff.
- [X] T020 Run `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` and fix all findings.
- [X] T021 Run the `quickstart.md` validation scenarios (stopped delete by name, running refusal plus stop-then-delete, repeat-delete not-found, retry after failure, failure handling) against the implemented SDK in `specs/017-delete-microvm/quickstart.md`.

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
- **User Story 2 (P1)**: Can start after Foundational (Phase 2) - Builds on the US1 coordinator flow (T013 extends T009); independently testable via doubles
- **User Story 3 (P2)**: Can start after Foundational (Phase 2) - Builds on the US1 coordinator's network step (T017 exercises T009); Linux adapter work (T016) is independent of the coordinator and testable via doubles

### Within Each User Story

- Tests (included per constitution) are written alongside implementation using deterministic doubles
- Domain/port work (Phase 2) before coordinator work (US1)
- Coordinator skeleton (T009) before refusal wiring (T013) and network coverage (T017)
- Core implementation before failure-path and concurrency coverage
- Story complete before moving to next priority

### Parallel Opportunities

- T003 and T004 can run in parallel (different files: domain vs ports)
- T006 and T007 can run in parallel (different test targets)
- T012 can run in parallel with T013 (different files: integration failure-path tests vs manager coordinator)
- T016 can run in parallel with T017 (different files: Linux adapter vs manager tests with the fake)
- T018 and T019 can run in parallel (docs vs diff review)
- Once Foundational phase completes, all user stories can start in parallel (if team capacity allows)

---

## Parallel Example: User Story 1

```bash
# Launch contract and preflight tests together (different targets):
Task: "Add public-contract coverage in crates/sdk/tests/public_api.rs"
Task: "Add deterministic preflight coverage in crates/sdk/tests/failure_paths.rs"

# Launch refusal test and coordinator wiring together (different files):
Task: "Add refusal failure-path coverage in crates/sdk/tests/failure_paths.rs"
Task: "Wire running-refusal plus probe-error branch in crates/sdk/src/manager.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently (stopped VM deletes fully, result carries the name, repeat reports not-found, failures keep the record for retry)
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
   - Developer A: User Story 1 (volume helper, coordinator, lifecycle plus failure coverage)
   - Developer B: User Story 2 (running refusal, probe errors, concurrency)
   - Developer C: User Story 3 (Linux absent-tolerant release, scoping plus retry coverage)
3. Stories complete and integrate independently

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Avoid: vague tasks, same file conflicts, cross-story dependencies that break independence
