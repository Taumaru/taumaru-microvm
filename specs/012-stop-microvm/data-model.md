# Phase 1 Data Model: Stop Running MicroVM

## Input

### Stop intent

The operation takes only a VM name. The SDK home comes from the constructed `MicroVmSdk`; the
volume directory and every runtime reference are read from the inventory record, never from the
caller.

| Field | Type | Rules |
|---|---|---|
| `name` | `&str` | 1–64 ASCII characters; first character alphanumeric; remaining alphanumeric, `-`, or `_`. Validated with the existing `validate_vm_name` helper before any host work. |

No volume-path input exists. No request struct is introduced; the name is passed directly, mirroring
`start_microvm`.

## Records read

### `MicroVmRecord` (existing, read-only for stop)

One durable row identifies one independently managed VM. Stop reads `name`, `volume_path`,
`socket_path`, and `rootfs_path` for probe inputs only; it never modifies these fields. Incomplete
creations (`StoredMicroVm::require_complete` fails) are rejected with `LifecycleConflict` before
any host work, mirroring start's guard.

### `PersistedRuntime` (existing, probe input plus written on transition)

The recorded runtime supplies the process reference for the exit wait and the SIGKILL escalation:

| Field | Role in stop |
|---|---|
| `firecracker_path` | PID-reference check input: the recorded process must reference this path or the socket path to count as this VM's process. |
| `firectl_path` | Preserved verbatim on the stopped write (never cleared, needed for the next start). |
| `socket_path` | Must equal the record socket path; the sole target of the shutdown request and liveness probes. |
| `process_id` | `Some(pid)` is the only identity ever signaled; `None` means nothing to force. |
| `process_state` | Informational only; never decides running versus stopped. |

## Records written

### `MicroVmStopResult` (new public type)

Returned on every success path, including already-stopped and the escalation race where the
process exits on its own:

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | Stable VM identifier |
| `state` | `MicroVmState` | Always `Stopped` on success |
| `socket_path` | `PathBuf` | Volume-local control socket; silent at return |
| `forced` | `bool` | `true` only when SIGKILL was delivered to the recorded process; `false` for graceful exit, already-stopped, and natural-exit-during-wait |

### Stopped runtime row (existing shape, reset values)

On success the runtime row is rewritten with the same helper start uses for stale cleanup:

| Field after successful stop | Value |
|---|---|
| `firecracker_path` | Preserved persisted path |
| `firectl_path` | Preserved persisted path |
| `socket_path` | `{volume_path}/firecracker.sock` |
| `process_id` | `None` |
| `process_state` | `"stopped"` |

A stale leftover socket file is removed only at exactly the persisted socket path and only after
liveness proves no listener. If no write is needed (already-settled refs on the already-stopped
path), no write happens, mirroring `clear_stale_runtime`.

## State machine

```text
Stopped (socket silent) ──► return MicroVmStopResult { Stopped, forced: false }
                                (no shutdown, no signal, no host change)

Running (socket answers) ──► PUT /actions SendCtrlAltDel ──► 60 s exit poll
        │                                                              │
        │ delivery fails while                                        ├── socket silent + process gone
        │ socket still answers                                        │   ──► commit stopped, forced: false
        │   ──► typed error, no forced attempt                        │
        │                                                              ├── 60 s expires, still running
        │ socket already silent at                                    │   + recorded PID references this VM
        │ delivery time                                               │   ──► SIGKILL ──► 10 s re-verify ──► stopped
        │   ──► re-verify: stopped ──► forced: false                  │       still running afterwards ──► error
        │                                                              │
        │                                                              └── still running, no usable PID
        │                                                                  ──► typed error, never report success
```

The machine counts as exited when the socket is silent AND the recorded process no longer references
the VM. File existence of the socket without an answering connection never counts as running; PID
existence without a command line referencing this VM never counts as live.

## Liveness decision table

| Socket answers | Recorded process references this VM | Decision |
|---|---|---|
| no | either | stopped: return success, settle refs |
| yes | n/a (no usable recorded PID) | running, unforceable: attempt graceful, error if still running after wait |
| yes (then delivery fails) | either | typed error, no forced attempt |
| yes (then 60 s wait expires) | yes | SIGKILL, re-verify, success with `forced: true` |
| yes (then 60 s wait expires) | no | typed error, never signal an unrelated process |

## Ownership and multi-VM rules

- The per-name lock (`{home}/vms/{name}`) plus the volume lock (when the persisted volume differs)
  are held across the liveness check, shutdown request, exit wait, escalation, and commit, so
  concurrent same-name stops run one shutdown sequence and all callers observe one stopped result.
- Only the recorded PID of this VM may be signaled, and only after it is re-verified to reference
  this VM at escalation time. A previously recorded PID from an already-settled row is never
  signaled.
- No network, artifact, credential, or other-VM state is touched. Stop never moves, overwrites, or
  reinterprets the persisted volume directory and never places or removes any file except the
  owned stale socket at exactly the persisted socket path.
