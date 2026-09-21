# Feature Specification: Stop Running MicroVM

**Feature Branch**: `012-stop-microvm`

**Created**: 2026-09-20

**Status**: Draft

**Input**: User description: "Build the SDK operation that stops a running machine. It receives the name of a running machine and first checks whether the machine is already stopped, in which case it returns success. Otherwise it stops the machine by sending a shutdown message to Firecracker through the machine's control socket, then waits for the machine process to exit; if the process does not exit, it force-terminates the process by its recorded identity. In the end the operation returns whether stopping required forcing or not."

## Clarifications

### Session 2026-09-20

- Q: When the graceful shutdown request cannot be delivered through the machine's control socket (socket error mid-call), what should the stop operation do? → A: Return a typed error with no forced termination attempt.
- Q: When the machine stays running after the graceful wait expires, how should forced termination be delivered to the recorded process? → A: Immediate SIGKILL to the recorded process identity, with no intermediate SIGTERM step.
- Q: How long should the stop operation wait for the machine to exit after the graceful shutdown request before escalating to forced termination? → A: 60 seconds, matching the start operation's socket readiness deadline.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Gracefully stop a running machine (Priority: P1)

An application developer stops a running MicroVM by its unique name. The operation asks the machine to shut down through its control channel, the machine process exits on its own, and the returned result reports a stopped machine with no forcing used.

**Why this priority**: This is the core value of the feature: turning a running machine into a stopped one with a single call while giving the guest a chance to shut down cleanly.

**Independent Test**: Start one MicroVM, call the stop operation with the VM name, and verify the machine reaches stopped state, its control socket goes silent, and the result indicates graceful (unforced) termination.

**Acceptance Scenarios**:

1. **Given** a genuinely running MicroVM (its control socket answers), **When** the caller stops it by name, **Then** the operation sends a graceful shutdown request through the control socket, the machine process exits, and the result reports stopped with forcing marked as not used.
2. **Given** a stop call that succeeds gracefully, **When** the caller later checks machine state or starts the machine, **Then** state surfaces report stopped and start proceeds as for a stopped machine.
3. **Given** a successful graceful stop, **When** the caller inspects the result, **Then** the result clearly indicates that no forced termination was used.

---

### User Story 2 - Stopping an already-stopped machine succeeds (Priority: P1)

An application developer stops a machine that is already stopped (never started, previously stopped, or its process died outside the SDK). The operation returns success reporting stopped without touching host processes or sending any shutdown request.

**Why this priority**: The caller explicitly requires idempotency: "already stopped returns success". Without it, repeated or redundant stops become errors and recovery flows break.

**Independent Test**: Call stop against a stopped machine, a never-started machine, and a machine whose process was killed outside the SDK; every call returns success reporting stopped with no host changes.

**Acceptance Scenarios**:

1. **Given** a MicroVM whose control socket does not answer (stopped), **When** the caller stops it by name, **Then** the operation returns success reporting stopped with forcing marked as not used, sending no shutdown request and signaling no process.
2. **Given** a MicroVM that was never started, **When** the caller stops it by name, **Then** the operation returns success reporting stopped.
3. **Given** a MicroVM whose process was terminated outside the SDK, **When** the caller stops it, **Then** the operation returns success reporting stopped instead of an error.

---

### User Story 3 - Unresponsive machine is forced to stop (Priority: P2)

An application developer stops a running machine that ignores the graceful shutdown request. After a bounded wait the operation force-terminates the machine process, verifies the machine is stopped, and the result reports that forcing was used.

**Why this priority**: A machine that never honors graceful shutdown must still be stoppable; without the forced fallback, one hung guest strands the machine forever. The forced flag lets callers distinguish a clean shutdown from an unclean one.

**Independent Test**: Stop a running machine whose guest ignores the shutdown request (test double that never exits on request); verify the machine reaches stopped and the result indicates forced termination.

**Acceptance Scenarios**:

1. **Given** a running MicroVM that does not exit within 60 seconds after the graceful request, **When** the caller stops it, **Then** the operation force-terminates the machine process with SIGKILL, verifies the machine is stopped, and returns stopped with forcing marked as used.
2. **Given** a forced stop, **When** the caller inspects the result, **Then** the caller can distinguish the forced outcome from a graceful one using the result alone.
3. **Given** a forced stop that succeeded, **When** later state checks run, **Then** every state surface reports the machine as stopped.

---

### Edge Cases

