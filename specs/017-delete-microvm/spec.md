# Feature Specification: Delete MicroVM

**Feature Branch**: `017-delete-microvm`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Build an SDK operation that deletes a created MicroVM by name. It refuses with a stop-first error when the machine is running. It removes everything owned by that machine — its disk copy, SSH files, socket files, inventory record and child rows, and its machine-specific host network configuration — while preserving shared artifacts such as kernels and distribution images."

## Clarifications

### Session 2026-09-22

- Q: Should the delete operation return a result value describing what was deleted, or just succeed with no value? → A: Return a small result carrying the deleted machine name.
- Q: Should deleting a machine that is already gone (unknown name because it was deleted earlier) succeed or fail? → A: Fail with not-found; only an existing record can be deleted.
- Q: When part of the deletion succeeds but a later step fails, what should a retry do? → A: Keep the record on failure and require full cleanup on success, so retrying finishes the job. Owned network items that are already absent are skipped, not failed; only a fully unknown machine name errors as not-found.
- Q: Should the machine's private volume directory itself be removed, or only its contents? → A: Remove the entire volume directory, including the directory itself.
- Q: When the owned volume directory or an owned file cannot be removed (for example a permission problem), should the delete operation keep the record and fail, or remove the record anyway? → A: Keep the record and fail, so a retry resumes the remaining removal.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Delete a stopped machine and all its owned traces (Priority: P1)

An application developer deletes a stopped MicroVM by its unique name. The operation removes the machine's per-VM disk copy, its SSH key files, its control socket files, its inventory record with all child rows, and every host network item recorded as owned by that machine. Shared kernels, distribution images, and tool binaries stay fully intact, and the deleted name no longer resolves afterwards.

**Why this priority**: This is the core value of the feature: full, scoped removal of one machine with a single call, freeing its disk, credentials, and host network footprint without endangering anything shared.

**Independent Test**: Create a MicroVM, stop it, call the delete operation with the VM name, then verify the name resolves to not-found, the per-VM disk, SSH, and socket files are gone, the inventory record is gone, the machine's host network resources are released, and the referenced kernel and image remain usable by other machines.

**Acceptance Scenarios**:

1. **Given** a stopped MicroVM with its disk copy, SSH files, socket file, inventory record, and host network attachment present, **When** the caller deletes it by name, **Then** the operation succeeds and returns a result carrying the deleted machine name, and every item owned by that machine is gone: disk copy, SSH files, socket files, inventory record with child rows, and machine-specific host network configuration.
2. **Given** a successful deletion, **When** the caller looks the same name up afterwards, **Then** the lookup reports the machine as not found, and deleting that same name again returns a typed not-found error rather than success.
3. **Given** a deleted machine that referenced a kernel and distribution image also used by a remaining machine, **When** the remaining machine starts or lists afterwards, **Then** it resolves its kernel and image exactly as before with no re-download required.
4. **Given** a stored machine whose owned files were already partially removed outside the SDK (e.g. the disk copy was manually deleted), **When** the caller deletes it by name, **Then** the operation still succeeds as long as the end state holds: no record and no owned resources remain.

---

### User Story 2 - Deleting a running machine is refused with a stop-first error (Priority: P1)

An application developer deletes a machine that is currently running. The operation refuses with a typed error telling the caller to stop the machine first, changing nothing on the host: no files removed, no network touched, no record altered.

**Why this priority**: The caller explicitly requires this guard. Deleting under a live machine would orphan a running process and corrupt host network accounting; an explicit refusal keeps deletion safe and the recovery path obvious (stop, then delete).

**Independent Test**: Start one MicroVM, call delete against its name, verify a stop-first error is returned and every owned file, the record, and the network attachment are byte-for-byte unchanged; then stop the machine, delete again, and verify success.

**Acceptance Scenarios**:

1. **Given** a genuinely running MicroVM (its control socket answers), **When** the caller deletes it by name, **Then** the operation returns a typed lifecycle-conflict error directing the caller to stop the machine first, with zero host changes.
2. **Given** a refused running-machine delete, **When** the caller stops the machine and deletes it again, **Then** the second call succeeds and removes everything owned by the machine.

