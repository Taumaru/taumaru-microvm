# Implementation Plan: Prune Unused Artifacts

**Branch**: `015-prune-unused-artifacts` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/015-prune-unused-artifacts/spec.md`

## Summary

Implement one SDK-only `prune_unused_artifacts()` operation that reclaims disk by deleting
downloaded distro kernels and distro images no existing MicroVM record references. The
operation takes no input (the SDK home comes from the existing `MicroVmSdk` instance),
enumerates candidates from the SQLite inventory (downloaded kernels, downloaded images, and
orphan `downloads` rows of either type, including incomplete/failed records), builds the
referenced set from every `microvms` row regardless of lifecycle state, skips artifacts with
an actively in-progress transfer into a separate skipped list, deletes the rest file-first
then rows (replicating the existing `remove_member` relational cleanup), and returns a
`PruneSummary` with removed identities per kind plus freed bytes per kind and in total.
Partial deletion failures continue with the remaining candidates and return a new additive
`SdkError::PruneIncomplete` carrying the partial summary plus per-artifact causes. No CLI
changes, no migration, and no registry contact are part of this feature.

## Technical Context

**Language/Version**: Rust 2024 edition, using the repository's stable toolchain.

**Primary Dependencies**: Existing `tokio` (`fs`, `macros`, `rt`, `sync`), `rusqlite`
(bundled SQLite), `serde`/`serde_json`, `thiserror`; no new crates. Filesystem work uses
`tokio::fs` plus std `symlink_metadata`; concurrency reuses the existing per-target
`target_lock` map with `try_lock` for active-transfer detection. No shell, no new process
mechanics.

**Storage**: Existing SQLite inventory under the explicit SDK home; no schema migration is
needed. Reads cover `microvms` (reference columns only), `kernels` + `downloads`,
`distribution_images` + `distributions` + `downloads`, and orphan `downloads` rows.
Writes replicate the existing `remove_member` relational cleanup per candidate (kernel:
`distribution_kernels` links, `kernels` row, `downloads` row; image:
`distribution_images` row, `downloads` row with `ON DELETE CASCADE` children; orphan:
`downloads` row only). Artifact files stay under `artifacts/kernels/` and
`artifacts/rootfs/`; prune deletes only regular files at recorded paths verified below the
SDK home, never the state database or any VM volume.

**Testing**: Existing SDK unit/integration suites (`crates/sdk/tests/`, manager tests)
extended with deterministic prune tests using the existing `FixtureServer` for downloads
plus direct SQL seeding of `microvms` rows (the technique `sqlite_persistence.rs` already
uses), avoiding root/network-dependent creation. Manager unit tests cover the
active-transfer skip via the crate-internal lock map. Run the repository Cargo quality
gates.

**Target Platform**: Linux hosts with permission to read the inventory and delete owned
artifact files. Hosts without read/delete permission return typed SDK errors.

**Project Type**: Reusable SDK library in `crates/sdk`; the CLI is not modified for this
feature.

**Performance Goals**: Prune scans the inventory once and stats each candidate once; no
hashing, no registry traffic, no background worker. Runtime scales with candidate count
(tens of artifacts on a normal host). A no-op (fully referenced or empty home) returns
after the read snapshot with no writes.

**Constraints**: The SDK is silent and returns typed `Result` errors. It must not read home
paths from environment variables, accept any input beyond the constructed instance, contact
the registry, emit progress callbacks, touch referenced artifacts, follow symlinks or
remove trees, delete binaries/supporting artifacts/tool binaries/VM volumes, or change any
CLI surface. Freed bytes come from filesystem-observed sizes with checked `u64`
arithmetic; absent files contribute zero.

**Scale/Scope**: Multiple independent VMs per host; prune is always home-wide with no
single-VM assumption. This feature owns the SDK prune operation only; download, creation,
start, stop, listing, and CLI presentation remain untouched.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle behavior; CLI remains thin | PASS: design is SDK-only | PASS: no CLI source changes or duplicate orchestration |
| Public SDK is typed, silent, non-panicking, and side-effect explicit | PASS: typed errors and repository boundary required | PASS: no output/logging/global state; file and row deletes are explicit; partial failure is a typed payload |
| Domain is independent from infrastructure | PASS: domain/ports/adapters separation selected | PASS: new public types live in domain; SQLite/file mechanics stay behind ports/adapters |
| SQLite is local source of truth | PASS: inventory rows plus VM records decide candidacy | PASS: no migration; reference set and candidates derive from existing tables only |
| Firecracker/firectl remain replaceable implementation details | PASS: no runtime work in prune | PASS: prune never touches the runtime port, sockets, or processes |
| Registry and artifact boundaries are explicit | PASS: no registry contact; artifact scope is kernels/images only | PASS: binaries, supporting artifacts, and tools are excluded by construction |
| Multiple MicroVMs are supported | PASS: home-wide scan with a per-VM-agnostic reference set | PASS: shared references keep artifacts; per-target locks serialize same-path work |
| Public contracts and compatibility changes are documented | PASS: contract/data-model artifacts planned | PASS: Rustdoc, contract, data-model, and quickstart are included; new error variant is additive |
| Project text is English | PASS | PASS |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/015-prune-unused-artifacts/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── sdk-prune.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── src/
│   │   ├── lib.rs              # Re-export PruneSummary, PrunedImageId, PruneFailure
│   │   ├── manager.rs          # prune_unused_artifacts coordinator
│   │   ├── error.rs            # Additive PruneIncomplete variant
│   │   ├── domain/
│   │   │   ├── mod.rs          # Re-export prune types
│   │   │   ├── artifact.rs     # PruneSummary, PrunedImageId, PruneFailure
│   │   │   └── microvm.rs      # Unchanged (reference columns only)
│   │   ├── ports/
│   │   │   └── repository.rs   # Crate-internal prune read/delete methods
│   │   └── adapters/
│   │       └── persistence/
│   │           └── sqlite.rs   # Prune queries + transactional deletes
│   └── tests/
│       ├── artifact_prune.rs   # Prune lifecycle, idempotency, skip, failures
│       ├── public_api.rs       # Prune type exports
│       └── failure_paths.rs    # Prune failure payload paths
└── cli/
    └── (untouched by this feature)
```

