# SDK Contract: Prune Unused Artifacts

This contract describes the public SDK surface for the feature. It intentionally contains no
CLI command, SQL statement, registry call, or filesystem implementation detail.

## Public types

The types below are re-exported from `crates/sdk/src/lib.rs` (and `domain::mod`) and carry
Rustdoc describing the same invariants.

```rust
pub struct PrunedImageId {
    pub distribution_id: String,
    pub image_id: String,
}

pub struct PruneSummary {
    pub removed_kernels: Vec<String>,
    pub removed_images: Vec<PrunedImageId>,
    pub skipped_artifact_keys: Vec<String>,
    pub freed_bytes_kernels: u64,
    pub freed_bytes_images: u64,
    pub freed_bytes_total: u64,
}

pub struct PruneFailure {
    pub artifact_key: String,
    pub reason: String,
}
```

`removed_kernels` holds kernel registry IDs in ascending order. `removed_images` holds
`(distribution_id, image_id)` pairs ordered by distribution then image. `skipped_artifact_keys`
holds stable artifact keys skipped because of an actively in-progress transfer, in ascending
order; skipped artifacts are kept fully intact. `freed_bytes_total` is the checked sum of the
two per-kind counters. Removal counts are the lengths of the removed vecs. A no-op success
returns empty vecs and zero counters. Stale records (row present, file already absent) appear
in the removed lists and contribute zero bytes. Types derive `Clone`, `Debug`, `Eq`,
`PartialEq` so callers can assert on them in tests.

## Public operations

### Prune unused artifacts

```rust
impl MicroVmSdk {
    /// Deletes downloaded kernels and images no existing MicroVM references.
    pub async fn prune_unused_artifacts(&self) -> Result<PruneSummary, SdkError>;
}
```

Required behavior:

1. The SDK home comes from the constructed SDK instance. No other input is accepted.
2. Enumerate prune candidates from the host-local inventory: kernel rows with a download,
   distribution-image rows, and orphan `downloads` rows of either type with no member row.
   Kernel metadata references with no download are never candidates. Binaries, supporting
   artifacts, tool binaries, the state database, and MicroVM volumes are never candidates.
3. Build the referenced set from every `microvms` row (all three reference columns),
   regardless of lifecycle state, process liveness, socket responsiveness, or creation
   completeness. Existence alone protects an artifact. References to never-downloaded
   artifacts match nothing.
4. Delete every unreferenced candidate in deterministic order (kernels by ID, images by
   distribution then image, orphans by artifact key): file first, then the inventory rows.
   Artifacts with an actively in-progress transfer are skipped into `skipped_artifact_keys`
   and left fully intact. Referenced artifacts are never touched and appear in no list.
5. On total success return `PruneSummary`. On any deletion failure, continue with the
   remaining candidates and return the typed `PruneIncomplete` error below, which carries
   the partial summary (removed, freed bytes so far, skipped) plus the per-artifact
   failures, so the caller can repair and retry without losing the success record.

Repeated calls are idempotent: the second run with no intervening changes succeeds with
empty removed lists and zero counters. Concurrent same-path downloads in the same SDK
instance are serialized per candidate; MicroVM deletion concurrent with prune is safe in
both directions.

## Error contract

One additive `SdkError` variant is introduced; every other failure reuses the existing
surface:

```rust
pub enum SdkError {
    // ... existing variants unchanged ...
    PruneIncomplete { summary: PruneSummary, failures: Vec<PruneFailure> },
}
```

| Category | Variant and distinguishing data |
|---|---|
| One or more deletions failed | `PruneIncomplete` with the partial `summary` and one `PruneFailure` per failed candidate (unparseable key, escaping path, non-regular file, filesystem I/O, database failure). |
| Invalid or unreadable home, unreadable inventory | Existing `InvalidHome` / `Sqlite` / `Filesystem` / `Migration` paths; nothing is deleted. |

The new variant is additive only: all existing variants, messages, and matching behavior are
unchanged. Its display text is English, names each failed artifact key with its reason, and
repeats the reclaimed counts so the message alone is actionable. No error text includes key
material, registry secrets, or guest data.

## Side-effect contract

- The operation is silent: no stdout/stderr, logging subscriber, tracing event, process
  exit, or global mutable state.
- It never contacts the registry and never emits progress callbacks.
- Regular artifact files of unreferenced candidates are deleted; their inventory rows are
  removed in one transaction per candidate. Nothing else is ever created, moved, renamed,
  or deleted: no VM record, volume, key, socket, runtime, network, bridge, binary, or
  database file is touched.
- Symlinks, directories, or other non-regular files at an artifact path are never followed
  or removed; they become `PruneFailure` entries and the row is kept.
- Files under the artifact directories with no inventory record are left untouched.

## Example usage

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;

// Reclaim everything no existing MicroVM references.
let summary = sdk.prune_unused_artifacts().await?;
assert!(summary.skipped_artifact_keys.is_empty());

// Partial failure keeps the success record for repair and retry.
match sdk.prune_unused_artifacts().await {
    Ok(summary) => println!("freed {} bytes", summary.freed_bytes_total),
    Err(SdkError::PruneIncomplete { summary, failures }) => {
        println!("freed {} bytes, {} failed", summary.freed_bytes_total, failures.len());
    }
    Err(other) => return Err(other),
}
```

The example intentionally uses only public SDK operations. It does not download, create,
start, stop, configure, list, or assemble a host command.