---

### User Story 3 - Host network cleanup is scoped to exactly this machine (Priority: P2)

An application developer deletes one of several machines on the same host. The operation releases only the host network configuration recorded as owned by the deleted machine. Other machines keep their addresses, routes, and attachments, and host configuration not owned by the SDK is never touched.

**Why this priority**: Network state is host-global and shared by nature; an over-broad cleanup would disconnect surviving machines or damage the operator's own networking. Scoped cleanup is what makes per-machine deletion safe on a multi-VM host.

**Independent Test**: Create and network two machines, delete one, and verify the deleted machine's owned host resources are released while the surviving machine's connectivity, addresses, and routes are unchanged.

**Acceptance Scenarios**:

1. **Given** two machines with their own network attachments on one host, **When** the caller deletes one of them, **Then** only the deleted machine's owned host network items are released and the surviving machine is fully unaffected.
2. **Given** host network configuration that the SDK did not create and does not own, **When** any delete runs, **Then** that configuration is left untouched.

---

### Edge Cases
- What happens when the VM name is unknown? The operation returns a typed not-found error with no host changes.
- What happens when the VM name is empty or malformed? The operation rejects the request with a typed invalid-input error before touching host state.
- What happens when the machine's control socket cannot be probed (probe failure rather than clean silence)? The machine MUST NOT be treated as safely stopped for deletion; the operation returns a typed error instead of deleting under a possibly live machine.
- What happens when an owned file is already missing (manual removal, previous partial delete)? The missing item counts as already removed; deletion continues and succeeds when the end state holds.
- What happens when releasing an owned host network item fails? The operation keeps the record, returns a typed error, and MUST NOT report the machine as deleted; the caller retries the same delete to finish the job.
- What happens when an owned host network item is already absent (rule, route, or address gone outside the SDK)? The absent item is skipped as already removed, exactly like an already-missing owned file; only a fully unknown machine name errors as not-found.
- What happens when a delete is retried after a partial failure? The retry resumes deletion from the remaining owned resources and succeeds once no record and no owned resources remain.
- What happens when two delete calls run concurrently for the same VM? They coordinate so that at most one deletion sequence runs and all callers observe a consistent outcome.
- What happens to shared artifacts (kernels, distribution images, tool binaries, caches)? Nothing: they are never deletion targets, even when the deleted machine referenced them.
- What happens to a stored record left behind by an interrupted creation? It is deletable like any stopped record, so failed creations can be cleaned up.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The delete operation MUST accept the explicit SDK home directory and the unique MicroVM name from its caller, and MUST NOT require any other input.
- **FR-002**: The operation MUST validate the VM name before changing host state, rejecting empty or malformed names with typed invalid-input errors.
- **FR-003**: The operation MUST verify the VM exists in the local inventory by name and return a typed not-found error without host changes when it does not.
- **FR-004**: The operation MUST decide running versus stopped at call time from control-socket responsiveness (an answering socket means running), using the same definition as the start and stop operations; it MUST NOT decide from stored state alone, process liveness alone, or socket file existence alone.
- **FR-005**: When the machine is running, the operation MUST refuse with a typed lifecycle-conflict error directing the caller to stop the machine first, removing no files, releasing no network resources, and altering no records.
- **FR-006**: On a stopped machine, the operation MUST remove the machine's entire private volume directory including the directory itself — the per-VM disk copy derived from the distribution image, the per-VM SSH key files, the control socket file(s), and all remaining volume-local files — so no empty directory is left behind.
- **FR-007**: The operation MUST remove the machine's inventory record together with all of its child rows (network, credential, and runtime metadata), so the name no longer resolves afterwards.
- **FR-008**: The operation MUST release every host network item recorded as created or owned by the SDK for this machine, and MUST leave host configuration it does not own untouched.
- **FR-009**: The operation MUST preserve shared artifacts: kernels, distribution images, tool binaries, and any cached content not owned exclusively by this machine MUST NOT be removed, even when the deleted machine referenced them.
- **FR-010**: The operation MUST apply to any stored record under the name, including leftovers from an interrupted creation, provided the machine is stopped at call time.
- **FR-011**: An owned file or directory that is already absent at deletion time MUST count as already removed and MUST NOT fail the operation.
- **FR-012**: When removing an owned file or the volume directory, releasing a present owned host network item, or removing the record fails, the operation MUST keep the record, return a typed error, and MUST NOT report the machine as deleted; retrying the same delete resumes from the remaining owned resources and succeeds once no record and no owned resources remain.
- **FR-013**: Concurrent delete calls for the same VM MUST coordinate so that at most one deletion sequence runs and all callers observe a consistent outcome.
- **FR-014**: The operation MUST target exactly one machine by name and MUST NOT touch other MicroVMs' files, network attachments, or records.
- **FR-015**: On success the operation MUST return a small result carrying the deleted machine name, so the caller can confirm which machine was deleted from the result alone.
- **FR-016**: All expected failure paths (unknown VM, invalid input, running machine, unprobable liveness, unremovable owned file or volume, unreleasable owned network item, unremovable record) MUST be returned as typed errors; the operation MUST NOT panic, terminate the calling process, write to standard output or error, emit logs, or hide behavior in global side effects.

