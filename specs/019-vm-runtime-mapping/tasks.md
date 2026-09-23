---
description: "Implementation tasks for per-VM loop and Device Mapper lifecycle"
---

# Tasks: Per-VM Runtime Disk Mapping

**Input**: Design documents from specs/019-vm-runtime-mapping/

**Prerequisites**: plan.md and spec.md; also research.md, data-model.md, contracts/runtime-disk-lifecycle.md, and quickstart.md.

**Tests**: Included because each user story defines an independent test and the project constitution requires feature-appropriate lifecycle and failure-path tests. Add story tests before their implementation tasks.

**Organization**: Tasks are grouped by user story to preserve traceability and stage the SDK lifecycle behavior.

## Format

Every task uses the required checklist form: checkbox, sequential task ID, optional parallel marker, optional user-story label, action, and exact repository path.

## Phase 1: Setup

**Purpose**: Add the shared dependency needed by the selected runtime lock design.

- [ ] T001 Add fs4 to the workspace dependency table in Cargo.toml and consume it from crates/sdk/Cargo.toml for cross-process advisory locking.

---

## Phase 2: Foundational

**Purpose**: Create the private SDK boundary and host primitives required by all lifecycle stories.

- [ ] T002 [P] Define the private RuntimeDiskController port and mapping identity/result types in crates/sdk/src/ports/runtime_disk.rs, and declare the module in crates/sdk/src/ports/mod.rs.
- [ ] T003 [P] Create DeviceMapperRuntime and declare its private runtime module in crates/sdk/src/adapters/runtime/device_mapper.rs and crates/sdk/src/adapters/runtime/mod.rs; apply the data-model rules: "Normalize and canonicalize the existing SDK home for runtime identity generation." and "Validate using the existing VM-name validator." Use a mapper UUID that is a "Stable owner UUID with a Taumaru prefix and the full digest." Ensure the mapper name follows the data-model rule: "It must fit Linux Device Mapper naming limits."
- [ ] T004 Wire the production runtime-disk adapter into MicroVmSdk while preserving an injectable fake controller for SDK tests in crates/sdk/src/manager.rs.
- [ ] T005 Implement the per-VM cross-process lock helper in crates/sdk/src/adapters/runtime/device_mapper.rs using nonblocking fs4 lock attempts with async waiting; hold the lock file for the full start/stop transition and never unlink it after use.
- [ ] T006 Implement shell-free losetup and dmsetup command helpers with captured output and typed error mapping in crates/sdk/src/adapters/runtime/device_mapper.rs.

**Checkpoint**: The private runtime-disk port, Linux adapter, process command helpers, and cross-process lock are ready for story work.

---

## Phase 3: User Story 1 - Start a MicroVM through its runtime disk mapping (Priority: P1)

**Goal**: Prepare or reuse a verified per-VM mapping before launch, pass only its path to firectl, and preserve the existing public start result.

**Independent Test**: Start two stopped MicroVMs with different root disks. Confirm each launch uses a distinct mapped path associated with its own file and neither launch passes the root disk file directly.

### Tests for User Story 1

- [ ] T007 [P] [US1] Add adapter tests for deterministic identity, valid mapping reuse, foreign UUID/backing conflicts, and cleanup of verified unused partial resources in crates/sdk/src/adapters/runtime/device_mapper.rs.
- [ ] T008 [P] [US1] Add manager tests proving mapping preparation precedes launch, two VMs receive independent paths, and a conflict never launches firectl in crates/sdk/src/manager.rs.
- [ ] T009 [P] [US1] Add Firecracker adapter tests proving the internal mapped path is passed as the writable root drive in crates/sdk/src/adapters/runtime/firecracker.rs.

### Implementation for User Story 1

