---

description: "Actionable implementation tasks for the Taumaru Registry artifact integration"
---

# Tasks: Taumaru Registry Artifact Integration

**Input**: Design documents from `/specs/002-taumaru-registry-artifacts/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/sdk-registry.md](./contracts/sdk-registry.md), and
[quickstart.md](./quickstart.md)

**Tests**: Unit, contract, integration, persistence, and failure-path tests are explicitly
required by the feature request and constitution. Test tasks are listed before the implementation
tasks in each user-story phase and should be written to fail before the behavior is implemented.

**Organization**: Tasks are grouped by user story after shared setup and foundational work. Every
task names the files it owns so the work can be executed without rediscovering the design.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Prepare the SDK crate, dependencies, official registry types, and deterministic test
fixtures without changing the CLI lifecycle surface.

- [ ] T001 Add the `tokio`, `reqwest`, `rusqlite`, `serde`, `serde_json`, `sha2`, and `thiserror` dependency declarations with the planned minimal features in `Cargo.toml`, `crates/sdk/Cargo.toml`, and `Cargo.lock`.
- [ ] T002 Create the planned SDK module, migration, fixture, and integration-test paths under `crates/sdk/src/domain/`, `crates/sdk/src/ports/`, `crates/sdk/src/adapters/persistence/`, `crates/sdk/src/adapters/registry/`, `crates/sdk/migrations/`, and `crates/sdk/tests/`, while preserving `example_message` in `crates/sdk/src/lib.rs`.
- [ ] T003 [P] Add a schema-version 1 manifest fixture, small kernel/binary/image payloads, and a reusable local HTTP fixture-server helper in `crates/sdk/tests/fixtures/` and `crates/sdk/tests/support/mod.rs` for deterministic tests without production downloads.
- [ ] T004 [P] Download and vendor the exact official Rust registry definition referenced by `types/registry.rs` into `crates/sdk/src/domain/registry.rs`, preserving its published content and source provenance from `https://artifacts.taumaru.com/v1/types/registry.rs` rather than manually recreating the models.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Establish the typed domain boundary, replaceable ports, normalized SQLite schema, and
constructor wiring required by every user story.

**⚠️ CRITICAL**: No user-story implementation can begin until this phase is complete.

