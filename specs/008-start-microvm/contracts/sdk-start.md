# SDK Contract: Start Configured MicroVM

This contract describes the public SDK surface for the feature. It intentionally contains no CLI
command, `firectl` command line, shell command, SQLite statement, private-key material, or process
handle implementation detail.

## Public types

The type below is re-exported from `crates/sdk/src/lib.rs` and carries Rustdoc describing the same
invariants. Existing `MicroVmState`, `NetworkConfiguration`, and `SshConnectionInfo` types are
reused unchanged.

```rust
pub struct MicroVmStartResult {
    pub name: String,
    pub state: MicroVmState,
    pub volume_path: std::path::PathBuf,
    pub rootfs_path: std::path::PathBuf,
    pub socket_path: std::path::PathBuf,
    pub process_id: u32,
    pub network: NetworkConfiguration,
    pub ssh: SshConnectionInfo,
}
```

`state` is always `MicroVmState::Running` on success. `socket_path` is always inside
`volume_path` and is the preferred control channel for later operations. `process_id` is the
background machine process and exists as the fallback for forced termination of an unresponsive
machine. `ssh` carries the private-key path, never key contents; the guest identity stays
`root` on port `22`.

## Public operations

### Start a MicroVM

```rust
impl MicroVmSdk {
    /// Starts one configured MicroVM and returns its running identity.
    pub async fn start_microvm(
        &self,
        name: &str,
    ) -> Result<MicroVmStartResult, SdkError>;
}
```

Required behavior:

1. Validate the VM name with the existing name rules before touching host state. The SDK home
   comes from the constructed SDK instance; the volume directory comes only from the inventory
   record. No volume-path input is accepted.
2. Serialize with the per-name plus volume lock pair, load the record by name, return `NotFound`
   for unknown names, and reject `Creating` rows with a typed lifecycle conflict. Only
   `Configured` and `Running` rows proceed.
3. Validate the persisted machine data: volume is a real absolute directory, rootfs and socket
   paths stay inside it, the rootfs file matches the recorded size, credential paths/modes/metadata
   match the contract, and the kernel plus Firecracker/`firectl` paths still verify against the
   artifact inventory. No host mutation has happened yet.
4. Decide running-versus-stopped from live host evidence only: the recorded process is live only
   when its command line still references this VM, and the socket counts only when a connection
   plus a minimal control read answers. Both must agree for running; any mismatch is stale.
5. When genuinely running, return the current running identity without launching a process and
   without changing persisted configuration. When stale, clear or replace the stale runtime
   references and continue with a fresh start.
6. Validate host virtualization support, reconcile the persisted network through the existing
   network port with the stored attachment as the baseline, keep correct items untouched, repair
   only missing or SDK-owned stale items, keep the persisted network identity unchanged, and
   persist the reconciled attachment.
7. Launch the full persisted configuration (vCPUs, effective memory, VM-local root disk, kernel,
   network attachment, boot parameters) as a detached background process with the socket inside
   the volume, then wait with a bounded readiness poll until the control channel answers.
8. On readiness, persist the runtime record (process ID, running marker, volume-local socket) and
   the `Running` state atomically, then return the running identity. The caller regains control
   immediately; no session must be held open.
9. On launch failure, terminate only the just-spawned child, remove the owned socket only if this
   attempt created it, reset runtime refs to stopped without claiming `Running`, keep repaired
   network items in place, and return the typed launch error with no orphan process left behind.

Repeated calls are idempotent: a genuinely running VM returns the same identity with exactly one
machine process. Concurrent same-name calls produce at most one process and one observed identity.

## Error contract

The public result is `Result<_, SdkError>`. No new public error variant is introduced; the
implementation reuses the existing surface:

| Category | Variant and distinguishing data |
|---|---|
| Invalid VM name | `InvalidRequest` with field and reason; no host mutation. |
| Unknown VM name | `NotFound` for kind `MicroVM`; no host mutation. |
| Stored state is `Creating` | `LifecycleConflict` with VM name, current state, and operation. |
| Volume, rootfs, key, path, or guest-data problem | `StorageConflict`, `Filesystem`, `GuestFilesystem`, or `Credential` naming the path. |
| Boot artifact no longer verified | `ArtifactPrerequisite` with kind, registry ID, local path, and reason. |
| Network cannot be inspected or repaired | `Network` with mode, operation, resource, and reason; VM stays stopped. |
| Host cannot run VMs | `RuntimeIncompatible` or `HostCommand`; no launch attempted. |
| Launch, readiness, or stale-socket handling fails | `TemporaryRuntime` with component, reason, and stopped flag; no orphan process. |
| Launch fails after repair | Primary launch error; repaired network stays persisted; VM stays stopped. |

All error display text is English and safe to expose to callers. No error variant includes a
private-key value, serialized secret, or a promise that the VM is running.

## Side-effect contract

- The operation is silent: no stdout/stderr, logging subscriber, tracing event, process exit, or
  global mutable state. Launch diagnostics append to a VM-local log file inside the volume.
- Preflight failures (name, lookup, `Creating` gate, persisted-data validation) perform no host
  mutation.
- The stale socket file is removed only at exactly the persisted socket path and only after live
  checks prove no listener. A previously recorded process ID is never signalled.
- The operation never moves, overwrites, merges, or reinterprets the persisted volume directory
  and never places an active socket outside it.
- A previously recorded process ID is never signalled.

## Example usage

```rust
let sdk = MicroVmSdk::new("/var/lib/taumaru")?;

let running = sdk.start_microvm("build_vm").await?;

assert_eq!(running.state, MicroVmState::Running);
assert_eq!(running.socket_path, running.volume_path.join("firecracker.sock"));

// A repeated call while the VM is live returns the same identity without a new process.
let again = sdk.start_microvm("build_vm").await?;
assert_eq!(again.process_id, running.process_id);
```

The example intentionally uses only public SDK operations. It does not configure, stop, reboot,
delete, list, inspect, download, or assemble a launch command.