- [ ] T010 [US1] Implement mapping preparation in crates/sdk/src/adapters/runtime/device_mapper.rs: attach the rootfs to a writable loop device with overlap protection; create "one snapshot-origin target covering the full rootfs block range" with a "Device Mapper sector count matching the loop device size"; verify the UUID, mapping table, dependency, and backing-file identity before reuse; and treat a name match without a UUID and table match as a conflict.
- [ ] T011 [US1] Rename the internal StartRequest disk field to runtime_disk_path and pass that value to firectl as the writable root-drive path with the :rw suffix in crates/sdk/src/ports/runtime.rs and crates/sdk/src/adapters/runtime/firecracker.rs; preserve the data-model rule that rootfs_path "Must remain the VM volume's existing regular rootfs.ext4 file. It is never replaced by the Device Mapper path in public results."
- [ ] T012 [US1] Integrate mapping preparation and the full-lifecycle lock into start_microvm in crates/sdk/src/manager.rs; persist process_id with process_state starting before readiness and mark it running only after readiness, following the data-model rules: "Record the spawned firectl identity before readiness completes; retain it if exit cannot be confirmed so stop can retry." and "Use starting while waiting for readiness, running after readiness, and stopped only after process and mapping cleanup complete. On retry, adopt a starting process only when PID, executable identity, and ready socket agree; do not clear it merely because the socket is silent. No new table or column is needed." If persisting the PID fails, terminate the child and verify exit before releasing mappings.
- [ ] T013 [US1] Document in crates/sdk/src/domain/microvm.rs that MicroVmStartResult.rootfs_path continues to identify the persistent .ext4 file and that the mapper path remains internal.

**Checkpoint**: Start launches through the verified mapper path, returns the unchanged public rootfs path, and safely reuses or rejects existing runtime resources.

---

## Phase 4: User Story 2 - Stop a MicroVM and release its runtime disk resources (Priority: P1)

**Goal**: After Firecracker exits, remove only the VM's owned mapper and loop association, retain retry evidence on failure, and preserve its rootfs.ext4.

**Independent Test**: Start and stop one VM; confirm its process and owned resources are gone, its rootfs.ext4 remains, and another VM's process and mapping remain untouched.

### Tests for User Story 2

- [ ] T014 [P] [US2] Add adapter tests proving normal Device Mapper removal precedes exact loop detachment, busy resources remain, and unrelated resources are untouched in crates/sdk/src/adapters/runtime/device_mapper.rs.
- [ ] T015 [P] [US2] Add manager tests for graceful and forced stop cleanup, silent sockets with live recorded processes, busy mappings, idempotent leftover cleanup, and preservation of a second VM in crates/sdk/src/manager.rs.

### Implementation for User Story 2

- [ ] T016 [US2] Implement idempotent mapping release in crates/sdk/src/adapters/runtime/device_mapper.rs; verify ownership before removal, remove the mapper before detaching its loop, wait for both resources to disappear, and never delete or modify rootfs.ext4. Follow the data-model constraint for mapper open_count: "Current kernel references reported for the mapper; nonzero means cleanup must not proceed."
- [ ] T017 [US2] Integrate release into every stop path in crates/sdk/src/manager.rs, including already-silent sockets; verify process exit and disk release before cleanup, retain process metadata on cleanup failure, and clear runtime metadata/remove the stale socket only after mapping cleanup succeeds.

**Checkpoint**: Stop is retryable and idempotent, removes only the target VM's verified resources, and leaves persistent disk contents intact.

---

## Phase 5: User Story 3 - Recreate transient mappings after a host restart (Priority: P2)

**Goal**: Prove that start reconstructs transient mappings from stable VM identity and rootfs state after kernel mapping state is lost.

**Independent Test**: Preserve inventory and rootfs.ext4, simulate loss of all loop and Device Mapper state, then start the VM and confirm a fresh per-VM mapping is created without relying on a persisted loop number.

**Implementation relationship**: The production create-on-missing path is implemented by User Story 1's mapping preparation. This phase adds separate reboot-recovery regression coverage and does not add a second mapping lifecycle.

