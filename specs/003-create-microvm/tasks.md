---

description: "Actionable task list for MicroVM creation and initial configuration"
---

# Tasks: Create and Initially Configure a MicroVM

**Input**: Design documents from `/specs/003-create-microvm/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/sdk-create.md](./contracts/sdk-create.md), and
[quickstart.md](./quickstart.md)

**Tests**: Included because `FR-024` explicitly requires SDK tests for success, failure, rollback,
multiple VMs, persistence, reconciliation, stopped state, and silent/error-safe behavior.

**Organization**: Tasks are grouped by the four P1 user stories. Shared ports, persistence, domain
types, and artifact readiness are completed before story-specific implementation.

## Path Conventions

- SDK production code: `crates/sdk/src/`
- SDK integration and public-contract tests: `crates/sdk/tests/`
- SDK database migrations: `crates/sdk/migrations/`
- Feature design documents: `specs/003-create-microvm/`
- The CLI and files from `main` are not modified by these tasks.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish the SDK module/test structure and dependencies required by the feature.

- [ ] T001 Add the planned SDK module declarations for MicroVM domain, storage, credentials, network, and runtime boundaries in `crates/sdk/src/domain/mod.rs`, `crates/sdk/src/ports/mod.rs`, and `crates/sdk/src/adapters/mod.rs`.
- [ ] T002 Add the minimal process, time, and network Tokio features plus Rust Ed25519 SSH-key serialization dependencies in `crates/sdk/Cargo.toml` and update `Cargo.lock` without adding a CLI dependency on the SDK implementation.
- [ ] T003 [P] Add deterministic MicroVM fixture builders and fake artifact, repository, storage, credential, network, and runtime ports in `crates/sdk/tests/support/microvm.rs` and register them in `crates/sdk/tests/support/mod.rs`.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Implement shared domain, persistence, artifact, validation, and port boundaries. No
user story implementation can begin until this phase is complete.

- [ ] T004 [P] Create the MicroVM identity, creation-result, network-result, SSH metadata, and immutable configuration types in `crates/sdk/src/domain/microvm.rs`, `crates/sdk/src/domain/config.rs`, and `crates/sdk/src/domain/lifecycle.rs`, including the states `creating`, temporary `running`, and successful stopped `configured`.
- [ ] T005 [P] Extend distribution-image metadata and validation for optional `minimum_size_bytes`, ext4 format/filesystem checks, and split runtime package fixtures in `crates/sdk/src/domain/registry.rs`, `crates/sdk/src/adapters/registry/taumaru.rs`, and `crates/sdk/tests/fixtures/manifest.json`.
- [ ] T006 [P] Add typed SDK errors for invalid requests, artifact readiness, disk-size prerequisites, runtime compatibility, storage ownership, lifecycle conflicts, network/permission failures, guest filesystem/credentials, temporary startup, and aggregate cleanup in `crates/sdk/src/error.rs` without including private-key contents or unrestricted command output.
- [ ] T007 Add `0002_microvm_creation.sql` with `microvms`, `vm_networks`, `network_bridges`, `vm_network_resources`, `vm_credentials`, and `vm_runtime` tables, ownership constraints, path/address indexes, and foreign-key behavior in `crates/sdk/migrations/0002_microvm_creation.sql`; register it and require its schema in `crates/sdk/src/adapters/persistence/migrations.rs`.
- [ ] T008 Extend the artifact and repository ports for local prerequisite resolution, binary-component candidate lookup, VM lookup, immutable-config comparison, provisional state transitions, network ownership, credential paths, and runtime metadata in `crates/sdk/src/ports/artifacts.rs` and `crates/sdk/src/ports/repository.rs`.
- [ ] T009 Implement the SQLite repository methods and transaction boundaries for VM records, child metadata, bridge sharing, network-resource fingerprints, credentials, runtime state, and rollback deletion in `crates/sdk/src/adapters/persistence/sqlite.rs`; add migration and persistence assertions in `crates/sdk/tests/sqlite_persistence.rs`.
- [ ] T010 Add replaceable ports for ext4/guest storage, Ed25519 credentials, Linux network reconciliation, and temporary Firecracker process control in `crates/sdk/src/ports/storage.rs`, `crates/sdk/src/ports/credentials.rs`, `crates/sdk/src/ports/network.rs`, `crates/sdk/src/ports/runtime.rs`, and `crates/sdk/src/ports/mod.rs`.
- [ ] T011 Implement a complete local creation-prerequisite resolver in `crates/sdk/src/manager.rs` and `crates/sdk/src/ports/artifacts.rs` using the existing verified binary inventory and `resolve_binary(package_id, component_name)` behavior; select a combined package or independently selected split packages by required component, host architecture, valid semantic version, highest version, and deterministic package-ID tie-breaking, without requiring equal versions.
- [ ] T012 Implement request, path, resource, architecture, memory conversion, name, and volume ownership validation in `crates/sdk/src/domain/config.rs` and `crates/sdk/src/manager.rs`, enforcing the exact name rule `1–64 ASCII characters; first character alphanumeric; remaining characters alphanumeric, `-`, or `_`; no spaces, separators, dots, or control characters`, positive disk/vCPU/RAM values, SQLite-safe integer ranges, and the default `{sdk_home}/vms/{name}` directory.
- [ ] T013 Re-export the stable request, result, lifecycle, network, and SSH types from `crates/sdk/src/lib.rs` while keeping ports, adapters, process handles, commands, and secret material private.
- [ ] T014 [P] Add foundational tests for optional image minimum metadata, ext4 validation, split package selection with different versions, name/path/resource validation, migration drift, and required schema in `crates/sdk/tests/registry_contract.rs`, `crates/sdk/tests/public_api.rs`, and `crates/sdk/tests/sqlite_persistence.rs`.

**Checkpoint**: The SDK has typed domain boundaries, persistent VM schema, local artifact
resolution, validation, fake ports, and no CLI lifecycle changes. User-story work may begin.

---

## Phase 3: User Story 1 - Create and Configure a Host-Only MicroVM (Priority: P1) 🎯 MVP

**Goal**: Create one VM from verified local artifacts, copy and prepare its writable ext4 rootfs,
inject SSH access, configure host-only `/30` networking, persist the result, and return a stopped
`configured` VM.

**Independent Test**: With a verified selected distribution image, default kernel, and compatible
Firecracker/`firectl` artifacts in fake local inventory, create a host-only VM and verify its unique
identity, copied/resized rootfs, injected public key, persisted SSH private-key path, `/30` network,
requested resources, and absent process/socket after success.

### Tests for User Story 1

- [ ] T015 [P] [US1] Write failing public-contract tests for `CreateMicroVmRequest`, `MicroVmCreationResult`, `MicroVmState::Configured`, stopped-state guarantees, and private-key-path-only SSH metadata in `crates/sdk/tests/public_api.rs`.
- [ ] T016 [P] [US1] Write failing host-only lifecycle tests for explicit image selection, default/custom volume paths, copied `rootfs.ext4`, `/30` network metadata, SSH public-key injection, requested resources, persisted records, and no active Firecracker process or socket in `crates/sdk/tests/lifecycle.rs`.

### Implementation for User Story 1

- [ ] T017 [P] [US1] Implement verified source-image copying, attempt-local temporary files, atomic publication, monotonic file expansion, offline ext4 resize, and final byte/filesystem verification in `crates/sdk/src/adapters/storage/ext4.rs`; preserve the source image and reject any shrink operation.
- [ ] T018 [P] [US1] Implement per-VM Ed25519 key generation, restrictive host file modes, public-key serialization, fingerprint calculation, and path-only credential metadata in `crates/sdk/src/adapters/credentials/ed25519.rs`.
- [ ] T019 [US1] Implement offline guest-rootfs mutation in `crates/sdk/src/adapters/storage/guest_fs.rs`, mounting the VM-local copy privately, preserving existing keys, appending the generated public key once to `/root/.ssh/authorized_keys`, enforcing guest path/mode checks, unmounting, and removing the temporary mountpoint.
- [ ] T020 [P] [US1] Implement host-only `/30` allocation, collision checks against persisted/live state, deterministic TAP/MAC naming, host endpoint configuration, forwarding, per-VM nftables NAT ownership, and static guest `ip=` parameters in `crates/sdk/src/adapters/network/linux.rs`.
- [ ] T021 [P] [US1] Implement the private `firectl` runtime adapter in `crates/sdk/src/adapters/runtime/firecracker.rs`, constructing validated arguments for the independent Firecracker binary, kernel, VM-local writable rootfs, vCPUs, effective memory MiB, boot/network parameters, TAP/MAC, and `{volume_path}/firecracker.sock`, while capturing output silently.
- [ ] T022 [US1] Implement the host-only creation coordinator in `crates/sdk/src/manager.rs`, ordering preflight, provisional `creating` persistence, volume copy/resize, credentials, guest injection, host-only network, runtime preparation, final revalidation, and stopped `configured` persistence.
- [ ] T023 [US1] Implement complete result loading and persistence mapping for volume, rootfs, expected socket, requested/effective resources, network, runtime artifact IDs, SSH paths, and `root:22` metadata in `crates/sdk/src/manager.rs`, `crates/sdk/src/adapters/persistence/sqlite.rs`, and `crates/sdk/src/domain/microvm.rs`.
- [ ] T024 [US1] Make the public API and lifecycle tests pass while asserting that the SDK is silent, never exposes private-key contents, never mutates the verified source image, and returns only after the VM is stopped in `crates/sdk/tests/public_api.rs` and `crates/sdk/tests/lifecycle.rs`.

**Checkpoint**: User Story 1 is independently usable as the host-only MVP and returns a configured,
stopped VM with a copied rootfs and SSH metadata.

---

## Phase 4: User Story 2 - Reject Missing Prerequisites Safely (Priority: P1)

**Goal**: Fail before host mutation for missing/stale/incompatible prerequisites and fully roll back
post-preflight failures, while supporting idempotent creation and immutable conflicts.

**Independent Test**: Repeat creation with missing inventory records, missing/corrupt files, bad
architecture, stale binaries, invalid disk sizes, ownership conflicts, and injected runtime/network/
guest/SQLite failures; verify typed errors, no unsafe output, no configured row, and complete cleanup.

### Tests for User Story 2

- [ ] T025 [P] [US2] Write failing prerequisite tests for unknown/incomplete/stale distribution images, missing registry minimum, stale kernel, missing or non-executable Firecracker/`firectl`, incompatible architecture, invalid resources, and no host mutation in `crates/sdk/tests/failure_paths.rs`.
- [ ] T026 [US2] Write failing concurrency, immutable-conflict, volume-ownership, rollback, caller-data-preservation, and SDK-silence tests in `crates/sdk/tests/concurrency.rs` and `crates/sdk/tests/failure_paths.rs`.

### Implementation for User Story 2

- [ ] T027 [US2] Map every preflight failure from artifact, registry, filesystem, KVM, executable, architecture, and resource validation to actionable typed errors before provisional VM or host-resource creation in `crates/sdk/src/manager.rs` and `crates/sdk/src/error.rs`.
- [ ] T028 [US2] Enforce source-image minimum/current-size checks, empty-or-owned VM-volume rules, path containment, and caller-data preservation in `crates/sdk/src/manager.rs` and `crates/sdk/src/adapters/storage/ext4.rs`.
- [ ] T029 [US2] Implement identical-request reuse and immutable configuration conflict detection for name, distribution, image, resources, network mode, and volume path in `crates/sdk/src/manager.rs`, `crates/sdk/src/ports/repository.rs`, and `crates/sdk/src/adapters/persistence/sqlite.rs`.
- [ ] T030 [US2] Implement an attempt ownership journal and reverse-order rollback for process, socket, network resources, shared bridge references, guest mount, keys, copied rootfs, attempt-created directory, and provisional SQLite rows in `crates/sdk/src/manager.rs`, `crates/sdk/src/domain/lifecycle.rs`, and `crates/sdk/src/adapters/persistence/sqlite.rs`.
- [ ] T031 [US2] Ensure every retry performs fresh inventory and filesystem preflight, captures subprocess output without forwarding it, omits private-key material from errors, and returns aggregate cleanup failures without publishing a usable VM in `crates/sdk/src/manager.rs`, `crates/sdk/src/error.rs`, and `crates/sdk/src/adapters/runtime/firecracker.rs`.
- [ ] T032 [US2] Make the prerequisite, conflict, rollback, concurrency, caller-data-preservation, no-output, and no-panic tests pass in `crates/sdk/tests/failure_paths.rs` and `crates/sdk/tests/concurrency.rs`.

**Checkpoint**: User Stories 1 and 2 are safe to retry and cannot publish a partial or misleading
VM when prerequisites or post-preflight steps fail.

---

## Phase 5: User Story 3 - Configure a LAN-Reachable MicroVM (Priority: P1)

**Goal**: Add explicit LAN exposure using a managed bridge, detected uplink, external DHCP, and a
temporary stopped-before-return runtime path, without falling back to host-only mode.

**Independent Test**: On a prepared test network with fake or isolated Linux network ports, create
LAN-enabled VMs and verify bridge/uplink/TAP ownership, MAC-correlated DHCP address, persistence,
conflict handling, multiple-VM isolation, and no running process/socket after creation.

### Tests for User Story 3

- [ ] T033 [P] [US3] Write failing LAN creation tests for bridge/uplink detection, DHCP address discovery, SSH connection metadata, explicit no-fallback behavior, permission/DHCP/uplink errors, and stopped final state in `crates/sdk/tests/lan_network.rs`.
- [ ] T034 [US3] Write failing multi-VM LAN tests for shared bridge reference counting, unique TAP/MAC/address ownership, active-peer conflict handling, and preservation of another VM during rollback in `crates/sdk/tests/lan_network.rs`.

### Implementation for User Story 3

- [ ] T035 [US3] Implement detected default-route uplink handling, SDK-managed bridge creation/reuse, host address/route preservation, physical-uplink attachment, and shared bridge ownership/refcounts in `crates/sdk/src/adapters/network/linux.rs` and `crates/sdk/src/adapters/persistence/sqlite.rs`.
- [ ] T036 [US3] Implement LAN TAP attachment, external DHCP boot parameter rendering, MAC-correlated lease observation, address conflict validation, and typed no-DHCP/no-uplink/permission errors in `crates/sdk/src/adapters/network/linux.rs` and `crates/sdk/src/ports/network.rs`.
- [ ] T037 [US3] Extend temporary runtime control with bounded LAN start, DHCP observation, exact-process stop/wait, active-socket removal, and final stopped verification in `crates/sdk/src/adapters/runtime/firecracker.rs` and `crates/sdk/src/ports/runtime.rs`.
- [ ] T038 [US3] Integrate the LAN path into creation, persist bridge/uplink/lease/address metadata, preserve explicit LAN mode on failure, and prohibit silent host-only fallback in `crates/sdk/src/manager.rs`, `crates/sdk/src/domain/config.rs`, and `crates/sdk/src/adapters/persistence/sqlite.rs`.
- [ ] T039 [US3] Make LAN creation, DHCP, no-fallback, address-conflict, shared-bridge, multi-VM, rollback, and stopped-process tests pass in `crates/sdk/tests/lan_network.rs`.

**Checkpoint**: User Story 3 adds explicit LAN reachability without changing host-only defaults or
leaving a temporary runtime active.

---

## Phase 6: User Story 4 - Reconcile Network Configuration for an Existing VM (Priority: P1)

**Goal**: Expose an independent network operation that reads persisted desired state, skips correct
resources, repairs missing or SDK-owned stale resources, and leaves every VM stopped.

**Independent Test**: Recreate the SDK with persisted host-only and LAN VMs, remove or preserve
selected host resources, call `configure_network`, and verify applied/skipped reporting, no duplicate
resources, preserved identities/credentials, and a stopped final state.

### Tests for User Story 4

- [ ] T040 [P] [US4] Write failing reconciliation tests for correct-resource skips, missing/stale resource repair, foreign-resource conflicts, applied/skipped result reporting, idempotent repetition, and stopped state in `crates/sdk/tests/network_reconciliation.rs`.
- [ ] T041 [US4] Write failing SDK-recreation and cross-VM isolation tests for persisted desired mappings, host-only `/30` reuse, LAN bridge/lease reuse, missing host state, and preservation of another VM in `crates/sdk/tests/network_reconciliation.rs`.

### Implementation for User Story 4

- [ ] T042 [US4] Add repository loading and atomic persistence for desired network configuration, resource fingerprints, ownership, shared bridges, and observed lease/address state in `crates/sdk/src/ports/repository.rs` and `crates/sdk/src/adapters/persistence/sqlite.rs`.
- [ ] T043 [US4] Implement host-only reconciliation that independently inspects TAP, `/30`, address, forwarding, NAT, and Firecracker interface resources, reports matching items as skipped, and repairs only missing or SDK-owned stale items in `crates/sdk/src/adapters/network/linux.rs`.
- [ ] T044 [US4] Implement LAN reconciliation for managed bridge/uplink/TAP attachments and DHCP lease/address drift, retaining shared resources used by other VMs and rejecting foreign ownership in `crates/sdk/src/adapters/network/linux.rs`.
- [ ] T045 [US4] Expose `configure_network(vm_name)` and applied/skipped `NetworkConfigurationResult` through `crates/sdk/src/manager.rs` and `crates/sdk/src/lib.rs`, reading the persisted desired mode instead of accepting an unpersisted replacement mode.
- [ ] T046 [US4] Enforce configured-VM, rootfs/credential/path, process, and socket revalidation plus temporary LAN stop/remove-socket behavior before reconciliation returns in `crates/sdk/src/manager.rs` and `crates/sdk/src/adapters/runtime/firecracker.rs`.
- [ ] T047 [US4] Make network reconciliation, SDK recreation, skip/repair, foreign ownership, multi-VM, and stopped-state tests pass in `crates/sdk/tests/network_reconciliation.rs`.

**Checkpoint**: User Story 4 provides the focused post-restart network repair operation without
introducing list, inspect/status, start, stop, reboot, delete, or CLI lifecycle behavior.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Complete public documentation, privileged-test coverage, quickstart verification, and
repository quality gates without expanding feature scope.

- [ ] T048 [P] Add Rustdoc for all new public SDK types, `create_microvm`, `configure_network`, typed error behavior, stopped-state guarantees, and private-key-path semantics in `crates/sdk/src/lib.rs`, `crates/sdk/src/domain/microvm.rs`, `crates/sdk/src/domain/config.rs`, and `crates/sdk/src/error.rs`.
- [ ] T049 [P] Add opt-in Linux namespace tests for `ip`/`nft` command translation, TAP/bridge ownership, `/30` allocation, and capability failures in `crates/sdk/tests/linux_integration.rs` and `crates/sdk/tests/support/linux.rs`.
- [ ] T050 Verify the SDK-only usage, host prerequisites, host-only/LAN outcomes, reconciliation behavior, and failure expectations in `specs/003-create-microvm/quickstart.md` against the implemented public API.
- [ ] T051 Run `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` from the repository root; record any environment-gated Linux test limitations in `specs/003-create-microvm/quickstart.md`.
- [ ] T052 Review the final diff for SDK/CLI boundary violations, accidental edits outside `specs/003-create-microvm/` and `crates/sdk/`, private-key leakage, destructive cleanup, unresolved placeholders, and English-only repository text in `specs/003-create-microvm/plan.md`, `specs/003-create-microvm/spec.md`, and the affected SDK files.

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: T001–T003; no user-story work starts before the SDK module/test harness is available.
- **Foundational (Phase 2)**: T004–T014 depend on Setup and block every user story.
- **User Story 1 (Phase 3)**: T015–T024 depend on Foundational and deliver the host-only MVP.
- **User Story 2 (Phase 4)**: T025–T032 depend on the creation path from US1, then harden its preflight, conflict, and rollback behavior.
- **User Story 3 (Phase 5)**: T033–T039 depend on the common runtime/storage boundaries from US1 and rollback/ownership guarantees from US2.
- **User Story 4 (Phase 6)**: T040–T047 depend on persisted network resources from US1/US3 and ownership-safe persistence from US2.
- **Polish (Phase 7)**: T048–T052 depend on the desired user stories being complete.

### User Story Dependencies

- **US1 (P1)**: Starts after Foundational; no dependency on another user story. It is the MVP path.
- **US2 (P1)**: Depends on the US1 creation coordinator so its failure tests exercise the real preflight/rollback path; its safety behavior is required before production use of US1.
- **US3 (P1)**: Depends on shared artifact, storage, runtime, persistence, and cleanup boundaries from US1/US2; LAN is an additive network mode.
- **US4 (P1)**: Depends on persisted host-only/LAN desired state and resource ownership from US1/US3 plus rollback-safe repository behavior from US2.

### Parallel Opportunities

- T004, T005, and T006 can proceed in parallel after Setup because they touch separate domain/registry/error areas.
- T015 and T016 can be written in parallel because they target separate public API and lifecycle test files.
- T017, T018, T020, and T021 can be implemented in parallel after their ports exist because storage, credentials, network, and runtime adapters have separate ownership.
- T026 can be written after T025 because both extend the failure-path coverage and share `crates/sdk/tests/failure_paths.rs`; its concurrency-specific cases remain isolated in `crates/sdk/tests/concurrency.rs`.
- T033 and T034 can be written in parallel only if the test file ownership is split; otherwise keep them sequential in `lan_network.rs`.
- T040 and T041 can be written in parallel only if their test cases remain isolated within the reconciliation test module.
- T048 and T049 can proceed in parallel with separate source/test files after the story implementations stabilize.

## Parallel Example: User Story 1

```text
Task T015: Write public API contract tests in crates/sdk/tests/public_api.rs
Task T016: Write host-only lifecycle tests in crates/sdk/tests/lifecycle.rs

