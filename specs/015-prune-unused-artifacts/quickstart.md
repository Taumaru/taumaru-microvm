# Quickstart: Prune Unused Artifacts

This feature is consumed through the Rust SDK. It does not add a CLI command. Artifact
acquisition and MicroVM creation are separate operations and must complete before prune can
reclaim anything.

## Host prerequisites

The host must be Linux with an explicit SDK home containing `state/inventory.db` and the
downloaded artifacts. The SDK home must be supplied explicitly. It is the directory
containing the inventory, artifacts, tools, and VM volumes. The SDK does not read `HOME` or
another environment variable to choose it.

No privileges beyond reading the inventory and deleting owned artifact files are required.
Prune never touches the network, the registry, VM volumes, keys, sockets, or runtime state.

## Prepare downloaded artifacts

Use the existing SDK download operations first. Prune does not acquire, verify, or repair
anything; it reads the inventory rows those operations committed:

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;
```

MicroVM references come from the inventory rows written by creation. Nothing needs to run:
stopped, never-started, and externally-killed machines still protect their artifacts.

## Prune unreferenced kernels and images

```rust
let summary = sdk.prune_unused_artifacts().await?;
```

On success:

- `summary.removed_kernels` lists pruned kernel IDs in ascending order;
- `summary.removed_images` lists pruned `(distribution_id, image_id)` pairs ordered by
  distribution then image;
- `summary.skipped_artifact_keys` lists artifacts skipped because of an actively
  in-progress transfer, kept fully intact;
- `summary.freed_bytes_kernels`, `summary.freed_bytes_images`, and
  `summary.freed_bytes_total` report reclaimed bytes from filesystem-observed sizes;
- the removed vec lengths are the removal counts.

Pruning when everything is referenced (or nothing is downloaded) succeeds with empty vecs
and zero counters, changing nothing. Repeating a prune with no changes in between reports
zero further removals.

## Expected failure handling

Prune fails before any deletion when the home is invalid or the inventory cannot be read.
When individual deletions fail (permissions, locks, I/O), prune continues with the
remaining candidates and returns `SdkError::PruneIncomplete`, which carries the partial
summary (removed, freed bytes so far, skipped) plus one `PruneFailure` per failed
candidate with its artifact key and reason. Fix the cause and run prune again; a
filesystem failure degrades to a stale record that the retry reclaims as a zero-byte
removal. Every expected failure is a typed `SdkError` with no panic, no process exit,
and no stdout/stderr output.

## Verification commands

Run the repository gates after implementation:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The automated suite covers mixed-ownership reclamation (referenced intact, unreferenced
gone), fully-referenced and empty-home no-ops, idempotent repeats, incomplete/failed
records and stale rows, the active-transfer skip list, partial-failure payloads that keep
the success record, unrecorded stray files left untouched, and operation silence through
the existing fixture server plus direct inventory seeding. No privileged host setup is
needed: MicroVM rows are seeded with direct SQL inserts, never through root-dependent
creation.