**Structure Decision**: All behavior lives in the SDK crate behind the existing
domain/ports/adapters layout. The manager orchestrates; the repository port owns inventory
reads and transactional deletes; the SQLite adapter implements them; new public types live
in `domain::artifact`. No new top-level modules or CLI changes.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Candidate enumeration needs no migration: `kernels` rows with `download_id IS NOT NULL`
   plus `downloads`, `distribution_images` plus `distributions` plus `downloads`, and orphan
   `downloads` rows of type `kernel`/`distribution_image` with no member row. Kernel rows
   with `download_id IS NULL` are metadata references, never candidates.
2. "Incomplete/failed" in the real schema means orphan `downloads` rows (member relation
   missing after a crash or cancelled transfer), because `downloads` only stores
   `verification_status = 'verified'`. The orphan's identity is parsed from the stable
   `artifact_key` (`kernel:{id}`, `distribution_image:{dist}:{image}`); unparseable keys
   become failure entries, never deletions.
3. The reference set is two in-memory sets (kernel IDs, `(distribution, image)` pairs) from
   one light `microvms` query over all rows; state, liveness, socket, and completeness are
   never consulted. References to never-downloaded artifacts match nothing.
4. Active transfers in the same SDK instance are detected with `try_lock` on the existing
   per-target lock keyed by the candidate's absolute path (the same mutex downloads hold
   across inspect/stream/publish); held means skipped into the separate list. Mid-transfer
   artifacts are additionally usually row-less because `replace_member` deletes the row
   before streaming to `tmp/.*.part` and publishes atomically. Cross-instance transfers are
   not detected (documented, same convention as the codebase); no OS file locks are added.
5. Delete order is file first, then the inventory rows in one transaction replicating
   `remove_member` cleanup, so a filesystem failure degrades to a self-healing stale record
   (next run reclaims it at zero bytes) while row-first would strand invisible stray files.
   Only regular files are deleted; symlinks/dirs become failures with the row kept; absent
   files drop the row as zero-byte removals. Freed bytes are filesystem-observed with
   checked `u64` sums.
6. Partial failure needs one new additive error variant (e.g. `PruneIncomplete { summary,
   failures }`); `Cleanup` carries only strings and cannot hold the summary the spec
   requires. The variant is additive, so no existing behavior changes.
7. Placement is `manager.rs` reusing `target_lock`, `run_repository`, `path_is_below_home`,
   and `remove_invalid_target_if_exists` file-removal style; the trait grows
   crate-internal methods only, so no public breakage. Tests seed `microvms` rows with
   direct SQL through `FixtureServer` downloads, needing no privileges.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): candidate sources, reference sets, result and failure
  records, row-delete shapes, state machine, and ownership rules.
- [contracts/sdk-prune.md](./contracts/sdk-prune.md): public Rust types, operation, errors,
  side-effect contract, and usage.
- [quickstart.md](./quickstart.md): SDK usage, host prerequisites, failure behavior, and gates.

### Public SDK boundary

Extend the `MicroVmSdk` facade with:

- `prune_unused_artifacts(&self) -> Result<PruneSummary, SdkError>`.

Add `PruneSummary { removed_kernels, removed_images, skipped_artifact_keys,
freed_bytes_kernels, freed_bytes_images, freed_bytes_total }`, `PrunedImageId {
distribution_id, image_id }`, and `PruneFailure { artifact_key, reason }` to
`domain::artifact`, re-exported deliberately from `crates/sdk/src/lib.rs` (and
`domain::mod`) with Rustdoc describing the ordering, accounting, and skip invariants.
Derive `Clone, Debug, Eq, PartialEq` for test assertability. Do not add CLI commands or
any other operation in this feature.

