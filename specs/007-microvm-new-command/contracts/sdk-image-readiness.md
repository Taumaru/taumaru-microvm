# SDK Contract: Distribution Image Readiness Query

This contract describes the single additive SDK surface for this feature. It
contains no CLI command, shell command, SQLite statement, or registry HTTP
implementation detail. All existing acquisition, persistence, and creation
behavior is unchanged.

## New public operation

```rust
impl MicroVmSdk {
    /// Reports whether a distribution image is verified locally.
    pub async fn is_distribution_image_ready(
        &self,
        distribution_id: &str,
        image_id: &str,
    ) -> Result<bool, SdkError>;
}
```

The method is re-exported through the existing `MicroVmSdk` type; no new
public type is introduced. Rustdoc states the same invariants as this
contract.

Required behavior:

1. Validate both IDs as non-blank registry identifiers before any registry
   or inventory access.
2. Resolve the distribution from the current manifest; an unknown
   distribution returns `NotFound { kind: "distribution", .. }`.
3. Resolve the image within that distribution's published image list; an
   image absent from that distribution returns
   `NotFound { kind: "distribution image", .. }`.
4. Resolve the existing per-image inventory relationship and recheck the
   physical file (presence, size, digest) through the same verification path
   creation uses.
5. Return `Ok(true)` only when the inventory relationship exists and the
   file is present with matching size and digest. Return `Ok(false)` for
   missing, incomplete, or stale entries — the same conditions that make
   creation return `ArtifactPrerequisite`.
6. Perform no download, no mutation, no cache repair, and no persistence
   write on any path.
7. Stay silent: no stdout/stderr, logging subscriber, tracing event, process
   exit, or global mutable state. All expected failures are typed `SdkError`
   values.

## Preserved behavior

- `resolve_binary` semantics are unchanged; this query is its per-image
  counterpart for the downloaded marker.
- `download_kernel`, `download_binary`, `download_distribution`,
  `download_distribution_image` (plus cancellation variants), and
  `create_microvm` are unaffected.
- Persistence keys, on-disk layout, and
  `resolve_distribution_image(distro, image)` semantics are unchanged.
- The `DownloadProgress` and `CreationProgress` event shapes are unchanged.

## Error contract

| Condition | Typed error |
|---|---|
| Blank distribution or image ID | `InvalidRequest`-family validation via the existing ID check |
| Unknown distribution ID | `NotFound { kind: "distribution", id }` |
| Image ID not published by that distribution | `NotFound { kind: "distribution image", id }` |
| Transport, decode, schema, filesystem, or SQLite failure | Existing typed variants, unchanged |

`Ok(false)` is not an error: it is the normal "needs downloading" answer.
Callers treat `Err` as abort-the-flow, never as a marker value.

All error display text is English and safe to expose to callers.
