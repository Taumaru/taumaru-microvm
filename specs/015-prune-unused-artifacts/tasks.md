---

description: "Actionable task list for pruning unused artifacts"

---

# Tasks: Prune Unused Artifacts

**Input**: Design documents from `/specs/015-prune-unused-artifacts/`

**Prerequisites**: [plan.md](./plan.md), [spec.md](./spec.md), [research.md](./research.md),
[data-model.md](./data-model.md), [contracts/sdk-prune.md](./contracts/sdk-prune.md), and
[quickstart.md](./quickstart.md)

**Tests**: Included because the constitution requires tests for domain rules, state transitions,
typed errors, and failure paths, and the plan's test design mandates mixed-ownership, no-op,
idempotency, orphan/stale, active-transfer skip, partial-failure, stray-file, and silence
coverage.

**Organization**: Tasks are grouped by the three user stories (two P1, one P2). Shared domain and
repository-port work is completed before story-specific implementation. No migration, no CLI, no
registry contact, and no other lifecycle operation are part of these tasks.

## Path Conventions

- SDK production code: `crates/sdk/src/`
- SDK integration and public-contract tests: `crates/sdk/tests/`
- Feature design documents: `specs/015-prune-unused-artifacts/`
- The CLI and files from `main` are not modified by these tasks.

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish the test scaffolding the prune work builds on.

- [X] T001 Add a prune test-seeding helper (download kernel plus image rows through `FixtureServer` public downloads, then direct SQL inserts of `microvms` rows referencing a chosen subset) reusable by all prune tests in `crates/sdk/tests/support/mod.rs`.
- [X] T002 [P] Add a prune assertion helper (inventory rows gone versus intact, artifact files gone versus byte-identical, summary ordering and byte-counter checks) reusable by all prune tests in `crates/sdk/tests/support/mod.rs`.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core types and port boundaries that MUST be complete before ANY user story can be implemented.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [X] T003 [P] Add the `PruneSummary` type with the constraint "`removed_kernels` ascending kernel IDs, `removed_images` ordered by distribution then image, `skipped_artifact_keys` ascending artifact keys, `freed_bytes_total` is the checked sum of the two per-kind counters, empty vecs with zero counters on a no-op" in `crates/sdk/src/domain/artifact.rs`, plus the `PrunedImageId` type with the constraint "`distribution_id` plus `image_id` identify one distribution image" in `crates/sdk/src/domain/artifact.rs`, plus the `PruneFailure` type with the constraint "`artifact_key` is the stable key with an English `reason`; never key material" in `crates/sdk/src/domain/artifact.rs`, re-export all three from `crates/sdk/src/domain/mod.rs`, and re-export all three from `crates/sdk/src/lib.rs` with Rustdoc.
- [X] T004 [P] Add the additive `PruneIncomplete` error variant with the constraint "carries the partial `PruneSummary` plus one `PruneFailure` per failed candidate; display text names each failed artifact key with its reason and repeats reclaimed counts; all existing variants unchanged" in `crates/sdk/src/error.rs`.
- [X] T005 Extend the crate-internal `ArtifactRepository` in `crates/sdk/src/ports/repository.rs` with `list_prune_references` (one `SELECT distribution_id, image_id, kernel_id FROM microvms` read), `list_prunable_kernels` (kernels with `download_id IS NOT NULL` plus `downloads` paths, ordered by registry ID), `list_prunable_images` (distribution images plus owning distribution ID plus `downloads` paths, ordered by distribution then image), `list_orphan_artifact_downloads` (kernel/distribution_image `downloads` rows with no member row, ordered by artifact key), `delete_kernel_if_unreferenced`, `delete_image_if_unreferenced`, and `delete_orphan_download_if_unreferenced` (one write transaction each re-checking the reference predicate and applying the `remove_member`-equivalent relational deletes), keeping `inspect_member`, `remove_member`, `resolve_kernel`, and `resolve_distribution_image` unchanged.
- [X] T006 Implement the read half of the new repository methods (`list_prune_references`, `list_prunable_kernels`, `list_prunable_images`, `list_orphan_artifact_downloads`) in `crates/sdk/src/adapters/persistence/sqlite.rs`.
- [X] T007 Implement the write half of the new repository methods (`delete_kernel_if_unreferenced`, `delete_image_if_unreferenced`, `delete_orphan_download_if_unreferenced`) in `crates/sdk/src/adapters/persistence/sqlite.rs` (depends on T006).