- [ ] T005 [P] Write failing unit tests for caller-path normalization, SDK-home containment, registry digest/size validation, and invalid registry-controlled path rejection in `crates/sdk/src/domain/artifact.rs` and `crates/sdk/src/manager.rs`.
- [ ] T006 [P] Define the public non-panicking `SdkError` enum with home, input, registry, filesystem, task, SQLite, migration, integrity, stale/missing-inventory, and compatibility variants in `crates/sdk/src/error.rs`, including source errors and stable context fields without output or logging side effects.
- [ ] T007 Implement the local artifact, progress, phase, disposition, download-result, and installed-binary types in `crates/sdk/src/domain/artifact.rs` and expose their module through `crates/sdk/src/domain/mod.rs`; preserve monotonic member and aggregate byte counters and the `Downloading`, `Verifying`, `Completed`, `AdoptedExisting`, and `SkippedExisting` phases from `contracts/sdk-registry.md`.
- [ ] T008 [P] Define the replaceable registry and inventory boundaries in `crates/sdk/src/ports/artifacts.rs`, `crates/sdk/src/ports/repository.rs`, and `crates/sdk/src/ports/mod.rs`, keeping filesystem, HTTP, and rusqlite details out of domain types.
- [ ] T009 [P] Write failing SQLite unit and persistence tests for migration idempotence, checksum drift, foreign-key enforcement, uniqueness, nullable kernel download references, ordered child metadata, and the distribution-kernel many-to-many relationship in `crates/sdk/tests/sqlite_persistence.rs`.
- [ ] T010 Create `crates/sdk/migrations/0001_artifact_inventory.sql` with `schema_migrations` and `downloads`; preserve the data-model constraints verbatim: `version` is the primary key and migration numbers are monotonically increasing; `name` is a required migration filename; `checksum` is a required lowercase SHA-256; `applied_at` is a required UTC Unix timestamp; `artifact_key`, `relative_path`, and `absolute_path` are unique; `expected_size_bytes` and `actual_size_bytes` cannot be negative; every retained row has non-null actual size and digest; and `verification_status` is currently `verified`, with failed or invalid files having no inventory row.
- [ ] T011 Extend `crates/sdk/migrations/0001_artifact_inventory.sql` with `kernels`, `binary_packages`, `binary_files`, `distributions`, `distribution_boot_args`, `distribution_images`, `distribution_kernels`, `elf_metadata`, `elf_needed_libraries`, `image_filesystems`, `image_filesystem_features`, and `image_capabilities`; preserve the data-model constraints that registry IDs are required and unique, `kernels.download_id` is a nullable unique foreign key, `(binary_package_id, component_name)` is unique, binary `executable` is `0` or `1`, distribution images are unique within a distribution, ordered children use position keys, and `distribution_kernels` has a composite primary key plus at most one default kernel per distribution.
- [ ] T012 Implement the ordered migration runner and checksum ledger in `crates/sdk/src/adapters/persistence/migrations.rs` and `crates/sdk/src/adapters/persistence/mod.rs`; load separate SQL files in compile-time order, use `CREATE TABLE IF NOT EXISTS` and `CREATE INDEX IF NOT EXISTS`, apply each migration inside a transaction, record `(version, name, checksum, applied_at)`, skip matching versions, and return a typed migration-drift or incompatible-schema error without dropping data.
- [ ] T013 Implement the rusqlite connection setup and normalized inventory repository in `crates/sdk/src/adapters/persistence/sqlite.rs`; enable foreign keys and a bounded busy timeout, keep transactions short, enforce verified-only download rows, insert/delete child rows in foreign-key-safe order, and expose repository operations needed for cache inspection and later artifact persistence.
- [ ] T014 Implement synchronous `MicroVmSdk::new` and registry-override construction in `crates/sdk/src/manager.rs` and `crates/sdk/src/lib.rs`; normalize the explicit caller path, create the SDK-owned `state`, `artifacts`, `tools`, `runtime`, `cache`, and `tmp` directories, open `<home>/state/inventory.db`, run migrations before returning, and never read `HOME` or `TAUMARU_HOME` or construct a Tokio runtime.

**Checkpoint**: The SDK has typed boundaries, official registry models, a durable normalized
schema, idempotent migrations, and a constructor that cannot return before the inventory schema is
ready.

---

## Phase 3: User Story 1 - Discover Available Artifacts (Priority: P1) 🎯 MVP

**Goal**: Let callers list kernels, binary packages, and distributions with all published nested
metadata through the SDK without parsing registry responses themselves.

**Independent Test**: Point the registry override at the local schema-version 1 fixture, call all
three public listing methods, and verify identifiers, versions, architectures, members, URLs,
filenames, sizes, digests, and nested metadata; repeat against unavailable, malformed, and
unsupported-schema fixtures and assert typed errors with no SDK output or panic.

### Tests for User Story 1

- [ ] T015 [P] [US1] Write failing unit tests for schema-version checks, duplicate identifiers, required metadata, URL/path containment, lowercase 64-hex SHA-256 validation, and unsupported registry responses in `crates/sdk/src/adapters/registry/taumaru.rs`.
- [ ] T016 [P] [US1] Write failing async contract tests for `list_kernels`, `list_binaries`, and `list_distributions`, including nested binary files, distribution images, ELF/filesystem metadata, and typed unavailable/malformed/schema errors in `crates/sdk/tests/registry_contract.rs`.
- [ ] T017 [P] [US1] Write failing public API and Rustdoc compile tests for `MicroVmSdk`, the three list methods, deliberate registry-type re-exports, and the unchanged `example_message` in `crates/sdk/tests/public_api.rs`.

### Implementation for User Story 1