Task T017: Implement ext4 copy/resize in crates/sdk/src/adapters/storage/ext4.rs
Task T018: Implement Ed25519 credentials in crates/sdk/src/adapters/credentials/ed25519.rs
Task T020: Implement host-only network resources in crates/sdk/src/adapters/network/linux.rs
Task T021: Implement the Firecracker runtime adapter in crates/sdk/src/adapters/runtime/firecracker.rs
```

These tasks can run together after Foundational because they do not share implementation files.
T019, T022, and T023 then integrate their outputs sequentially.

## Implementation Strategy

### MVP First

1. Complete Setup and Foundational phases.
2. Complete US1 host-only creation and verify the stopped configured result.
3. Complete the minimum US2 preflight/rollback gates before treating the MVP as safe to run.
4. Stop and validate the host-only workflow independently before adding LAN behavior.

### Incremental Delivery

1. Foundation ready: typed domain, SQLite schema, artifact prerequisites, and fake ports.
2. US1: host-only creation with copied/resized rootfs, SSH key injection, and stopped result.
3. US2: missing-prerequisite errors, ownership conflicts, idempotency, and rollback hardening.
4. US3: explicit LAN bridge/DHCP path with no fallback.
5. US4: persisted network reconciliation with skip/repair reporting after SDK recreation.
6. Polish: Rustdoc, isolated Linux tests, quickstart validation, and full quality gates.

## Notes

- Every task has the required checkbox, sequential ID, optional `[P]` marker, required story label
  for story phases, and an explicit repository path.
- Tests are written before their story implementation tasks and must fail for the missing behavior
  before implementation makes them pass.
- `tasks.md` does not add list, inspect/status, start, stop, reboot, delete, cloning, snapshots,
  migration, automatic download, or CLI lifecycle work.
