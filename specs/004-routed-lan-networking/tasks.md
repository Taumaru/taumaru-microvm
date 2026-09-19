---

description: "Actionable task list for routed LAN networking migration"

---

# Tasks: Routed LAN Networking

**Input**: Design documents from `/specs/004-routed-lan-networking/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md), [data-model.md](./data-model.md), [contracts/sdk-routed-lan.md](./contracts/sdk-routed-lan.md), and [quickstart.md](./quickstart.md)

**Tests**: Included because the spec's measurable outcomes (SC-001 through SC-006) require allocation, isolation, reconcile, rollback, and privilege failure-path coverage, and the constitution requires unit, failure-path, and boundary tests for behavior changes.

**Organization**: Tasks are grouped by the four user stories. Shared domain, persistence, port, and error foundations are completed before story-specific implementation. The CLI is not modified.

## Path Conventions

- SDK production code: `crates/sdk/src/`
- SDK integration and public-contract tests: `crates/sdk/tests/`
- SDK database migrations: `crates/sdk/migrations/`
- Feature design documents: `specs/004-routed-lan-networking/`
- The CLI and registry/download behavior are not modified by these tasks.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish the routed-LAN work surface and confirm the starting tree.

- [X] T001 Create the routed-LAN task work surface by confirming `crates/sdk/src/adapters/network/linux.rs`, `crates/sdk/src/manager.rs`, `crates/sdk/src/domain/microvm.rs`, `crates/sdk/src/domain/config.rs`, `crates/sdk/src/ports/network.rs`, `crates/sdk/src/error.rs`, `crates/sdk/src/adapters/persistence/sqlite.rs`, and `crates/sdk/src/adapters/persistence/migrations.rs` exist and host the bridge/DHCP/nftables LAN implementation to be replaced.
- [X] T002 [P] Record the pre-migration LAN baseline (`cargo test -p taumaru-microvm --all-features`, `cargo clippy -p taumaru-microvm --all-targets --all-features -- -D warnings`) in the task tracking notes without changing code.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Implement shared domain, error, persistence, and port boundaries for routed LAN. No user story work can begin until this phase is complete.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [X] T003 Add the optional explicit LAN override `lan_address: Option<std::net::Ipv4Addr>` to `CreateMicroVmRequest` in `crates/sdk/src/domain/microvm.rs`, re-export it from `crates/sdk/src/lib.rs` with Rustdoc, and reject `Some` with `expose_on_lan: false` as an invalid request in `crates/sdk/src/domain/config.rs`.
- [X] T004 [P] Add the public routed address `lan_address: Option<std::net::IpAddr>` (`Some` for LAN, `None` for host-only) to `NetworkConfiguration` in `crates/sdk/src/domain/microvm.rs` with Rustdoc.
- [X] T005 [P] Add routed `NetworkResource` kinds `HostRoute`, `ProxyArpEntry`, `ForwardRule`, and `IptablesNat` with exact `as_str`/`parse` coverage (`"host_route"`, `"proxy_arp_entry"`, `"forward_rule"`, `"iptables_nat"`) in `crates/sdk/src/domain/microvm.rs`, keeping legacy bridge/DHCP kinds parseable for old rows only.
- [X] T006 Extend `PersistedNetwork` in `crates/sdk/src/domain/microvm.rs` with the committed LAN address, uplink CIDR, proxy-ARP ownership detail, and per-resource fingerprints needed for routed reconcile and rollback.
- [X] T007 Add `crates/sdk/migrations/0003_routed_lan.sql` with routed `vm_networks` columns (`lan_ip`, `uplink_cidr`, `proxy_arp_enabled_by_sdk`, rule-ownership detail), register it as version 3 in `crates/sdk/src/adapters/persistence/migrations.rs`, and extend required-schema verification; legacy bridge/`nft` columns stay untouched and old bridge rows are ignored, never migrated.
- [X] T008 Extend the repository load/persist coverage for the new routed columns and resource kinds in `crates/sdk/src/adapters/persistence/sqlite.rs`, including `vm_networks` insert/update/select, resource row rewrite, and migration-drift assertions in `crates/sdk/tests/sqlite_persistence.rs`.
- [X] T009 Extend the replaceable `NetworkController` port in `crates/sdk/src/ports/network.rs` with routed-LAN inputs (uplink identity, LAN offer, override), ownership-aware reconcile/repair signals, and rollback deletion needed by host setup, guest setup, reconcile, and cleanup.
- [X] T010 Extend typed `SdkError` diagnostics in `crates/sdk/src/error.rs` for routed operations (`detect uplink`, `allocate LAN address`, `probe duplicate`, `publish proxy entry`, `apply iptables rule`), duplicate/conflict owners, guest-setup conditions, privilege failures naming the missing capability with command plus diagnostic output, and aggregate cleanup failures; never include private-key content or guest secrets.
- [X] T011 [P] Add foundational tests for LAN override validation, routed resource-kind parsing, migration schema requirements, and privilege-error shape in the existing SDK unit tests and `crates/sdk/tests/public_api.rs`.

**Checkpoint**: The SDK has routed domain types, migration 3, repository coverage, port boundaries, and diagnostics with no CLI changes. User-story work may begin.

---

## Phase 3: User Story 1 - Create a LAN-Reachable MicroVM Over Any Uplink (Priority: P1) 🎯 MVP

**Goal**: Create one LAN VM over Wi-Fi or Ethernet with a private `/30`, a duplicate-checked LAN address, host route plus proxy ARP, `iptables` forwarding/NAT, and post-boot SSH guest setup, returning a stopped `configured` VM.

**Independent Test**: Create one LAN VM on a Wi-Fi host with verified artifacts, then verify the host reaches both addresses, another LAN device reaches the LAN address, the guest reaches public names, and no process/socket remains active.

### Tests for User Story 1

- [X] T012 [P] [US1] Add public-contract coverage for the `lan_address` override, routed `NetworkConfiguration`, new resource kinds, stopped `configured` result, and path-only SSH metadata in `crates/sdk/tests/public_api.rs` and manager unit tests.
- [X] T013 [P] [US1] Add deterministic creation coverage for uplink detection, selection order (explicit, previous-valid, automatic), duplicate handling, TAP/route/proxy/`iptables` ownership, guest end-state, and stopped postconditions with injected ports in `crates/sdk/src/manager.rs` tests.
- [X] T014 [P] [US1] Implement default-uplink detection (default-route interface, global IPv4 CIDR, gateway, derived LAN subnet) with typed pre-mutation errors in `crates/sdk/src/adapters/network/linux.rs`.
- [X] T015 [US1] Implement LAN offer selection in `crates/sdk/src/adapters/network/linux.rs`: validate explicit overrides against the uplink subnet while excluding network, broadcast, host, and gateway addresses; reuse the previous assignment when valid and free; otherwise search the upper subnet with `arping -D` duplicate evidence and the documented `ping` fallback (depends on T014).
- [X] T016 [US1] Implement routed host setup in `crates/sdk/src/adapters/network/linux.rs`: deterministic TAP, `<host>/30`, up state, `<lan>/32` host route, uplink proxy ARP entry, forwarding/proxy sysctls, and owned `iptables` FORWARD plus private-range MASQUERADE rules via typed argv with captured diagnostics (depends on T015).
- [X] T017 [US1] Implement bounded post-boot SSH guest setup in the runtime/network boundary (`crates/sdk/src/adapters/runtime/firecracker.rs` and `crates/sdk/src/adapters/network/linux.rs`): boot private-only, wait for private SSH with the VM key, apply LAN `/32`, routes, and DNS, verify private/LAN/egress reachability, stop the exact process, and remove the socket (depends on T016).
- [X] T018 [US1] Implement the LAN creation coordinator ordering in `crates/sdk/src/manager.rs`: preflight, provisional `creating` persistence, uplink/offer commitment, host setup, credential reuse, private boot, guest setup, final revalidation, and stopped `configured` persistence with ownership journaling (depends on T014, T015, T016, T017).
- [X] T019 [US1] Persist the committed routed mapping, fingerprints, and ownership flags transactionally in `crates/sdk/src/adapters/persistence/sqlite.rs` without holding transactions across host commands (depends on T007, T008, T018).

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently

---

## Phase 4: User Story 2 - Run Network Setup With Elevated Privileges By Default (Priority: P1)

**Goal**: Attempt privileged mutations directly as the normal path and fail fast with a named privilege error plus rollback when privilege is absent.

**Independent Test**: Run creation privileged (succeeds end to end) and unprivileged (typed privilege error naming the missing capability within 30 seconds, no configured VM, attempt-owned resources removed).

### Tests for User Story 2


### Implementation for User Story 2
- [X] T020 [P] [US2] Add failure-path coverage for unprivileged TAP/route/proxy/`iptables` mutations, command-plus-output diagnostics, and never-absence semantics in `crates/sdk/tests/failure_paths.rs` and adapter unit tests.
- [X] T021 [US2] Route every new `ip`/`iptables`/`sysctl`/`arping` call in `crates/sdk/src/adapters/network/linux.rs` through captured-stderr diagnostics with the `CAP_NET_ADMIN` hint on privilege denial, and treat only explicit not-found output as absence in existence probes.
- [X] T022 [US2] Ensure privileged file ownership and modes for attempt-created volume, key, and rootfs paths remain usable by subsequent privileged operations in `crates/sdk/src/adapters/storage/ext4.rs`, `crates/sdk/src/adapters/storage/guest_fs.rs`, `crates/sdk/src/adapters/credentials/ed25519.rs`, and `crates/sdk/src/manager.rs`.
- [X] T023 [US2] Wire first-privileged-mutation failure to full attempt rollback and no `configured` result in `crates/sdk/src/manager.rs` (depends on T021).
---

## Phase 5: User Story 3 - Reconcile Routed LAN State After a Host Restart (Priority: P2)

**Goal**: Restore only missing or SDK-owned stale routed items while keeping committed addresses stable and the VM stopped.

**Independent Test**: Wipe host-side TAP/route/proxy/rules while keeping SQLite, reconcile, and verify the same addresses return with no duplicates and the VM stopped.

### Tests for User Story 3
- [X] T024 [P] [US3] Add reconcile coverage for skip-correct, repair-missing/stale, address stability, no cross-VM disturbance, and stopped return in manager unit tests and `crates/sdk/tests/lifecycle.rs` (or the nearest lifecycle suite).
- [X] T025 [US3] Implement routed reconcile comparison per resource (TAP, address, route, proxy entry, sysctls, owned `iptables` rules) with skip/repair/conflict outcomes in `crates/sdk/src/adapters/network/linux.rs` (depends on Phase 2 foundations).
- [X] T026 [US3] Wire persisted-mapping reconcile through `configure_network` in `crates/sdk/src/manager.rs`, persisting observations without reallocating addresses and leaving the VM stopped (depends on T025).

---

## Phase 6: User Story 4 - Roll Back Routed LAN Attempts Cleanly (Priority: P2)

**Goal**: Prove that any failed routed attempt removes every owned host trace while preserving shared and foreign state.

**Independent Test**: Force failure after network setup and verify owned TAP/route/proxy/rules are gone, shared chains and unrelated rules remain, and retry can reuse the freed address.

- [X] T027 [P] [US4] Add injected-failure rollback coverage proving owned-resource absence and shared-state preservation in `crates/sdk/tests/failure_paths.rs` and manager unit tests.
- [X] T028 [US4] Implement reverse-order routed rollback (owned rules, proxy entry, host route, TAP, keys/rootfs, provisional row) with reference-checked shared sysctls in `crates/sdk/src/adapters/network/linux.rs` and `crates/sdk/src/manager.rs` (depends on T016, T018).
- [X] T029 [US4] Unify host-only NAT on the `iptables` backend with identical observable semantics in `crates/sdk/src/adapters/network/linux.rs`, keeping host-only free of LAN routes and proxy entries (depends on T028).


**Checkpoint**: All user stories should now be independently functional

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Cross-story validation, docs, and gates.

- [X] T030 [P] Update Rustdoc for new request/config/resource/persistence contracts and routed invariants in `crates/sdk/src/lib.rs`, `crates/sdk/src/domain/microvm.rs`, `crates/sdk/src/error.rs`, and touched adapter/ports modules.
- [X] T031 [P] Add multi-VM isolation coverage (distinct LAN plus private addresses, no cross-delivery) with injected ports in the SDK manager tests.
- [X] T032 Run `specs/004-routed-lan-networking/quickstart.md` validation, including privileged LAN creation, host-only regression, reconcile-after-loss, and failure drills.
- [X] T033 Run the repository gates (`cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`) and record results.

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - User stories can then proceed in parallel (if staffed)
  - Or sequentially in priority order (P1 → P2 → P3)
- **Polish (Final Phase)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 2 (P1)**: Can start after Foundational (Phase 2) - Integrates privilege diagnostics with US1 host mutations but independently testable via failure drills
- **User Story 3 (P2)**: Can start after Foundational (Phase 2) - Reuses US1 mapping shape but reconciles independently
- **User Story 4 (P2)**: Can start after Foundational (Phase 2) - Depends on US1 setup order for rollback coverage but validates independently

### Within Each User Story

- Tests MUST be written and FAIL before implementation
- Domain/persistence before adapter behavior
- Host setup before guest setup before coordinator wiring
- Core implementation before integration
- Story complete before moving to next priority

### Parallel Opportunities

- T002 with T001 (baseline recording is read-only)
- T004, T005, T010, T011 can run in parallel (different files, no dependencies)
- T012 with T013 (contract vs deterministic creation coverage, different files)
- T014 with T020/T024/T027 test authoring once foundations land (different files)
- Once Foundational completes, US1/US2/US3/US4 can start in parallel if staffed
- T030 with T031 (docs vs isolation tests, different files)

---

## Parallel Example: User Story 1

```bash
# Launch all tests for User Story 1 together:
Task: "Add public-contract coverage for the lan_address override in crates/sdk/tests/public_api.rs"
Task: "Add deterministic creation coverage with injected ports in crates/sdk/src/manager.rs tests"

# Launch host detection and test scaffolding together once T014 lands:
Task: "Implement default-uplink detection in crates/sdk/src/adapters/network/linux.rs"
Task: "Add failure-path coverage for unprivileged mutations in crates/sdk/tests/failure_paths.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently (Wi-Fi LAN creation, dual reachability, stopped result)
5. Deploy/demo if ready

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Test independently → Deploy/Demo (MVP!)
3. Add User Story 2 → Test independently → Deploy/Demo
4. Add User Story 3 → Test independently → Deploy/Demo
5. Add User Story 4 → Test independently → Deploy/Demo
6. Each story adds value without breaking previous stories

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: User Story 1 (host setup + guest setup + coordinator)
   - Developer B: User Story 2 (privilege diagnostics + ownership)
   - Developer C: User Story 3 + User Story 4 (reconcile + rollback)
3. Stories complete and integrate independently

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Verify tests fail before implementing
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
- Avoid: vague tasks, same file conflicts, cross-story dependencies that break independence
