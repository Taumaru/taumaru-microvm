# Phase 0 Research: Start Configured MicroVM

## Research goals

Resolve the implementation questions for one SDK-only `start_microvm(name)` operation that turns a
`Configured` VM into a durably `Running` one: live running detection that cannot be fooled by stale
database rows, a full-configuration `firectl` launch that returns immediately, bounded readiness,
repair-only network reconciliation, single-process concurrency, and failure cleanup that never
orphans a process or claims a false `Running` state.

## Findings

### `firectl` argument surface

- `firectl` accepts independent flags for every persisted configuration value: `--firecracker-binary`,
  `--kernel`, `--kernel-opts`, `--root-drive` (path optionally suffixed with `:ro` or `:rw`),
  `--tap-device` as `DEVICE/MAC`, `-c/--ncpus`, `-m/--memory` in MiB, `-s/--socket-path`, and
  `-l/--firecracker-log` for the VMM log file. The SDK therefore passes the VM-local `rootfs.ext4`
  with an explicit `:rw` suffix, the checked effective MiB value, and the volume-local socket path.
- `--kernel-opts` is a single kernel command line. The SDK composes it from the distribution's
  persisted `boot.kernel_args` plus the VM's persisted `desired_boot_parameters` (static `ip=` for
  host-only, DHCP marker for LAN), joined with spaces. The distribution `boot.root_device` selects
  the `root=` device.
- The runtime adapter builds the argv as typed argument vectors through the existing host-command
  conventions; no shell string is ever constructed.

Sources: [firectl README](https://github.com/firecracker-microvm/firectl/blob/main/README.md).

### Firecracker control socket protocol

- Each running Firecracker process serves an HTTP API over its Unix-domain socket, one socket per
  microVM. A minimal read such as `GET /machine-config` over UDS returning HTTP 200 proves the
  socket is served by a live VMM; a refused, timed-out, or error response proves it is not.
- File existence of `firecracker.sock` proves nothing by itself: a leftover file from a killed
  process connects to nobody. The probe must connect and read, never merely stat.
- The existing `tokio` workspace features already include `net`, so `tokio::net::UnixStream` is
  available with no new dependency.

Sources: [Firecracker swagger](https://github.com/firecracker-microvm/firecracker/blob/main/src/firecracker/swagger/firecracker.yaml),
[PandaStack REST walkthrough](https://www.pandastack.ai/blog/firecracker-rest-api-walkthrough).

### Process liveness without PID-reuse hazards

- A recorded PID is evidence only when `/proc/<pid>` exists AND its command line still references
  this VM (the volume-local socket path or the persisted runtime binary path). A bare existence
  check is unsafe because the OS recycles PIDs to unrelated processes.
- The check is read-only via standard-library filesystem APIs; the operation never signals a
  previously recorded PID. Only the child spawned by the current call may ever be terminated, and
  only on that call's own launch failure.
- Running requires both signals to agree (live verified process AND answering socket). Every
  mismatch — dead process, silent socket, PID reuse, orphaned socket file — resolves to
  stopped-plus-stale-refs followed by a fresh launch, never to a duplicate launch beside a live
  machine.

### Detached launch and caller return

- `tokio::process::Command` (the `process` feature is already enabled) spawns `firectl` with stdin
  nulled and stdout/stderr appended to a VM-local log file inside the volume directory, so all
  launch diagnostics stay with the VM and the SDK stays silent.
- The handle uses `kill_on_drop(false)` and is forgotten after the PID is recorded, so the caller
  regains control immediately with no session held open. On Unix the child is placed in a new
  process group so terminal hangup does not take the VM down with the caller.
- A passive detached reaper (`tokio::spawn` awaiting the forgotten handle's exit status, mutating
  nothing) avoids zombie accumulation in long-lived caller processes. It is a reaper, not a worker:
  no polling, no state machine, no retry logic.
- Alternative considered: double-fork daemonization with `setsid`. Rejected: it needs `libc` (not
  a current dependency) and complicates failure attribution, while process-group detachment plus a
  reaper achieves the required semantics with existing dependencies.

### Readiness before claiming `Running`

- After spawn, readiness polls the volume-local socket (connect plus `GET /machine-config`) on a
  sub-second interval with a bounded total well under the spec's 2-minute budget. Only a responsive
  socket commits the durable `Running` state with the PID and socket path in one repository
  closure.
- On expiry or spawn failure the operation kills only the just-spawned child, removes the socket
  file only if this attempt created it, resets runtime refs to stopped without claiming `Running`,
  and returns the typed launch error. Repaired network items stay persisted as correct state for
  the next retry, per the clarified spec.

### Network repair reuse

- No network adapter changes are needed. Start calls the existing `NetworkController::configure`
  exactly like `configure_network` does: the persisted network is passed as `existing`, the
  host-only and LAN used-address lists are loaded from the repository, correct resources return as
  `skipped`, and only missing or SDK-owned stale resources return as `applied`.
- The request mode is derived from the record's `expose_on_lan` and validated against the persisted
  mode first, so a mode switch is impossible and the persisted network identity never changes.
- Alternative considered: a start-specific repair path. Rejected: it would duplicate the
  ownership-aware reconciliation engine and risk divergent behavior between start and the standalone
  repair operation.

### Concurrency

- Start reuses the existing `target_lock` pair (per-name lock at `{home}/vms/{name}` plus the
  volume lock when the persisted volume differs), held across the live check, repair, launch, and
  commit. Concurrent starts for one VM therefore produce at most one child process, and every
  caller observes the same running identity.
- Alternative considered: a new in-process start mutex. Rejected: the path-keyed lock map already
  provides exactly this serialization, including across concurrently constructed SDK instances in
  the same process.

### Persistence without migration

- The `microvms.state` column already round-trips `"running"` through `MicroVmState::parse`, and
  `vm_runtime` already stores `process_id` and `process_state`. No schema migration is required.
- The `Running` commit is one repository closure running `persist_runtime` (PID, `"running"`,
  volume-local socket) plus `update_state(Running)` after readiness succeeds. Stale recovery and
  launch-failure cleanup use one closure resetting runtime refs plus `update_state(Configured)`.
- Transactions are never held open across host commands, process spawn, or socket polling; each
  closure covers one durable transition only.

### Error surface reuse

- Every start failure maps to an existing `SdkError` variant: `InvalidRequest` (name),
  `NotFound` (unknown name), `LifecycleConflict` (`Creating` rows), `StorageConflict` /
  `Filesystem` / `GuestFilesystem` / `Credential` (volume, rootfs, key problems),
  `ArtifactPrerequisite` (kernel or runtime binary no longer verified), `Network` (unrepairable
  attachment), `RuntimeIncompatible` / `HostCommand` (host cannot run VMs), `TemporaryRuntime`
  (launch, readiness, or stale-socket handling). No new public error variant is expected.
- The creation-only assertion that no socket file exists and the runtime is stopped must not be
  reused verbatim: start handles stale leftovers through liveness instead of failing on them.

No unresolved technical questions remain for Phase 1 design.