- [ ] T018 [US1] Implement the Taumaru registry adapter in `crates/sdk/src/adapters/registry/taumaru.rs`, `crates/sdk/src/adapters/registry/mod.rs`, and `crates/sdk/src/adapters/mod.rs`; reuse one asynchronous `reqwest::Client`, fetch the canonical manifest, call `error_for_status`, decode through `domain/registry.rs`, validate schema and metadata before returning, and keep HTTP failures typed and silent.
- [ ] T019 [US1] Implement registry-backed listing orchestration in `crates/sdk/src/manager.rs` and `crates/sdk/src/ports/artifacts.rs`; expose current manifest collections without stale fallback, preserve every official identifier/version/architecture/source/integrity field and nested member, and reject invalid entries before filesystem or database writes.
- [ ] T020 [US1] Complete the public facade and Rustdoc re-exports in `crates/sdk/src/lib.rs`, `crates/sdk/src/domain/mod.rs`, and `crates/sdk/src/error.rs`; expose only the stable list-facing registry and local types required by `contracts/sdk-registry.md` while keeping adapter and SQLite details private.

**Checkpoint**: User Story 1 is independently usable when the fixture contract tests and public API
tests pass for all three collections and their typed failure cases.

---

## Phase 4: User Story 2 - Download Verified Artifacts (Priority: P1)

**Goal**: Download kernels, every file in a binary package, and every image in a distribution into
the explicit SDK home with live progress, atomic publication, executable permissions, integrity
verification, and normalized inventory updates.

**Independent Test**: Against the local fixture registry and temporary SDK home, exercise a fresh
download, correct-file adoption, complete cache skip, wrong-file replacement, interrupted or
corrupt transfer, multi-file partial failure, and distribution compatibility persistence; inspect
paths, progress events, hashes, sizes, modes, and SQLite rows.

### Tests for User Story 2

- [ ] T021 [P] [US2] Write failing unit tests for the three cache decisions—correct file plus complete relationship skips, correct file without a relationship adopts, and missing/wrong file removes records before verified replacement—in `crates/sdk/src/manager.rs`.
- [ ] T022 [P] [US2] Write failing unit tests for monotonic member/aggregate progress, terminal phases, cached/adopted events, and callback payload identity in `crates/sdk/src/domain/artifact.rs`.
- [ ] T023 [P] [US2] Write failing async download-flow tests for kernel downloads, multi-file binary packages, multi-image distributions, no second transfer on cache hits, adoption of an existing correct file, atomic final paths, and executable mode in `crates/sdk/tests/download_flow.rs`.
- [ ] T024 [P] [US2] Write failing failure-path tests for interrupted, truncated, oversized, wrong-digest, unsafe-path, permission, and callback-independent failures; assert typed errors, no invalid final file, no unverified binary record, no SDK output, and no panic in `crates/sdk/tests/failure_paths.rs`.
- [ ] T025 [P] [US2] Extend persistence tests for one physical row per member, binary package components, distribution images, ordered boot arguments, immediate registry-only kernel references, compatibility links, and full invalid-kernel cleanup in `crates/sdk/tests/sqlite_persistence.rs`.

### Implementation for User Story 2

- [ ] T026 [US2] Implement deterministic SDK-home paths, registry-component validation, unique temporary siblings, per-target coordination, and safe cleanup in `crates/sdk/src/manager.rs`; keep every final and temporary path below the explicit home and preserve the category layout from `data-model.md` and `quickstart.md`.
- [ ] T027 [US2] Implement bounded async response streaming, `tokio::fs` file writes, incremental `sha2` hashing, progress callback emission, executable mode application, flush/sync, size and SHA-256 verification, and atomic rename in `crates/sdk/src/manager.rs` and `crates/sdk/src/domain/artifact.rs`.
- [ ] T028 [US2] Implement the cache/reconciliation state machine in `crates/sdk/src/manager.rs` and `crates/sdk/src/ports/repository.rs`; calculate actual size and digest before every decision, distinguish complete database relationships from missing ones, adopt correct files without transfer, skip complete verified files, and remove invalid targets and records before replacement.
- [ ] T029 [US2] Implement verified physical-download persistence and deletion in `crates/sdk/src/adapters/persistence/sqlite.rs`; persist expected/actual size and lowercase SHA-256, absolute and relative paths, source metadata, timestamps, and `verification_status = verified`, and remove physical rows safely when a member is invalid or replacement fails.
- [ ] T030 [US2] Implement logical kernel, binary package/file, distribution/image, nested metadata, and compatibility relationship upserts in `crates/sdk/src/adapters/persistence/sqlite.rs`; ensure a registry-only kernel can have `download_id IS NULL`, a downloaded invalid kernel deletes its `distribution_kernels` rows before its kernel row, and no foreign-key or uniqueness violation is hidden.
- [ ] T031 [US2] Implement `download_kernel` in `crates/sdk/src/manager.rs` and expose its verified result in `crates/sdk/src/domain/artifact.rs`; fetch the selected registry entry, apply the cache state machine, publish only after size/hash verification, persist the kernel relation, and return typed acquisition or integrity errors.
- [ ] T032 [US2] Implement `download_binary` in `crates/sdk/src/manager.rs` and its result types in `crates/sdk/src/domain/artifact.rs`; process every `BinaryPackage.files` member, preserve executable indication and declared mode where supported, persist every verified component, and report the package complete only after all members pass.
- [ ] T033 [US2] Implement `download_distribution` in `crates/sdk/src/manager.rs` and its result types in `crates/sdk/src/domain/artifact.rs`; process every `Distribution.images` member, persist ordered boot arguments and all supported/default kernel relationships immediately, and do not implicitly download the default kernel.
- [ ] T034 [US2] Enforce aggregate readiness and partial-failure semantics in `crates/sdk/src/manager.rs`, `crates/sdk/src/adapters/persistence/sqlite.rs`, and `crates/sdk/tests/download_flow.rs`; allow other verified members to remain reusable, remove all records for a failed invalid member, and never return a package or distribution as ready before every required member is verified.