**Checkpoint**: The SDK has the public prune types, the additive error variant, and the full repository port with a working SQLite read/write implementation. User-story work may begin.

---

## Phase 3: User Story 1 - Reclaim disk from unused kernels and images (Priority: P1) 🎯 MVP

**Goal**: Delete every downloaded kernel and image (including orphans and stale rows) that no existing MicroVM record references, keep every referenced artifact byte-identical, and return the ordered summary with per-kind and total freed bytes.

**Independent Test**: Seed kernels and images with MicroVM rows referencing only a subset (covering stopped, never-started, and externally-killed owners), run `prune_unused_artifacts()` once, and verify exactly the unreferenced set is gone from disk and inventory while every referenced artifact stays byte-identical and resolvable.

### Tests for User Story 1

- [X] T008 [P] [US1] Add public-contract coverage for `PruneSummary`, `PrunedImageId`, `PruneFailure`, ascending/ordered list constraints, zero-counter no-op shape, and the `PruneIncomplete` variant shape in `crates/sdk/tests/public_api.rs`.
- [X] T009 [P] [US1] Add mixed-ownership coverage (referenced plus unreferenced kernels and images with stopped, never-started, and externally-killed owners: exactly the unreferenced set reclaimed, referenced byte-identical and resolvable, summary ordering and checked byte counters) in `crates/sdk/tests/artifact_prune.rs`.
- [X] T010 [P] [US1] Add orphan and stale coverage (orphan `downloads` rows with no member row reclaimed by parsed key, rows whose file is already absent drop the row as a zero-byte removal with no failure) in `crates/sdk/tests/artifact_prune.rs`.
- [X] T011 [P] [US1] Add reference-protection coverage (shared kernel kept while one of two owners remains, never-downloaded references ignored, kernel metadata rows with `download_id IS NULL` never candidates, stray unrecorded files untouched) in `crates/sdk/tests/artifact_prune.rs`.
- [X] T012 [US1] Implement the prune coordinator core in `crates/sdk/src/manager.rs`: snapshot candidates plus reference sets through `run_repository`, split referenced (dropped silently, in no list) from unreferenced, per-candidate file-first deletion (stat, regular-file guard, file delete, transactional row delete re-verifying the predicate) in deterministic order, checked `u64` byte sums from filesystem-observed sizes, `Ok(summary)` on total success (depends on T006, T007).

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently: one call reclaims exactly the unreferenced set and reports it with correct ordering and accounting.

---

## Phase 4: User Story 2 - Safe no-op when everything is in use (Priority: P1)

**Goal**: Make prune safe to call blindly: fully-referenced and empty homes succeed with zero removals and no writes, repeats are idempotent, and actively-transferring artifacts land in a separate skipped list untouched.

**Independent Test**: Run prune on a fully-referenced home and an empty home (success, no writes, zero removals), run it twice with no changes (second reports zero), and run it with one candidate's target lock held in-instance (that key appears in `skipped_artifact_keys`, fully intact).

### Tests for User Story 2

- [X] T013 [P] [US2] Add no-op and idempotency coverage (fully-referenced home and empty home succeed with no filesystem or inventory writes and zero removals; second run with no changes reports zero further removals) in `crates/sdk/tests/artifact_prune.rs`.
- [X] T014 [P] [US2] Add active-transfer skip coverage (candidate whose per-target lock is held in-instance lands in `skipped_artifact_keys` untouched while other unreferenced candidates are still reclaimed; skipped list ascending) in `crates/sdk/src/manager.rs` unit tests.
- [X] T015 [US2] Wire the skip and guard paths in `crates/sdk/src/manager.rs`: `try_lock` on the candidate's below-home-verified absolute path (held means skipped list, `path_is_below_home` rejection means failure entry with row kept), fresh reference re-check after lock acquisition (newly referenced means untouched, in no list), `symlink_metadata` non-regular guard (failure entry, row kept, link/tree never followed).
- [X] T016 [US2] Add invalid-home coverage (invalid or unreadable home returns a typed error with nothing deleted) in `crates/sdk/tests/failure_paths.rs`.

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently: full reclamation and the safe no-op/skip paths each behave per spec with no stray host effects.

---

## Phase 5: User Story 3 - Survive partial deletion failures without threatening machines (Priority: P2)

**Goal**: Continue past individual deletion failures, never touch referenced artifacts, and return the typed failure carrying the partial summary plus named per-artifact causes so the caller can repair and retry.

