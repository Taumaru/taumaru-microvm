# Phase 1 Data Model: Start Configured MicroVM

## Input

### Start intent

The operation takes only a VM name. The SDK home comes from the constructed `MicroVmSdk`; the
volume directory is read from the inventory record, never from the caller.

| Field | Type | Rules |
|---|---|---|
| `name` | `&str` | 1–64 ASCII characters; first character alphanumeric; remaining alphanumeric, `-`, or `_`. Validated with the existing `validate_vm_name` helper before any host work. |

No volume-path input exists. No request struct is introduced; the name is passed directly.

## Records read

### `MicroVmRecord` (existing, read-only for start)

One durable row identifies one independently managed VM. Start reads `name`, `state`,
`distribution_id`, `image_id`, `kernel_id`, `firecracker_package_id`, `firectl_package_id`,
`disk_size_bytes`, `memory_bytes`, `memory_effective_mib`, `vcpu_count`, `volume_path`,
`rootfs_path`, `socket_path`, and `expose_on_lan`. It never modifies these fields.

Validation rules applied by start:

- `state` must be `Configured` (proceed to live checks and launch) or `Running` (proceed to live
  verification for idempotency or stale recovery). `Creating` is rejected with
  `LifecycleConflict` before any host work.
- `volume_path` must be absolute and must be a real directory (not a symlink). `rootfs_path` must
  equal `{volume_path}/rootfs.ext4` and `socket_path` must equal
  `{volume_path}/firecracker.sock`; anything else is a `StorageConflict`.
- The `rootfs.ext4` file must exist as a regular file with exactly `disk_size_bytes` bytes.
- Credential paths must equal `{volume_path}/ssh/id_ed25519` and `{volume_path}/ssh/id_ed25519.pub`,
  both regular files with `0600`/`0644` modes, `ed25519` / `root` / `22` /
  `/root/.ssh/authorized_keys` metadata. Anything else is a `Credential` error.
- The kernel path resolved from `kernel_id` and the Firecracker/`firectl` paths resolved from
  their package IDs must still verify against the artifact inventory (executable,
  host-compatible); otherwise `ArtifactPrerequisite`.

### `PersistedNetwork` (existing, reconciled in place)

Start passes the stored network as `existing` to `NetworkController::configure`. The request mode
is derived from `expose_on_lan` and must match the persisted mode; the persisted addresses, TAP
name, guest MAC, and boot parameters are never changed by start. The reconciled result is written
back with `update_network`.

### `PersistedRuntime` (existing, rewritten on transition)

| Field after successful start | Value |
|---|---|
| `firecracker_path` | Reverified persisted path |
| `firectl_path` | Reverified persisted path |
| `socket_path` | `{volume_path}/firecracker.sock` |
| `process_id` | `Some(pid)` of the background child |
| `process_state` | `"running"` |

After stale recovery or launch failure, `process_id` is `None` and `process_state` is `"stopped"`.

## Records written

### `MicroVmStartResult` (new public type)

Returned on success, including the idempotent already-running path:

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | Stable VM identifier |
| `state` | `MicroVmState` | Always `Running` on success |
| `volume_path` | `PathBuf` | Persisted volume directory used |
| `rootfs_path` | `PathBuf` | VM-local writable root disk used for launch |
| `socket_path` | `PathBuf` | Volume-local control socket; the preferred control channel |
| `process_id` | `u32` | Background machine process; fallback for forced termination |
| `network` | `NetworkConfiguration` | Persisted network metadata after reconciliation |
| `ssh` | `SshConnectionInfo` | `root:22` metadata with the private-key path only |

## State machine

```text
Configured ── live checks show stopped ──► launch ── readiness ok ──► Running
    │                                            │
    │  live checks show running                  └── readiness/launch fail ──► Configured
    │  (process live + socket answers)                (no orphan, repaired net kept)
    └──► return current identity (no launch)

Running ── live checks show running ──► return current identity (no launch)
    │
    └── live checks show any mismatch ──► treat as stopped ──► launch ──► Running

Creating ──► rejected (LifecycleConflict, no host work)
```

`Running` is the only durable state start writes on success. The standalone repair operation keeps
its `Configured`-only gate; start is the sole writer of durable `Running`.

## Liveness decision table

| Recorded process verified live | Socket answers | Decision |
|---|---|---|
| yes | yes | running: return identity, launch nothing |
| yes | no | stale: clear refs, launch fresh |
| no | yes | stale: clear refs, launch fresh |
| no | no | stopped: launch fresh |

File existence of the socket without an answering connection never counts as running. PID existence
without a command line referencing this VM never counts as live.

## Ownership and multi-VM rules

- The per-name lock (`{home}/vms/{name}`) plus the volume lock (when the persisted volume differs)
  are held across check, repair, launch, and commit, so concurrent same-name starts yield one
  process and one observed identity.
- The stale socket file is removed only at exactly the persisted socket path and only after live
  checks prove no listener. No other file is ever deleted by start.
- Only the child spawned by the current call may be terminated, and only on that call's launch
  failure. A previously recorded PID is never signalled.
- Caller-owned data, source artifacts, foreign network resources, and other VM records are
  preserved. Launch diagnostics append to a VM-local log file inside the volume directory.
