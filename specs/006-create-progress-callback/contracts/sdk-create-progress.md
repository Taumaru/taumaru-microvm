# SDK Contract: MicroVM Creation Progress Observer

This contract describes the additive progress surface on top of the creation contract in
`specs/003-create-microvm/contracts/sdk-create.md`. It contains no CLI command,
Firecracker command line, shell command, SQLite statement, private-key material, or
registry HTTP implementation detail. The creation behavior contract (validation,
prerequisites, copy, keys, networking, persistence, idempotency, rollback) is unchanged
and is not restated here except where emission points attach to it.

## New public types

The types below are re-exported from `crates/sdk/src/lib.rs` with Rustdoc describing the
same invariants. They live in `crates/sdk/src/domain/microvm.rs` beside the existing
creation types and follow the `MicroVmState`/`NetworkMode` conventions (`Copy`,
`Display`, `pub(crate) as_str`).

```rust
pub const TOTAL_CREATION_STEPS: u64 = 6;

pub enum CreationStage {
    Validation,
    PrerequisiteResolution,
    VolumePreparation,
    CredentialSetup,
    NetworkConfiguration,
    Finalization,
}

pub enum CreationOutcome {
    Completed,
    AlreadyConfigured,
    Failed { stage: CreationStage },
}

pub struct CreationProgress {
    pub stage: CreationStage,
    pub completed_steps: u64,
    pub total_steps: u64,
    pub bytes_completed: Option<u64>,
    pub expected_bytes: Option<u64>,
    pub outcome: Option<CreationOutcome>,
}
```

- `CreationProgress` with `outcome: None` is a stage event; with `outcome: Some` it is
  a terminal event. Byte fields are `Some` together or `None` together, and are `Some`
  only on the `VolumePreparation` stage event (both equal the requested
  `disk_size_bytes`).
- `Display` for `CreationStage` yields `validation`, `prerequisite_resolution`,
  `volume_preparation`, `credential_setup`, `network_configuration`, `finalization`.
- `DownloadProgress` / `DownloadPhase` are untouched; creation events never reuse them.

## Changed public operation (before / after)

Before (removed — SDK unpublished at `0.1.0`, no external consumers, no deprecation
shim; all in-repo call sites migrate in the same change):

```rust
pub async fn create_microvm(
    &self,
    request: CreateMicroVmRequest,
) -> Result<MicroVmCreationResult, SdkError>;
```

After:

```rust
impl MicroVmSdk {
    /// Creates and initially configures one stopped MicroVM from verified local artifacts.
    ///
    /// When `on_progress` is `Some`, the SDK invokes the observer synchronously in the
    /// caller's task with one event per finished stage plus exactly one terminal event.
    /// The observer is caller-owned, infallible, and non-blocking, and cannot change
    /// integrity, persistence, or error decisions. When `None`, no events are emitted.
    pub async fn create_microvm<F>(
        &self,
        request: CreateMicroVmRequest,
        on_progress: Option<F>,
    ) -> Result<MicroVmCreationResult, SdkError>
    where
        F: FnMut(CreationProgress) + Send;
}
```

Call convention (same as the download callbacks):

```rust
let mut events = Vec::new();
let created = sdk
    .create_microvm(request, Some(|event: CreationProgress| events.push(event)))
    .await?;

// No observer:
let created = sdk.create_microvm(request, None::<fn(CreationProgress)>).await?;
```

## Required behavior

1. With `Some`, emit exactly one event per finished stage in fixed order
   (`Validation` 1/6 … `Finalization` 6/6), then exactly one terminal event.
   Success produces 7 events total; the `completed` terminal carries 6/6.
2. The `VolumePreparation` stage event reports `bytes_completed = expected_bytes =
   disk_size_bytes`; every other event reports both byte fields as `None`.
3. Failures emit the finished stages' events, then exactly one `failed` terminal
   carrying N/6 plus the failed stage — no extra stage event for the failure itself —
   then the existing typed error, unchanged.
4. Validation-stage rejections (invalid request, name conflict, non-configured
   existing state, stale persisted state) emit a single `failed` terminal at
   `Validation`, 0/6. Idempotent repeats emit a single `already-configured` terminal
   at `Validation`, 0/6, with no stage events and no host changes.
5. With `None`, behavior is identical to the previous contract: same results, typed
   errors, idempotency, conflicts, rollback, and zero events.
6. Silent: no stdout/stderr, logging subscriber, tracing event, process exit, or global
   state for progress. Per-call routing only; no cross-operation aggregation.

## Preserved behavior

- `configure_network`, all download operations, `resolve_binary`, listing operations,
  `DownloadProgress`, error variants, SQLite schema, on-disk layout, and rollback
  semantics are unchanged.
- Returned `MicroVmCreationResult` with an observer is field-identical to the same
  request without one.

## Error contract

Unchanged. No new `SdkError` variant. Terminal `failed` events accompany — never
replace — the existing typed errors. Observer panics are caller faults (same as
download callbacks), not SDK errors.