**Checkpoint**: User Story 2 is independently usable when all cache, transfer, progress, integrity,
multi-member, permission, atomicity, and SQLite relationship tests pass without SDK output or
panic.

---

## Phase 5: User Story 3 - Resolve Installed Binaries Reliably (Priority: P2)

**Goal**: Resolve a verified runtime binary from durable SQLite inventory by package and component
identity after process restart, while rejecting deleted, replaced, or mismatched paths.

**Independent Test**: Download two package versions/architectures with multiple components,
recreate `MicroVmSdk` with the same home, resolve each exact identity, then delete or mutate one
file and assert a typed stale/integrity error instead of an unsafe path.

### Tests for User Story 3

- [ ] T035 [P] [US3] Write failing unit tests for package/component/version/architecture identity queries, required verified relationships, missing records, and path revalidation in `crates/sdk/src/adapters/persistence/sqlite.rs`.
- [ ] T036 [P] [US3] Write failing restart and separation tests for multiple versions, architectures, and components in `crates/sdk/tests/sqlite_persistence.rs`; close and recreate the SDK and verify durable resolution without directory scanning.
- [ ] T037 [P] [US3] Write failing public failure-path tests for deleted/replaced binary files, incompatible or missing inventory, typed stale/integrity errors, no panic, and no unsolicited output in `crates/sdk/tests/failure_paths.rs` and `crates/sdk/tests/public_api.rs`.

### Implementation for User Story 3

- [ ] T038 [US3] Implement the durable binary lookup query and repository mapping in `crates/sdk/src/ports/repository.rs` and `crates/sdk/src/adapters/persistence/sqlite.rs`; require the complete `downloads` plus `binary_files` relationship and distinguish package ID, component, version, architecture, and verified path.
- [ ] T039 [US3] Implement `resolve_binary` in `crates/sdk/src/manager.rs`; revalidate that the recorded path remains below the SDK home and that its current size and SHA-256 match the verified inventory before returning `InstalledBinary`, otherwise return a typed stale or integrity error.
- [ ] T040 [US3] Complete public resolver exports and Rustdoc contract examples in `crates/sdk/src/lib.rs`, `crates/sdk/src/domain/artifact.rs`, and `crates/sdk/tests/public_api.rs`, preserving the existing bootstrap API and keeping resolution independent of process-local memory or directory scans.

**Checkpoint**: User Story 3 is independently usable when restart, identity-separation, path
mutation, and public API tests pass against the durable SQLite inventory.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Close documentation, safety, regression, and quality-gate gaps across the SDK.