### Prune coordinator

Implement the prune flow in `manager.rs` (or a focused private manager module) in this
order:

1. Snapshot candidates with one repository call returning kernel candidates
   `(registry_id, absolute_path)`, image candidates
   `(distribution_id, image_id, absolute_path)`, and orphan candidates
   `(artifact_key, artifact_type, absolute_path)`, each deterministically ordered.
2. Load the reference sets with one light `microvms` read (all rows, three columns);
   existence alone protects — never consult state, liveness, or completeness.
3. Split candidates: referenced identities are dropped silently (in no list); parseable
   orphans matching a live reference are dropped the same way; unparseable orphan keys are
   collected as failure entries without further work.
4. For each unreferenced candidate in deterministic order, resolve the per-target lock for
   its absolute path (rejecting paths that escape the SDK home as failure entries, row
   kept):
   - `try_lock` fails (held by a download in this instance): record the artifact key in
     the skipped list, untouched.
   - `try_lock` succeeds: hold the guard across the rest of the candidate; re-check
     references with a fresh read (a concurrent creation may have claimed it — now
     referenced means untouched, in no list).
   - `symlink_metadata`: absent means delete the rows and record a zero-byte removal;
     non-regular (symlink, dir, other) means a failure entry with the row kept; regular
     files proceed.
   - Delete the file; on I/O failure record a failure entry with the row kept.
   - On file success, delete the inventory rows in one transaction that re-verifies the
     unreferenced predicate inside the same transaction; on row success record the
     observed size, on row failure record a failure entry (degrades to a stale record
     the next run reclaims).
5. Return `Ok(summary)` when no failure entries exist; otherwise return the additive
   `PruneIncomplete { summary, failures }` carrying the partial summary (removed, freed
   bytes so far, skipped) plus every failure entry.

The coordinator must never use in-memory state as the source of truth across calls. A
fresh SDK instance must recover candidates and references from SQLite alone (the lock map
only adds same-instance transfer detection, never identity).

### Persistence and transaction boundaries

No migration is added. New crate-internal repository methods cover every access:

- `list_prune_references` (or reuse `list_stored_microvms` if the lighter query is
  deferred): one `SELECT distribution_id, image_id, kernel_id FROM microvms`;
- `list_prunable_kernels`, `list_prunable_images`, `list_orphan_artifact_downloads`:
  read-only joins returning paths and identities in deterministic order;
- `delete_kernel_if_unreferenced`, `delete_image_if_unreferenced`,
  `delete_orphan_download_if_unreferenced`: one write transaction each that re-checks the
  reference predicate, applies the `remove_member`-equivalent relational deletes, and
  commits; orphans delete the `downloads` row only.

Each closure covers one durable transition only and is never held open across file I/O:
the transaction opens after the file delete succeeds. Read snapshots use short-lived
connections through the existing `run_repository` blocking-task bridge.

### Error and side-effect design

Add one public variant and reuse the existing surface for everything else:

| Situation | Variant |
|---|---|
| One or more candidate deletions failed | New `PruneIncomplete { summary, failures }`; display text names each failed key with its reason and repeats reclaimed counts |
| Invalid or unreadable home, unreadable inventory | Existing `InvalidHome` / `Sqlite` / `Filesystem` / `Migration` paths; nothing deleted |
| Candidate path escapes the SDK home | `PruneFailure` entry inside `PruneIncomplete`; row kept, file untouched |
| Non-regular file at an artifact path | `PruneFailure` entry; row kept, link/tree never followed |
| Unparseable orphan artifact key | `PruneFailure` entry; row kept, never deleted |

The operation never contacts the registry, never emits progress, never reads environment
variables, never creates/moves/renames any file, and never touches VM records, volumes,
keys, sockets, runtimes, networks, bridges, binaries, or the database file itself.

### Test implementation

Add or update SDK tests for:

- mixed ownership: referenced plus unreferenced kernels/images (including stopped,
  never-started, and externally-killed owners) — exactly the unreferenced set reclaimed,
  referenced byte-identical and resolvable;
- fully-referenced and empty-home no-ops: success, no writes, zero removals;
- idempotent repeat: first run reclaims, second reports zero;
- incomplete/failed records and stale rows: orphans reclaimed, absent files drop rows at
  zero bytes with no failure;
- active-transfer skip: lock held in-instance means the skipped list, untouched;
- partial failure: one undeletable file still reclaims the rest, error carries the partial
  summary plus named failures, retry succeeds;
- unrecorded stray files untouched; never-downloaded references ignored;
- shared kernel kept while one of two owners remains;
- unknown/unreadable home: typed error, nothing deleted;
- operation silence: no stdout/stderr, logging, tracing, or process exit;
- public API exports for the three new types plus the new error variant shape.