**Independent Test**: Seed several unreferenced artifacts with exactly one undeletable (read-only directory), run prune, and verify the rest are reclaimed, referenced artifacts are intact, the error carries the removed list plus freed bytes so far plus the named failure, and a retry after fixing permissions succeeds.

### Tests for User Story 3

- [X] T017 [P] [US3] Add partial-failure coverage (one undeletable file still reclaims the rest, error carries the partial summary with freed bytes so far and names the failed key, retry after repair succeeds, referenced artifacts intact throughout) in `crates/sdk/tests/artifact_prune.rs`.
- [X] T018 [P] [US3] Add unparseable-key coverage (orphan `downloads` row with a malformed artifact key becomes a `PruneFailure` entry with the row kept, never deleted) in `crates/sdk/tests/artifact_prune.rs`.
- [X] T019 [US3] Wire partial-failure aggregation in `crates/sdk/src/manager.rs`: collect failure entries across candidates, continue with the remainder, return `PruneIncomplete { summary, failures }` when any exist (depends on T012, T015).
- [X] T020 [US3] Add operation-silence coverage (no stdout/stderr, logging, tracing, or process exit on success and on partial failure) in `crates/sdk/tests/artifact_prune.rs`.

**Checkpoint**: All user stories should now be independently functional: reclamation, safe no-op/skip, and partial-failure survival each behave per spec with referenced machines never threatened.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Contract consistency, documentation, and quality gates across all stories.

- [X] T021 [P] Add Rustdoc for `prune_unused_artifacts`, `PruneSummary`, `PrunedImageId`, `PruneFailure`, and `PruneIncomplete` covering the no-input contract, existence-only reference rule, orphan scope, file-first ordering, skipped-list meaning, zero-byte stale rule, and idempotency in `crates/sdk/src/manager.rs`, `crates/sdk/src/domain/artifact.rs`, and `crates/sdk/src/error.rs`.
- [X] T022 [P] Verify no CLI source changes, no schema migration, no registry contact, and that all repository text is English, by reviewing the final diff.
- [X] T023 Run `cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` and fix all findings.
- [X] T024 Run the `quickstart.md` validation scenarios (mixed-ownership prune, no-op plus repeat, skip list, partial failure plus retry, failure handling) against the implemented SDK.

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
- **User Story 2 (P1)**: Can start after Foundational (Phase 2) - Builds on the US1 coordinator's candidate/reference wiring (T015 extends T012); independently testable via seeded inventories
- **User Story 3 (P2)**: Can start after Foundational (Phase 2) - Builds on the US1/US2 coordinator wiring (T019 extends T012, T015); independently testable via seeded inventories

### Within Each User Story

- Tests (included per constitution) are written alongside implementation using seeded inventories and doubles
- Domain/port work (Phase 2) before coordinator work (US1)
- Coordinator core (T012) before skip/guard wiring (T015) and failure aggregation (T019)
- Core implementation before failure-path and silence coverage
- Story complete before moving to next priority

### Parallel Opportunities

- All Setup tasks marked [P] can run in parallel
- T003 and T004 can run in parallel (different files: domain vs error)
- T008, T009, T010, and T011 can run in parallel (contract test target vs different coverage areas in the new test file - coordinate edits)
- T013 and T014 can run in parallel (different test targets)
- T017 and T018 can run in parallel (different coverage areas in the same test file - coordinate edits)
- Once Foundational phase completes, all user stories can start in parallel (if team capacity allows)
- T021 and T022 can run in parallel (docs vs diff review)

---

## Parallel Example: User Story 1

```bash
# Launch US1 test coverage together (coordinate same-file edits):
Task: "Add mixed-ownership coverage in crates/sdk/tests/artifact_prune.rs"
Task: "Add orphan and stale coverage in crates/sdk/tests/artifact_prune.rs"
Task: "Add reference-protection coverage in crates/sdk/tests/artifact_prune.rs"

# Launch contract coverage alongside:
Task: "Add public-contract coverage in crates/sdk/tests/public_api.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently (seeded mixed ownership reclaims exactly the unreferenced set with correct ordering and accounting)
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
   - Developer A: User Story 1 (coordinator core, mixed-ownership/orphan/protection tests)
   - Developer B: User Story 2 (no-op/idempotency, skip wiring, invalid home)
   - Developer C: User Story 3 (partial-failure aggregation, unparseable keys, silence)
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
