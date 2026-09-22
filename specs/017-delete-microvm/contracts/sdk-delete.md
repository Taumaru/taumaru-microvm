# SDK Contract: Delete MicroVM

This contract describes the public SDK surface for the feature. It intentionally contains no CLI
command, shell command, SQLite statement, private-key material, or host-command implementation detail.

## Public types

The type below is re-exported from `crates/sdk/src/lib.rs` and carries Rustdoc describing the same
invariants.

```rust
pub struct MicroVmDeleteResult {
    pub name: String,
}
```

`name` is the stable identifier of the deleted machine. On success the inventory record is gone,
the whole volume directory is gone, and the VM's owned host network items are released, while
shared kernels, distribution images, tool binaries, and other VMs are fully intact. Failures
surface as typed errors with no result.

## Public operations

### Delete a MicroVM

```rust
impl MicroVmSdk {
    /// Deletes one stopped MicroVM and everything it owns.
    pub async fn delete_microvm(
        &self,
        name: &str,
    ) -> Result<MicroVmDeleteResult, SdkError>;
}
```

Required behavior:

1. Validate the VM name with the existing name rules before touching host state. The SDK home
   comes from the constructed SDK instance; the volume directory and every owned reference come
   only from the inventory record. No other input is accepted.
2. Serialize with the per-name plus volume lock pair, load the record by name, and return
   `NotFound` for unknown names — including names deleted by an earlier call.
3. Decide running-versus-stopped from control-socket responsiveness only: an answering socket
   means running, clean silence means stopped. Stored state, process liveness, and socket file
   existence never decide alone. An unprobable socket propagates its typed probe error without
   deleting.
4. When running, refuse with a typed lifecycle conflict directing the caller to stop the machine
   first, with zero host changes: no files removed, no network released, no record altered.
5. Delete in the order host network release → whole volume-directory removal → inventory-record
   deletion. Incomplete creation records are deletable; absent child rows skip their step, and
   already-absent owned files or network items count as already removed.
6. Keep the record on every failure and never report success while the record or owned resources
   remain, so retrying the same delete resumes from the remaining owned resources and converges.
7. Return `MicroVmDeleteResult { name }` only after the record deletion commits. Shared
   artifacts are never removal targets, and other VMs are never touched.

A failed delete is always retryable: the caller fixes the cause (stops the machine, repairs
permissions, restores host access) and calls delete again with the same name.

## Error contract

The public result is `Result<_, SdkError>`. No new public error variant is introduced; the
implementation reuses the existing surface:

| Category | Variant and distinguishing data |
|---|---|
| Invalid VM name | `InvalidRequest` with field and reason; no host mutation. |
| Unknown VM name (including already deleted) | `NotFound` for kind `MicroVM`; no host mutation. |
| Running machine | `LifecycleConflict` with name, state `running`, and operation directing stop-before-delete; no host mutation. |
| Unprobable control socket | The probe's typed error propagated unchanged; no host mutation. |
| Persisted paths escape the volume or home | `StorageConflict` naming the volume; record kept for repair. |
| Owned volume removal fails | `Filesystem` naming the operation and path; record kept for retry. |
| Present owned network item unreleasable | The adapter's typed error (`Network`, `HostCommand`, or `Cleanup`); record kept for retry. |
| Record deletion fails | The repository's typed error; record still present for retry. |

All error display text is English and safe to expose to callers. No error variant includes a
private-key value, serialized secret, or a promise that the VM is deleted.

## Side-effect contract

- The operation is silent: no stdout/stderr, logging subscriber, tracing event, process exit, or
  global mutable state.
- Preflight failures (name, lookup, running refusal, unprobable socket) perform no host mutation.
- Host network release touches only the items recorded as owned by this VM; already-absent items
  skip, and unowned host configuration is never addressed.
- File removal deletes exactly one directory tree: the VM's persisted volume directory strictly
  below the SDK home. Nothing outside it is ever created, moved, or deleted.
- No process is ever signaled. No kernel, image, tool binary, cache, or tmp entry is ever
  removed. No other VM's files, attachments, or records are ever touched.

## Example usage

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;

// Deleting a running machine is refused: stop first.
match sdk.delete_microvm("build_vm").await {
    Err(SdkError::LifecycleConflict { .. }) => {
        sdk.stop_microvm("build_vm").await?;
        let deleted = sdk.delete_microvm("build_vm").await?;
        assert_eq!(deleted.name, "build_vm");
    }
    Ok(deleted) => assert_eq!(deleted.name, "build_vm"),
    Err(other) => return Err(other),
}

// The name no longer resolves afterwards.
let missing = sdk.delete_microvm("build_vm").await;
assert!(matches!(missing, Err(SdkError::NotFound { .. })));
```

The example intentionally uses only public SDK operations. It does not create, configure,
reboot, list, inspect, download, prune, or assemble a host command.