- [ ] T018 [P] [US3] Add a manager regression test that reopens the same SDK home with empty fake kernel mapping state, starts the persisted VM, and verifies its deterministic mapper identity is recreated with a newly allocated loop; preserve the data-model rule for device_node: "It may change after reboot and is never treated as stable identity." in crates/sdk/src/manager.rs.
- [ ] T019 [P] [US3] Add a persistence regression test confirming transient mapper names and loop device numbers are not stored in vm_runtime and inventory/rootfs_path survive simulated host mapping loss in crates/sdk/tests/sqlite_persistence.rs.

**Checkpoint**: Reboot recovery is independently covered for one VM and for multiple VMs with distinct stable identities.

---

## Phase 6: Polish and Cross-Cutting Validation

**Purpose**: Verify the SDK, host integration, and documented runtime assumptions after all stories are implemented.

- [ ] T020 Run cargo fmt --all -- --check, cargo check --all-targets --all-features, cargo clippy --all-targets --all-features -- -D warnings, and cargo test --all-targets --all-features from the repository root as listed in specs/019-vm-runtime-mapping/quickstart.md.
- [ ] T021 Execute the privileged Linux lifecycle and reboot-equivalent mapping-loss scenario in specs/019-vm-runtime-mapping/quickstart.md using the project's exact firectl and Firecracker artifacts; verify two-VM isolation, block-device access, cleanup order, and preservation of both rootfs.ext4 files.

---

## Dependencies and Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: T001.
- **Foundational (Phase 2)**: T002 and T003 can run after T001; T004 needs T002 and T003; T005 and T006 need the adapter scaffold; all foundational tasks block user-story work.
- **User Stories (Phases 3-5)**: Begin after T006 and the rest of Phase 2.
- **Polish (Phase 6)**: Run after all user stories.

### User Story Dependencies

- **User Story 1 (P1)**: Starts after Phase 2. It provides the verified mapping preparation and internal launch path required by later lifecycle work.
- **User Story 2 (P1)**: Implement after User Story 1 because start and stop share manager.rs and the stop path must release the mapping produced by the start path.
- **User Story 3 (P2)**: Implement its regression coverage after User Story 1 and User Story 2. Its runtime behavior uses User Story 1's create-on-missing mapping path.

### Parallel Opportunities

- After T001, T002 and T003 can run in parallel because they define separate port and adapter files.
- In User Story 1, T007, T008, and T009 can run in parallel after the foundation because they cover separate adapter, manager, and Firecracker files.
- In User Story 2, T014 and T015 can run in parallel because they cover adapter and manager tests.
- In User Story 3, T018 and T019 can run in parallel because they cover manager lifecycle and SQLite persistence tests.

## Parallel Execution Examples

### User Story 1

Run T007, T008, and T009 concurrently after Phase 2. Then implement T010, T011, and T012 in dependency order; T013 can follow the public behavior change.

### User Story 2

Run T014 and T015 concurrently after User Story 1. Then implement T016 before T017 so the manager can call the completed release operation.

### User Story 3

Run T018 and T019 concurrently after User Stories 1 and 2. Both tests exercise transient state loss through separate SDK boundaries.

## Implementation Strategy

### MVP First

Complete Setup and Foundational phases, then implement User Story 1 and User Story 2 before the first end-to-end demonstration. Both P1 stories are needed together so start creates runtime resources and stop releases them safely. Add User Story 3's reboot-recovery regression coverage next.

### Incremental Delivery

1. Complete the SDK port, adapter scaffolding, and lock/command helpers.
2. Add User Story 1 tests, then implement verified mapping creation and mapped firectl launch.
3. Add User Story 2 tests, then implement process-safe mapper and loop cleanup.
4. Add User Story 3 recovery tests.
5. Run the Rust quality gates and privileged Linux validation from quickstart.md.

## Notes

- Parallel markers apply only when tasks work in different files and have no dependency on incomplete tasks.
- Story labels map directly to the user stories in specs/019-vm-runtime-mapping/spec.md.
- No CLI task is required; the existing CLI continues to call the SDK lifecycle methods.
