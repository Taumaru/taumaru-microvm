# Phase 0 Research: Prune Unused Artifacts

## Research goals

Resolve the implementation questions for one SDK-only `prune_unused_artifacts()` operation:
how to enumerate prune candidates from the existing SQLite inventory without a migration,
how to derive the referenced set from MicroVM records, what "incomplete/failed" covers in
the actual schema, how to detect actively transferring artifacts, in which order to delete
files versus rows, and how to carry partial results on failure.

All findings below were verified against the current sources (`manager.rs`, `ports/repository.rs`,
`adapters/persistence/sqlite.rs`, `migrations/0001_artifact_inventory.sql`,
`migrations/0002_microvm_creation.sql`).

## Findings

### Candidate enumeration needs no migration: three read queries over the existing schema

- Downloaded kernels are `kernels` rows with `download_id IS NOT NULL` joined to `downloads`
  for `absolute_path` and `actual_size_bytes`. Kernel rows with `download_id IS NULL` are
  registry metadata references created by `ensure_kernel_reference` (compatibility data with
  no file on disk); they are never candidates and never deleted.
- Downloaded images are `distribution_images` rows joined to `distributions` (for the owning
  registry ID) and `downloads`. `distribution_images.download_id` is `NOT NULL`, so every
  image row is a real download.
- The `downloads` table only ever stores `verification_status = 'verified'` rows (enforced by
  `CHECK`). "Incomplete" in this codebase means a `downloads` row whose member relation is
  missing (`Incomplete` from `inspect_member` / `member_relation_exists`): e.g. a row left
  behind by a crash between `persist_download` and the member insert. The third candidate
  class is therefore orphan `downloads` rows of type `kernel` / `distribution_image` with no
  referencing `kernels` / `distribution_images` row. Orphans are, by construction, referenced
  by no kernel/image row; the prune query additionally excludes any orphan whose parsed
  identity matches a MicroVM reference (see key formats below).
- `distributions`, `binary_packages`, `binary_files`, tool binaries, and every `vm_*` table are
  out of scope per FR-006; no query touches them except `microvms` for the reference set.
- Ordering is deterministic: kernels by `registry_id`, images by
  `(distribution_registry_id, image_registry_id)`, orphans by `artifact_key`.

### Reference set comes from every `microvms` row, regardless of state or completeness

- `microvms` stores `distribution_id`, `image_id`, and `kernel_id` as plain registry-ID strings
  at creation time (`insert_microvm`). There is no join table between VMs and artifacts, so the
  reference set is two in-memory sets built from `list_stored_microvms` (or a lighter
  `SELECT distribution_id, image_id, kernel_id FROM microvms`): one of kernel IDs, one of
  `(distribution_id, image_id)` pairs.
- Lifecycle state, process liveness, socket responsiveness, and creation completeness are all
  irrelevant: even an incomplete row (`require_complete` would fail) protects its artifacts,
  because the row exists. This matches FR-003 exactly and is the point of the clarification.
- A MicroVM referencing a never-downloaded artifact contributes IDs that simply match no
  candidate row; nothing happens for it.

### Artifact key formats are stable and parseable for the orphan guard

