# Phase 0 Research: Stop Running MicroVM

## Research goals

Resolve the implementation questions for one SDK-only `stop_microvm(name)` operation: which
control-socket message asks a Firecracker machine to shut down gracefully, how to observe the exit
within the clarified 60-second bound, how to force-terminate with immediate SIGKILL through existing
port boundaries, and how to persist the stopped state without a schema migration.

## Findings

### Graceful shutdown message: `SendCtrlAltDel` over `PUT /actions`

- The Firecracker HTTP API over the per-VM Unix-domain socket exposes exactly three synchronous
  actions on `PUT /actions`: `InstanceStart`, `FlushMetrics`, and `SendCtrlAltDel` (swagger enum
  `InstanceActionInfo.action_type`). There is no halt/power-off action.
- `SendCtrlAltDel` emulates the CTRL+ALT+DEL key sequence through an i8042 controller; by
  convention most Linux distributions perform an orderly shutdown and reset on it, and Firecracker
  exits on CPU reset. Success returns HTTP 204. This is the documented mechanism for triggering a
  clean shutdown of the microVM.
- Request shape mirrors the existing socket probe style (UDS connect, HTTP/1.0 request, bounded
  read timeouts, std `UnixStream`, no new dependency):
  `PUT /actions HTTP/1.0` with body `{ "action_type": "SendCtrlAltDel" }`.
- Guest-driver caveat: the guest kernel needs `CONFIG_SERIO_I8042` and `CONFIG_KEYBOARD_ATKBD` for
  the emulated key sequence to land. If the guest image lacks them, the graceful request is
  delivered (204) but the guest never shuts down, so the operation always escalates through the
  60-second wait to SIGKILL. Guest images used with this SDK are expected to carry the drivers;
  the forced path is the backstop when they do not. A non-204 response or any write/read failure
  is a delivery failure: typed error, no forced fallback, per the clarified spec.
- Alternative considered: SSH `reboot`/`poweroff` inside the guest. Rejected: it needs guest
  credentials, network reachability, and guest-agent cooperation, while the control socket is
  already the preferred control channel and works without guest cooperation.

Sources: [Actions API](https://github.com/firecracker-microvm/firecracker/blob/main/docs/api_requests/actions.md),
[firecracker.yaml](https://github.com/firecracker-microvm/firecracker/blob/main/src/firecracker/swagger/firecracker.yaml).

### Exit wait: bounded poll reusing the existing probe pair

- The 60-second bound (clarified to match the start operation's readiness deadline) is a poll loop
  over the same two signals start uses: the recorded process still references this VM
  (`process_references_vm`: `/proc/<pid>` exists AND its command line references the VM socket or
  runtime binary) and the control socket answers (`socket_answers`: UDS connect plus a minimal
  control read, never file existence). The machine counts as exited when the socket goes silent
  AND the recorded process no longer references the VM.
- Poll interval mirrors the start readiness loop (200 ms). The wait lives in the `FirecrackerRuntime`
  adapter as `wait_for_stop` next to `wait_for_socket`, keeping the manager free of socket/process
  mechanics per the ports/adapters boundary.
- Alternative considered: manager-side polling loop. Rejected: host-process mechanics belong in the
  runtime adapter; the manager already delegates every other probe there.

### Forced termination: reuse `terminate_spawned` (immediate SIGKILL)

- The clarified spec requires immediate SIGKILL with no intermediate SIGTERM. The existing
  `terminate_spawned` adapter already sends `kill -KILL`, treats an already-gone process as
  success (re-checked via `/proc`), and returns a typed `HostCommand` error when the process
  survives. It is reused unchanged for the recorded PID.
- PID-reuse protection is preserved: SIGKILL fires only after `process_references_vm` confirms the
  recorded PID still references this VM at escalation time. A live unrelated process under a
  recycled identifier is never signaled; that case returns a typed error instead.
- After SIGKILL the operation re-waits with a short bound (10 seconds, planning decision) for the
  socket to go silent and the process reference to clear. Still running afterwards is a typed
  error; success is never claimed for a live machine.

### Undeliverable graceful request: typed error, no forced fallback

- Per the clarified spec, any failure to deliver the shutdown request (connect error beyond
  already-silent, write failure, read timeout, non-204 response) returns a typed error without
  attempting forced termination. Rationale recorded in clarification: force-killing over an unknown
  control-channel state risks disk corruption for what may be a transient socket error.
- Race rule: if the socket went silent between the entry liveness check and the shutdown attempt
  (connect reports already-silent), the operation re-verifies; a silent socket means success with
  `forced: false`, not an error. Only a still-answering socket after a failed delivery is an error.

### Concurrency and persistence reuse

- Stop reuses the existing `target_lock` pair (per-name lock at `{home}/vms/{name}` plus the volume
  lock when the persisted volume differs), held across the liveness check, shutdown, wait,
  escalation, and commit — the same pattern as start. Concurrent stops for one VM therefore run
  exactly one shutdown sequence; the second caller observes the stopped result.
- No schema migration is required. Success reuses the existing `reset_runtime_to_stopped` helper
  (`persist_runtime` with `process_id: None`, `process_state: "stopped"`) and the existing
  `clear_stale_runtime` settle check. The stale socket file is removed only at exactly the
  persisted socket path and only after liveness proves no listener, reusing the
  `remove_stale_socket` helper.
- Incomplete creations (`require_complete` fails) are rejected with `LifecycleConflict` before any
  host work, mirroring start's guard against half-configured machines.

### Error surface reuse

- Every stop failure maps to an existing `SdkError` variant: `InvalidRequest` (name),
  `NotFound` (unknown name), `LifecycleConflict` (incomplete creation),
  `TemporaryRuntime` (undeliverable shutdown, unforceable still-running machine, failed forced
  termination, post-kill still-running), `HostCommand` (kill execution failure),
  `Filesystem` (socket/probe I/O). No new public error variant is expected.

No unresolved technical questions remain for Phase 1 design.
