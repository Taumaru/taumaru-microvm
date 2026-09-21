# SDK Contract: Stop Running MicroVM

This contract describes the public SDK surface for the feature. It intentionally contains no CLI
command, Firecracker command line, shell command, SQLite statement, private-key material, or process
handle implementation detail.

## Public types

The type below is re-exported from `crates/sdk/src/lib.rs` and carries Rustdoc describing the same
invariants. Existing `MicroVmState` is reused unchanged.

```rust
pub struct MicroVmStopResult {
    pub name: String,
    pub state: MicroVmState,
    pub socket_path: std::path::PathBuf,
    pub forced: bool,
}
```

`state` is always `MicroVmState::Stopped` on success. `socket_path` is always inside the VM volume
and is silent at return. `forced` is `true` only when SIGKILL was delivered to the recorded
process; it is `false` for graceful exit, already-stopped, and natural-exit-during-wait. Callers
distinguish clean from forced shutdowns from this flag alone.

## Public operations

### Stop a MicroVM

```rust
impl MicroVmSdk {
    /// Stops one running MicroVM and reports whether forcing was used.
    pub async fn stop_microvm(
        &self,
        name: &str,
    ) -> Result<MicroVmStopResult, SdkError>;
}
```

Required behavior:

1. Validate the VM name with the existing name rules before touching host state. The SDK home
   comes from the constructed SDK instance; the volume directory and every runtime reference come
   only from the inventory record. No other input is accepted.
2. Serialize with the per-name plus volume lock pair, load the record by name, return `NotFound`
   for unknown names, and reject incomplete creations with a typed lifecycle conflict before any
   host work.
3. Decide running-versus-stopped from control-socket responsiveness only: an answering socket
   means running; anything else means stopped. Stored state, process liveness, and socket file
   existence never decide alone.
4. When already stopped, settle runtime refs if needed and return `Stopped` with `forced: false`,
   sending no shutdown request, signaling no process, and making no other host change.
5. When running, send one graceful shutdown request through the volume-local control socket, then
   wait 60 seconds for the socket to go silent and the recorded process to stop referencing the
   VM. When the machine exits within the bound, commit the stopped runtime row and return
   `Stopped` with `forced: false`.
6. When the socket already went silent before the request could be delivered, re-verify and return
   `Stopped` with `forced: false`. When delivery fails while the socket still answers, return a
   typed error with no forced-termination attempt.
7. When the 60-second wait expires with the machine still running, re-verify that the recorded
   process still references this VM, send SIGKILL to that identity only, re-verify for up to 10
   seconds that the machine is stopped, then commit the stopped runtime row and return `Stopped`
   with `forced: true`. If no usable process identity exists, or the machine is still running
   afterwards, return a typed error and never report success for a live machine.

Repeated calls are idempotent: an already-stopped VM returns success with `forced: false` and at
most a ref-settling write. Concurrent same-name calls run at most one shutdown sequence and
observe one consistent stopped result.

## Error contract

The public result is `Result<_, SdkError>`. No new public error variant is introduced; the
implementation reuses the existing surface:

| Category | Variant and distinguishing data |
|---|---|
| Invalid VM name | `InvalidRequest` with field and reason; no host mutation. |
| Unknown VM name | `NotFound` for kind `MicroVM`; no host mutation. |
| Incomplete creation record | `LifecycleConflict` with VM name, current state, and operation; no host mutation. |
| Graceful request undeliverable while socket answers | `TemporaryRuntime` with component, reason, and stopped flag; no forced attempt. |
| Still running with no usable process identity | `TemporaryRuntime`; never reports success for a live machine. |
| Forced termination fails or machine stays running | `TemporaryRuntime` or `HostCommand` naming the cause; never claims stopped for a live machine. |

All error display text is English and safe to expose to callers. No error variant includes a
private-key value, serialized secret, or a promise that the VM is stopped.

## Side-effect contract

- The operation is silent: no stdout/stderr, logging subscriber, tracing event, process exit, or
  global mutable state.
- Preflight failures (name, lookup, incomplete-record gate) perform no host mutation. The
  already-stopped path performs at most a runtime-ref settling write.
- The only process ever signaled is the recorded PID of this VM, only after re-verification that
  it still references this VM, and only with SIGKILL after the 60-second wait expires. A recycled
  PID belonging to an unrelated process is never signaled.
- The stale socket file is removed only at exactly the persisted socket path and only after live
  checks prove no listener. No other file is ever created, moved, or deleted.
- Other VMs, network attachments, artifacts, credentials, and the volume directory layout are
  untouched.

## Example usage

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;

// Graceful: the guest shuts down on its own.
let stopped = sdk.stop_microvm("build_vm").await?;
assert_eq!(stopped.state, MicroVmState::Stopped);
assert!(!stopped.forced);

// Stopping again is success with no host change.
let again = sdk.stop_microvm("build_vm").await?;
assert!(!again.forced);
```

The example intentionally uses only public SDK operations. It does not start, configure, reboot,
delete, list, inspect, download, or assemble a host command.