- `kernel_member` builds `artifact_key = "kernel:{kernel_id}"`; `distribution_image_member`
  builds `"distribution_image:{distribution_id}:{image_id}"` with `member_name = image_id`.
  Both formats use validated safe components (no `/` or `\`), so splitting on `':'` is
  unambiguous. An orphan whose key does not parse is never deleted; it becomes a `PruneFailure`
  entry instead of a silent skip.

### Active transfers are skipped via the existing per-target lock map (same SDK instance)

- Every download holds the `target_lock` mutex keyed by the candidate's `absolute_path` for
  the whole `download_member` call (inspect, hash, stream, publish). Prune resolves the same
  key — the stored `absolute_path` string, after verifying it stays below the SDK home with
  the existing `path_is_below_home` helper — and calls `try_lock`. A held lock means an
  in-progress transfer in this instance: the candidate goes to the skipped list, untouched.
- Mid-transfer artifacts are additionally invisible to prune for a second reason: `replace_member`
  deletes the inventory row *before* streaming to a `tmp/.<digest>.<ts>.<seq>.part` file and
  publishes with an atomic rename. During transfer there is usually no candidate row at all.
- Accepted limitation, consistent with the existing lock granularity: the lock map is
  per-`MicroVmSdk`-instance (in-memory), so transfers running under a *different* instance or
  process are not detected by `try_lock`. Cross-process coordination in this codebase is already
  limited to SQLite busy-timeout serialization; prune follows the same convention and documents it.
- Alternative considered: OS advisory file locks on artifact paths. Rejected: no such mechanism
  exists anywhere in the codebase, downloads do not take them, and introducing them would change
  the download path for a read-only-by-default operation.

### Delete order is file first, then the inventory row, mirroring `remove_member` cleanup

- Per candidate, while holding the target-lock guard: stat the file (`symlink_metadata`),
  re-check references in a short read transaction, delete the file, then delete the inventory
  row in one write transaction that replicates `remove_member`'s relational cleanup (kernel:
  `distribution_kernels` links, `kernels` row, `downloads` row; image: `distribution_images`
  row, `downloads` row with `elf_metadata` / filesystem children following `ON DELETE CASCADE`;
  orphan: `downloads` row only).
- Rationale: a filesystem failure leaves a *stale record* (row without file), which the next
  prune call reclaims as a zero-byte removal per FR-011 — the failure mode self-heals. Row-first
  ordering would instead leave a *stray file* with no record, which FR-012 makes permanently
  invisible. A database failure after a successful file delete degrades to the same self-healing
  stale record.
- Only regular files are ever deleted. Symlinks, directories, or other types at an artifact path
  become `PruneFailure` entries and the row is kept — prune never follows links or removes trees.
- A missing file at stat time is not a failure: the row is dropped in the same transactional
  cleanup and reported as a zero-byte removal (FR-011).
- Residual race documented, not closed: a MicroVM created between the re-check and the file
  delete (or a creation whose preflight passed before prune's delete) can reference a just-deleted
  artifact. Closing it would require a global creation/prune lock that does not exist and would
  change the creation path; the per-candidate re-check plus per-target locks reduce the window to
  the same class of race the codebase already accepts for concurrent lifecycle operations.
  Concurrent MicroVM *deletion* is safe in both directions (it only grows the unreferenced set).
- Freed bytes come from the filesystem-observed size at stat time, summed with checked `u64`
  arithmetic (mirroring the existing `total_size` helper); already-absent files contribute zero.
- Alternative considered: check-and-delete-row plus file-delete in one step reusing `remove_member`
  with a reconstructed `DownloadSpec`. Rejected: reconstructing a spec requires registry metadata
  prune does not have (expected size, sha, URLs) purely to satisfy a parameter the deletion logic
  does not need; dedicated `*_if_unreferenced` repository methods keep the boundary honest.

### Partial failure needs one new additive error variant

- The clarification requires the error to carry removed identities, freed bytes so far, the
  skipped list, and per-artifact causes. No existing variant holds structured data:
  `Cleanup { primary, failures }` carries only strings and would flatten the summary the spec
  requires callers to retain. The design therefore adds one additive variant, e.g.
  `SdkError::PruneIncomplete { summary: PruneSummary, failures: Vec<PruneFailure> }`
  (exact shape in the contract). Additive enum variants preserve all existing behavior; the
  trait extensions are `pub(crate)`, so there is no public breakage.
- Display text stays English, names the failed artifact keys with reasons, and repeats the
  reclaimed counts so the message alone is actionable without unwrapping fields.

### Placement and side-effect boundaries

- The operation is a method on `MicroVmSdk` in `manager.rs` next to the other lifecycle
  operations, reusing `target_lock`, `run_repository`, `path_is_below_home`, and the
  file-removal style of `remove_invalid_target_if_exists`. No new top-level module: one
  operation with small helpers does not justify a new layer, and every sibling operation lives
  on the manager.
- The `ArtifactRepository` trait grows crate-internal methods only
  (`list_prunable_kernels`, `list_prunable_images`, `list_orphan_artifact_downloads`,
  `delete_kernel_if_unreferenced`, `delete_image_if_unreferenced`,
  `delete_orphan_download_if_unreferenced`). External implementors cannot exist outside the
  crate, so no compatibility break is possible.
- Prune never contacts the registry, never emits progress callbacks, never reads environment
  variables for the home, and never touches the CLI. It is `async` for consistency with every
  other public operation (it uses `run_repository` and Tokio file APIs).

### Test strategy without privileges

- Downloading kernels/images through the public API against the existing `FixtureServer` needs
  no privileges; MicroVM rows are seeded with direct SQL inserts into `inventory.db` (the same
  technique `sqlite_persistence.rs` tests already use), avoiding root/network-dependent creation.
  Deterministic ordering makes assertions exact.
- The active-transfer skip is covered as a manager unit test (crate-internal): seed rows, hold
  the `target_lock` guard for a candidate path, run prune, assert the skipped list.
- Failure paths (undeletable file via read-only directory, stale record via manual file removal,
  unparseable orphan key) and the partial-failure payload shape are covered in the integration
  suite plus a `public_api.rs` export test for the new types.

No unresolved technical questions remain for Phase 1 design.
