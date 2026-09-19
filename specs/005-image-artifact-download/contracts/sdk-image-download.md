# SDK Contract: Single-Image Distribution Download

This contract describes the additive SDK surface for this feature. It contains no CLI
command, shell command, SQLite statement, or registry HTTP implementation detail.
Existing whole-distribution operations are unchanged.

## New public types

The type below is re-exported from `crates/sdk/src/lib.rs` with Rustdoc describing the
same invariants.

```rust
pub struct DownloadedDistributionImage {
    pub distribution: Distribution,
    pub image: DistributionImage,
    pub file: DownloadedFile,
}
```

- `distribution` is the registry metadata for the owning distribution as published.
- `image` is the exact registry image metadata selected for the operation.
- `file` is the single verified local image file, including its cache disposition
  (`Downloaded`, `AdoptedExisting`, or `SkippedExisting`).

## New public operations

```rust
impl MicroVmSdk {
    /// Downloads one distribution image into the SDK home.
    pub async fn download_distribution_image(
        &self,
        distribution_id: &str,
        image_id: &str,
    ) -> Result<DownloadedDistributionImage, SdkError>;

    /// Downloads one distribution image while observing a caller-owned
    /// cancellation handle.
    pub async fn download_distribution_image_with_cancellation(
        &self,
        distribution_id: &str,
        image_id: &str,
        cancellation: &DownloadCancellation,
        on_progress: impl FnMut(DownloadProgress) + Send,
    ) -> Result<DownloadedDistributionImage, SdkError>;
}
```

Required behavior:

1. Validate both IDs as non-blank registry identifiers before any registry access.
2. Resolve the distribution from the current manifest; an unknown distribution returns
   `NotFound { kind: "distribution", .. }`.
3. Resolve the image within that distribution's published image list; an image absent
   from that distribution returns `NotFound { kind: "distribution image", .. }`.
4. Acquire exactly that image through the existing member pipeline (cache decision,
   streaming, SHA-256 and size verification, per-image persistence with boot metadata
   and kernel references). No other image of the distribution is touched.
5. Honor the cancellation handle cooperatively: remove the in-flight temporary file
   before returning `Cancelled`, and preserve images already committed by earlier
   calls. Verified images remain available when a later image is cancelled.
6. Stay silent: no stdout/stderr, logging subscriber, tracing event, process exit, or
   global mutable state. All expected failures are typed `SdkError` values.

## Preserved behavior

- `download_distribution` and `download_distribution_with_cancellation` keep their
  existing whole-distribution semantics and return `DownloadedDistribution`.
- The `DownloadProgress` event shape is unchanged; single-image events carry
  `(DistributionImage, distribution_id, Some(image_id))` with single-member
  aggregate counters.
- Persistence keys, on-disk layout (`artifacts/rootfs/{distro}/{image}/{filename}`),
  and `resolve_distribution_image(distro, image)` semantics are unchanged, so a
  single-image download and a later whole-distribution download of the same
  distribution converge on identical rows.
- `list_distributions`, `list_kernels`, `list_binaries`, `download_kernel`,
  `download_binary`, and `create_microvm` are unaffected.

## Error contract additions

| Condition | Typed error |
|---|---|
| Blank distribution or image ID | `InvalidRequest`-family validation via the existing ID check |
| Unknown distribution ID | `NotFound { kind: "distribution", id }` |
| Image ID not published by that distribution | `NotFound { kind: "distribution image", id }` |
| Transport, decode, schema, integrity, filesystem, or SQLite failure | Existing typed variants, unchanged |
| Cancellation before publish | `Cancelled`, partial file removed, nothing published |

All error display text is English and safe to expose to callers.