- [ ] T041 Audit the public SDK boundary for Rustdoc completeness, typed failure returns, absence of `unwrap`, `expect`, `panic!`, assertions, output, logging, tracing, process termination, and hidden home selection in `crates/sdk/src/lib.rs`, `crates/sdk/src/error.rs`, `crates/sdk/src/manager.rs`, and `crates/sdk/src/adapters/`.
- [ ] T042 Update the implementation validation steps and database review expectations in `specs/002-taumaru-registry-artifacts/quickstart.md` to match the final test targets, verified-only inventory behavior, full invalid-kernel cleanup, and no production artifact downloads in default tests.
- [ ] T043 Run `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features`; resolve any failures in `Cargo.toml`, `Cargo.lock`, `crates/sdk/`, and `crates/cli/` without changing the CLI lifecycle boundary.

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 — Setup**: T001 precedes T002; T003 and T004 can run in parallel after T002.
- **Phase 2 — Foundational**: T005, T006, T008, and T009 can begin after the scaffold; T007 depends on T005; T010 and T011 implement the schema tested by T009; T012 depends on T010/T011; T013 depends on T012; T014 depends on T013 and blocks all stories.
- **Phase 3 — US1**: T015–T017 are parallel test-first tasks after T014; T018 precedes T019; T020 completes the public surface.
- **Phase 4 — US2**: T021–T025 are parallel test-first tasks after T020; T026–T030 establish transfer/persistence infrastructure; T031–T033 implement the three downloads; T034 closes aggregate semantics.
- **Phase 5 — US3**: T035–T037 are parallel test-first tasks after T034; T038 precedes T039; T040 completes the public resolver.
- **Phase 6 — Polish**: T041–T043 depend on the desired user stories and their focused tests.

### User Story Dependencies

- **US1 (P1)**: Depends on Phase 2 and has no dependency on another story; it is the MVP listing increment.
- **US2 (P1)**: Depends on US1's registry adapter and public manifest contract (T018–T020), then adds download and inventory behavior.
- **US3 (P2)**: Depends on US2's verified binary persistence (T029–T034) because resolution requires durable `downloads` and `binary_files` rows.

### Parallel Opportunities

- T003 and T004 can run in parallel after the SDK paths exist.
- T005, T006, T008, and T009 cover different foundation files and can be parallelized after T002.
- T015, T016, and T017 can be written in parallel before the registry adapter implementation.
- T021, T022, T023, T024, and T025 are independent test files and can be written in parallel before download implementation.
- T035, T036, and T037 are independent resolver test surfaces and can be written in parallel.

## Parallel Execution Examples

### User Story 1

```text
Task T015: Unit-test manifest/schema validation in crates/sdk/src/adapters/registry/taumaru.rs
Task T016: Contract-test all listing operations in crates/sdk/tests/registry_contract.rs
Task T017: Test public listing exports in crates/sdk/tests/public_api.rs
```

### User Story 2

```text
Task T021: Unit-test cache decisions in crates/sdk/src/manager.rs
Task T022: Unit-test progress events in crates/sdk/src/domain/artifact.rs
Task T023: Test download flows in crates/sdk/tests/download_flow.rs
Task T024: Test failure paths in crates/sdk/tests/failure_paths.rs
Task T025: Test SQLite relationships in crates/sdk/tests/sqlite_persistence.rs
```

### User Story 3

```text
Task T035: Unit-test durable identity lookup in crates/sdk/src/adapters/persistence/sqlite.rs
Task T036: Test restart and variant separation in crates/sdk/tests/sqlite_persistence.rs
Task T037: Test stale/replaced paths in crates/sdk/tests/failure_paths.rs and crates/sdk/tests/public_api.rs
```

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1 and Phase 2 so construction, official types, and migrations are ready.
2. Write and pass the US1 registry unit, contract, and public API tests.
3. Complete the registry adapter and three listing operations.
4. Stop and validate the listing workflow independently using the local fixture server.

### Incremental Delivery

1. Add US2 cache, transfer, integrity, progress, permissions, and normalized persistence.
2. Validate fresh, adopted, skipped, corrupted, interrupted, and multi-member downloads.
3. Add US3 durable binary resolution and restart/path-integrity checks.
4. Run the complete workspace quality gates and quickstart review.

### Notes

- Every task uses the required `- [ ] [TaskID] [P?] [Story?] description` checklist format.
- `[P]` appears only where files can be changed independently after stated prerequisites.
- Test tasks are intentionally first within each user-story phase and must demonstrate failure before implementation.
- The CLI remains a consumer only; this feature adds no CLI command or duplicate lifecycle engine.