### Key Entities

- **Delete request**: The caller's intent to delete one machine: the unique VM name resolved against the explicit SDK home. Nothing else comes from the caller.
- **Verified machine state**: The call-time answer (running vs. stopped) derived from control-socket responsiveness. An answering socket decides running; clean silence decides stopped; an unprobable socket is neither and blocks deletion with an error.
- **VM-owned files**: Everything private to the machine inside its volume directory: the per-VM disk copy of the distribution image, the per-VM SSH key files, the control socket file(s), and any remaining volume-local runtime files.
- **Shared artifacts**: Kernels, distribution images, tool binaries, and caches referenced — but never owned — by the machine. Explicitly outside the deletion scope.
- **Delete result**: The outcome of the operation: the deleted machine name. Returned on success only; failures surface as typed errors with no result.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A caller can delete a stopped VM by name and afterwards the name resolves to not-found, its disk copy, SSH files, and socket files are absent, its inventory record is absent, and its owned host network items are released — 100% of trials.
- **SC-002**: Deleting a running VM returns a stop-first error with zero host changes (owned files, record, and network attachment all unchanged), and deleting again after stopping succeeds — 100% of trials.
- **SC-003**: After deleting a machine that shared its kernel and image with a remaining machine, the remaining machine starts and runs with no re-download — 100% of trials.
- **SC-004**: Unknown and malformed VM names return actionable typed errors with no host changes, no panics, and no unsolicited SDK output — 100% of failure trials.
- **SC-005**: Deleting one machine on a multi-VM host leaves every other machine fully operable (its files, network attachment, and lifecycle flows unaffected) — 100% of trials.
- **SC-006**: Every delete call returns within 2 minutes on a capable host, including full file removal and host network cleanup.

## Assumptions

- "Running" means the machine's volume-local control socket answers at call time; "stopped" means clean silence, consistent with the start and stop operations. An unprobable socket is treated as unknown and blocks deletion.
- The SDK home is provided explicitly by the caller; the SDK discovers nothing from environment variables.
- The exact file layout below the per-VM volume directory (disk filename, SSH filenames, socket filename) is a planning decision; the spec constrains categories of owned content, not filenames.
- The exact host mechanism used to release owned network items (links, addresses, routes, rules) is a planning decision; the spec constrains ownership scope, not mechanism.
- Command-line presentation of delete (command shape, output, confirmations) is out of scope; this feature covers the SDK operation only.
- Starting, stopping, rebooting, listing, inspecting, and connection flows are out of scope except that they observe the deleted name as not-found afterwards.
- Multiple independent MicroVMs per host remain first-class: delete is always keyed to one machine with no single-VM assumption.