- What happens when the VM name is unknown? The operation returns a typed not-found error with no host changes.
- What happens when the VM name is empty or malformed? The operation rejects the request with a typed invalid-input error before touching host state.
- What happens when the socket answers but there is no usable recorded process reference (e.g. the machine was started outside the SDK)? The graceful request is still attempted through the socket; if the machine stays running and no process identity exists to force, the operation returns a typed error and MUST NOT report success for a still-running machine.
- What happens when the graceful shutdown request itself cannot be delivered (socket error mid-call)? The operation returns a typed error without attempting forced termination.
- What happens when the recorded process exits on its own between the wait expiring and the forced termination (race)? The operation re-verifies liveness; a machine verified as stopped returns success.
- What happens when forced termination fails and the machine is still running? The operation returns a typed error; it MUST NOT claim the machine is stopped.
- What happens when two stop calls run concurrently for the same VM? They coordinate so exactly one shutdown sequence runs and all callers observe a consistent stopped result.
- What happens to other machines when one machine stops? Nothing: stop targets exactly one machine by name and never touches other MicroVMs.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The stop operation MUST accept the explicit SDK home directory and the unique MicroVM name from its caller, and MUST NOT require any other input.
- **FR-002**: The operation MUST validate the VM name before changing host state, rejecting empty or malformed names with typed invalid-input errors.
- **FR-003**: The operation MUST verify the VM exists in the local inventory by name and return a typed not-found error without host changes when it does not.
- **FR-004**: The operation MUST decide running versus stopped at call time from control-socket responsiveness (an answering socket means running); it MUST NOT decide from stored state alone, process liveness alone, or socket file existence alone.
- **FR-005**: When the machine is already stopped, the operation MUST return success reporting stopped with forcing marked as not used, sending no shutdown request, signaling no process, and making no host changes.
- **FR-006**: When the machine is running, the operation MUST send a graceful shutdown request through the machine's control socket.
- **FR-007**: After the graceful request, the operation MUST wait 60 seconds for the machine process to exit and the control socket to go silent.
- **FR-008**: When the machine exits within the bound, the operation MUST return success reporting stopped with forcing marked as not used.
- **FR-009**: When the machine is still running after the 60-second wait expires, the operation MUST send SIGKILL to the recorded process identity (only when that identity still references this VM), re-verify that the machine is stopped, and return stopped with forcing marked as used.
- **FR-010**: The stop result MUST report the final machine state together with whether forced termination was used, so the caller distinguishes graceful from forced outcomes from the result alone.
- **FR-011**: When the machine is still running and no usable process identity exists to force termination, the operation MUST return a typed error and MUST NOT report success.
- **FR-012**: On success the operation MUST persist the stopped lifecycle state with cleared runtime references, leaving no stale running markers behind.
- **FR-013**: Concurrent stop calls for the same VM MUST coordinate so that at most one shutdown sequence runs and all callers observe a consistent stopped result.
- **FR-014**: All expected failure paths (unknown VM, invalid input, undeliverable shutdown request, unforceable still-running machine, failed forced termination) MUST be returned as typed errors; the operation MUST NOT panic, terminate the calling process, write to standard output or error, emit logs, or hide behavior in global side effects. A graceful request that cannot be delivered surfaces as a typed error with no forced-termination attempt.

### Key Entities

- **Stop request**: The caller's intent to stop one machine: the unique VM name resolved against the explicit SDK home. Nothing else comes from the caller.
- **Verified machine state**: The call-time answer (running vs. stopped) derived from control-socket responsiveness. An answering socket decides running; anything else decides stopped.
- **Graceful shutdown request**: The message sent through the machine's control socket asking the machine to shut down cleanly. The exact message shape is decided in planning.
- **Exit wait**: The 60-second observation window after the graceful request (matching the start operation's readiness deadline) during which the operation watches for the machine process to exit and the socket to go silent.
- **Stop result**: The outcome of the operation: the final machine state (always stopped on success) plus whether forced termination was used.
- **Stored machine record**: The durable inventory entry for the VM: identity, socket path, and process reference used as probe inputs and cleared when the machine reaches stopped.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A caller can stop a cooperative running VM by name and receive a result reporting stopped with forcing marked as not used — 100% of trials show a silent control socket and no remaining machine process afterwards.
- **SC-002**: Stopping an already-stopped machine returns success reporting stopped with forcing marked as not used and zero host process changes — 100% of trials across stopped, never-started, and externally-killed machines.
- **SC-003**: A running machine that ignores the graceful request reaches stopped with the result reporting forced termination — 100% of trials against a machine that never exits on request.
- **SC-004**: Unknown and malformed VM names return actionable typed errors with no host changes, no panics, and no unsolicited SDK output — 100% of failure trials.
- **SC-005**: Every stop call returns within 3 minutes on a capable host, including the forced path with its bounded wait.

## Assumptions

- "Running" means the machine's volume-local control socket answers at call time; "stopped" means anything else, consistent with live state verification. These are the only two states.
- The graceful shutdown travels over the volume-local control socket; the exact control message is a planning decision.
- The exit-wait bound is 60 seconds with continuous liveness probing, consistent in style with the start operation's bounded socket readiness wait.
- Forced termination uses the recorded process identity only when it still references this VM, preserving PID-reuse protection; a live unrelated process under a recycled identifier is never signaled.
- A graceful request that cannot be delivered surfaces as a typed error with no forced fallback.
- The SDK home is provided explicitly by the caller; the SDK discovers nothing from environment variables.
- Command-line presentation of stop (command shape, output, confirmations) is out of scope; this feature covers the SDK operation only.
- Starting, rebooting, deleting, listing, inspecting, and connection flows are out of scope except that they observe the stopped state afterwards.
- Multiple independent MicroVMs per host remain first-class: stop is always keyed to one machine with no single-VM assumption.
