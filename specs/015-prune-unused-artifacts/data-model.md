# Phase 1 Data Model: Prune Unused Artifacts

## Input

The operation takes no caller input beyond the constructed `MicroVmSdk` (the SDK home comes
from the instance, never from environment variables or arguments). There is no request struct,
mirroring `list_microvms`: prune always scans the whole home.

## Records read

### `microvms` (existing, read-only for prune)

One durable row per existing MicroVM. Prune reads only the three artifact-reference columns;
it never reads or interprets lifecycle state, process liveness, socket state, network,
credential, or runtime child rows:

| Column | Role in prune |
|---|---|
| `distribution_id` | Pairs with `image_id` to form the referenced-image key |
| `image_id` | Pairs with `distribution_id` to form the referenced-image key |
| `kernel_id` | Forms the referenced-kernel key |

The referenced set is two in-memory sets built from one light query
(`SELECT distribution_id, image_id, kernel_id FROM microvms`):

- `referenced_kernels: HashSet<String>` of `kernel_id`;
- `referenced_images: HashSet<(String, String)>` of `(distribution_id, image_id)`.

Existence of the row alone protects its artifacts. Incomplete creations, stopped machines,
never-started machines, and rows whose process died outside the SDK all count as references.

### `kernels` + `downloads` (existing, read for candidates)

Each candidate is a `kernels` row with `download_id IS NOT NULL` joined to its `downloads`
row for the file location and observed size:

| Field | Source | Rules |
|---|---|---|
| `registry_id` | `kernels.registry_id` | Candidate identity; matched against `referenced_kernels` |
| `absolute_path` | `downloads.absolute_path` | Must resolve below the SDK home (`path_is_below_home`), else the candidate becomes a failure entry and is never touched |
| `size_bytes` | `downloads.actual_size_bytes` | Informational; freed bytes always come from the filesystem-observed size at deletion time |

Kernel rows with `download_id IS NULL` are registry-metadata references (`ensure_kernel_reference`)
with no file; they are never candidates and never deleted.

### `distribution_images` + `distributions` + `downloads` (existing, read for candidates)

| Field | Source | Rules |
|---|---|---|
| `distribution_id` | `distributions.registry_id` | First half of the candidate identity |
| `image_id` | `distribution_images.registry_id` | Second half of the candidate identity |
| `absolute_path` | `downloads.absolute_path` | Same below-home guard as kernels |
| `size_bytes` | `downloads.actual_size_bytes` | Informational only |

`distribution_images.download_id` is `NOT NULL`, so every image row is a real download.

### Orphan `downloads` (existing, read for candidates)

A `downloads` row of type `kernel` or `distribution_image` with no referencing `kernels` /
`distribution_images` row (member relation missing). This is the only sense in which
"incomplete/failed" exists in the schema: the `downloads` table only stores
`verification_status = 'verified'` rows, so an orphan is a leftover from a crash between
`persist_download` and the member insert, or from a cancelled transfer that committed the
download row. The orphan's identity is recovered by parsing its stable `artifact_key`
(`kernel:{id}`, `distribution_image:{dist}:{image}`); keys that do not parse become failure
entries and are never deleted. Orphans whose parsed identity matches a live MicroVM reference
are excluded from deletion.

## Records written

### `PruneSummary` (new public type)

Returned on every success path, including the no-op (all vecs empty, all byte counters zero):

| Field | Type | Meaning |
|---|---|---|
| `removed_kernels` | `Vec<String>` | Pruned kernel registry IDs, sorted ascending; includes valid orphans of type `kernel` |
| `removed_images` | `Vec<PrunedImageId>` | Pruned `(distribution_id, image_id)` pairs, sorted by distribution then image; includes valid orphans of type `distribution_image` |
| `skipped_artifact_keys` | `Vec<String>` | Artifact keys skipped because of an actively in-progress transfer, sorted ascending; kept fully intact |
| `freed_bytes_kernels` | `u64` | Sum of filesystem-observed sizes for removed kernels |
| `freed_bytes_images` | `u64` | Sum of filesystem-observed sizes for removed images |
| `freed_bytes_total` | `u64` | Checked sum of the two per-kind counters |

Removal counts are `removed_kernels.len()` / `removed_images.len()`. Counts and byte counters
are derived from what was actually deleted, never estimated. Stale records (row present, file
already absent) appear in the removed lists with zero contributed bytes.

### `PruneFailure` (new public type) and `PruneIncomplete` error payload

One entry per candidate that could not be reclaimed:

| Field | Type | Meaning |
|---|---|---|
| `artifact_key` | `String` | Stable key identifying the failed candidate |
| `reason` | `String` | English cause (unparseable key, path escapes home, non-regular file, filesystem I/O, database failure) |

On partial failure the operation returns the new additive error variant carrying both the
partial summary (everything reclaimed before and during the run, plus the skipped list) and
the failure list, so the caller can repair and retry without losing the record of success.

### Inventory rows deleted (existing shapes, no migration)

Per reclaimed candidate, in this order — file first, then one write transaction replicating
the existing `remove_member` relational cleanup:

- kernel: `distribution_kernels` links for the kernel row, then the `kernels` row, then its
  `downloads` row;
- image: the `distribution_images` row, then its `downloads` row (`elf_metadata`,
  filesystem, and capability children follow existing `ON DELETE CASCADE`);
- orphan: the `downloads` row only.

File-first ordering is load-bearing: a filesystem failure leaves a stale record that the next
prune call reclaims as a zero-byte removal (self-healing per FR-011). Row-first ordering would
leave a stray file with no record, permanently invisible per FR-012.

## State machine

```text
enumerate (kernels + images + orphans, one read snapshot)
        │
        ▼
split by reference sets (existence only; state never consulted)
        │
        ├── referenced ──► untouched (never in any list)
        │
        └── unreferenced ──► per candidate, in deterministic order
                                (kernels by id; images by (dist, image); orphans by key):
                                │
                                ├── path escapes home ──► failure entry, row kept
                                ├── try_lock held (active transfer) ──► skipped list, untouched
                                ├── re-check references (fresh read)
                                │       └── now referenced ──► untouched, in no list
                                ├── stat file
                                │       ├── absent ──► delete row, removed list, +0 bytes
                                │       └── non-regular (symlink/dir) ──► failure entry, row kept
                                ├── delete file fails ──► failure entry, row kept
                                └── delete file ok ──► delete rows if still unreferenced
                                        ├── row delete ok ──► removed list, +observed bytes
                                        └── row delete fails ──► failure entry
                                                (degrades to stale record; next run reclaims it)
```

No failures anywhere: return `Ok(PruneSummary)`. Any failure entries: return the typed
`PruneIncomplete` error carrying the partial summary plus the failure list. The second run
with no intervening changes always reports zero removals.

## Ownership and multi-VM rules

- A single remaining MicroVM reference keeps the artifact, however many other owners were
  deleted. Deleting a MicroVM never deletes artifacts; only prune does, on a later call.
- The per-target lock for each candidate path is held across stat, file delete, and row
  delete, serializing prune against downloads of the same path in the same SDK instance.
  Cross-instance transfers are not detected (documented limitation, same convention as the
  rest of the codebase); concurrent MicroVM deletion is safe in both directions.
- Prune never touches runtime binary packages, supporting artifacts, tool binaries, the state
  database file, or any MicroVM volume, key, socket, or runtime row.
