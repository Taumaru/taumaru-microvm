# Feature Specification: CLI `delete` Command for Deleting a MicroVM

**Feature Branch**: `018-cli-delete-command`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Create the CLI command for the delete capability: reachable as `microvm delete` with an optional machine name, the same privilege flow as the sibling commands, a machine selector when no name is given, an explicit deletion confirmation, and a loading indicator while the deletion runs."

## Clarifications

### Session 2026-09-22

- Q: When the operator runs bare `microvm delete` with no name, which machines should the interactive selector list? → A: All stored machines with state labels; picking a running one reports the stop-first step.
- Q: In scripted runs with an explicit name and no terminal, should the command still ask for the deletion confirmation? → A: No confirmation in scripted mode; the explicit name plus root access carry the intent.
- Q: After a machine is deleted, what should the success output suggest as the next step? → A: Point to the creation command for making a new machine.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Delete a Named Machine With Confirmation (Priority: P1)

As a host operator, I want to run `microvm delete web-01`, confirm the deletion, and watch honest progress while it runs, so that removing a known machine takes one command with no menus and no doubt about what happened.

**Why this priority**: The named form is the fastest path and the scriptable form (with a non-interactive mode that performs no prompts). It delivers the full value of the feature on its own: from an established machine name to a deleted machine, guarded by elevation plus one explicit confirmation, as required for destructive actions over persistent data.

**Independent Test**: With a stopped machine, run `microvm delete web-01`, approve elevation when asked, confirm the shown deletion prompt, and verify the machine is gone (name no longer resolves), the success output names the deleted machine, and shared kernels and images still resolve for other machines; repeat scripted with an explicit name and no terminal, and verify no prompts occur and the outcome is deterministic.

**Acceptance Scenarios**:

1. **Given** stopped machine `web-01` and the operator runs `microvm delete web-01`, **When** the flow proceeds, **Then** it skips any machine selector, requests elevation through the existing privilege flow when not already privileged, shows an explicit confirmation naming `web-01` with a permanent-loss warning, invokes the single SDK delete operation with `web-01` only after confirmation, shows a loading indicator while the deletion runs, and reports the machine as deleted with a success status.
2. **Given** the operator declines the confirmation or interrupts it, **When** the command exits, **Then** nothing is deleted and the command reports the operation as cancelled.
3. **Given** scripted use with an explicit name and no interactive terminal, **When** the command runs, **Then** it performs no prompts of any kind, root access is required up front instead of prompting for elevation, the deletion proceeds without a confirmation prompt, and any missing requirement fails before any deletion attempt with usage guidance.
4. **Given** a successful deletion, **When** the operator looks the same name up afterwards, **Then** the name resolves as not found, and deleting that same name again reports not found rather than success.

---

### User Story 2 - Pick a Machine Interactively (Priority: P1)

As a host operator, I want to run bare `microvm delete` and pick one of my machines from a selector so that I do not need to memorize exact names.

**Why this priority**: Operators routinely keep several machines. A selector over every stored machine prevents the wasted round trip of typing a wrong name, and it mirrors the established `ssh` selection behavior the operator already knows — except it lists machines in any state, because deletion applies to stopped, never-started, and incompletely created machines alike.

**Independent Test**: Run bare `microvm delete` with several machines in mixed states (running, stopped, never-started), verify all stored machines are offered with state labels and the same presentation and behavior as the `ssh` selector, select one with the keyboard, approve elevation, confirm the deletion prompt, and verify that exact machine is deleted.

**Acceptance Scenarios**:

1. **Given** the operator runs bare `microvm delete` in an interactive terminal, **When** the flow begins, **Then** it first passes through the privilege step (prompt-driven escalation when not already privileged, exactly like the sibling commands), then presents a single-select list containing all stored machines with their states labeled, with keyboard navigation, confirmation, and cancellation behaving exactly like the `ssh` selector.
2. **Given** no machine exists locally, **When** the selector source returns empty, **Then** the command reports that there is nothing to delete, points to the creation command, and deletes nothing.
3. **Given** the operator cancels the selector or declines elevation, **When** the command exits, **Then** nothing is deleted and the command reports that the operation was cancelled.
4. **Given** the operator selects a machine that is currently running, **When** the deletion runs, **Then** the command reports the stop-first refusal (what happened, why the machine was not deleted, and the exact stop-then-retry next step) and deletes nothing.

---

### User Story 3 - Privileged Delete With Honest Progress and Outcome (Priority: P2)

As a host operator deleting a machine, I want the command to guarantee it runs privileged before touching the machine, show honest progress while owned files and network attachments are released, and tell me plainly what was deleted or why nothing was, so that I can trust the result of an irreversible action.

**Why this priority**: The SDK delete operation requires root and removes the machine permanently (volume, credentials, network attachment, inventory record). A delete that runs unprivileged, hangs silently, or hides a failure leaves the operator unable to trust an irreversible result. The confirmation gate plus the loading indicator plus calm failure reports make the destructive path trustworthy.

**Independent Test**: Delete a stopped machine and verify the run shows a live status indicator while deleting, then a deleted report naming the machine; attempt to delete a running machine, an unknown machine, and with an unavailable inventory, and verify each failure names the cause with a concrete next step and a nonzero status.

**Acceptance Scenarios**:

1. **Given** a machine is resolved (typed or selected) and the flow has not yet secured elevation (for example a non-interactive-shaped path that reached this point without the up-front gate), **When** the command approaches the SDK delete call, **Then** it mandatorily requests elevation through the existing privilege flow before invoking the SDK operation, so the delete call itself always runs privileged.
2. **Given** the SDK delete operation is in flight, **When** the machine is being deleted, **Then** the command shows a live spinner or status line consistent with the `start`/`stop` flows, then replaces it with the deleted report on success. The indicator MUST NOT invent progress values.
3. **Given** the machine is currently running, **When** the command reports the refusal, **Then** it names the machine, states that it must be stopped first, points to the exact stop command for that machine, and deletes nothing.
4. **Given** the deletion fails midway (owned network item or volume cannot be released, record cannot be removed), **When** the command reports the failure, **Then** it states that nothing was reported as deleted, names the cause, and points to fixing the cause and retrying the same delete, with a nonzero status.

---

### Edge Cases

- The name is empty, malformed, or not path-friendly; the command aborts immediately before any deletion attempt with the naming rule restated and no re-prompt.
- A positional name and `--name` disagree; the command reports the mismatch and deletes nothing.
- In `--non-interactive` mode the machine name is missing or root access is absent; the command fails before any deletion attempt with usage guidance and never prompts.
- The named machine does not exist (including an already-deleted name); the command reports it as not found with the affected name and points to creation without offering a creation shortcut.
- The named machine is currently running; the command reports the stop-first refusal with the exact stop command for that machine and deletes nothing.
- The named machine is an interrupted-creation leftover; the command deletes it like any stopped record and reports success.
- No machines exist when the selector opens; the command reports that there is nothing to delete and suggests creating one first.
- The operator interrupts during the selector, elevation, confirmation, or while the deletion is in flight; selector/elevation/confirmation interrupts report cancellation with no side effects and the standard interrupt status.
- The host-local base directory is unavailable or unwritable; the command explains the affected path and deletes safely nothing.
- The terminal has no color support, is narrow, or cannot receive interactive input; selector focus, selection, confirmation, and success/failure status remain distinguishable through text and symbols.
- The delete targets exactly one machine; nothing else in the inventory is touched, and shared kernels and images are never removal targets.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The CLI MUST expose the command as `microvm delete` with an optional positional machine name, an equivalent `--name` flag, and a `--non-interactive` flag, mirroring the `start`/`stop`/`ssh` flag shapes. A bare `microvm delete` MUST open the machine selector; `microvm delete web-01` MUST skip the selector and use `web-01` directly. When both the positional name and `--name` are supplied they MUST agree, otherwise the command MUST report the mismatch and delete nothing.
- **FR-002**: In an interactive terminal without a name, the command MUST present a single-select step listing all stored machines in any state with state labels, keyboard navigation, confirmation, and cancellation, behaving exactly like the `ssh` command selector (same prompt presentation, same cancellation semantics). Cancellation or an empty machine set MUST exit without deleting anything and MUST explain the outcome (cancelled, or nothing to delete with a pointer to the creation command).
- **FR-003**: When not already running with root access, the command MUST request elevation through the existing CLI privilege flow with the same cases and behavior as the sibling commands: prompt-driven escalation in interactive terminals (including the pre-selector escalation on the bare path, so listing and selection happen privileged), hard error with rerun guidance in non-interactive or non-terminal contexts. The command MUST NOT implement its own escalation mechanism or rewrite privilege behavior.
- **FR-004**: After machine resolution (typed or selected) and immediately before the SDK delete call, the command MUST pass through the privilege gate again with the escalated child command carrying the resolved name (the `start`/`stop`/`ssh` re-execution pattern), so that any path that reached this point without secured elevation still escalates before the delete call. The SDK delete call itself MUST always run privileged.
- **FR-005**: In non-interactive mode (`--non-interactive`) the command MUST perform no prompts of any kind: the machine name is required (positional or `--name`), root access is required up front, and any missing requirement MUST fail before any deletion attempt with usage guidance. Elevation prompting MUST NOT occur in non-interactive mode. The explicit confirmation prompt MUST be skipped in non-interactive mode (the explicit name plus the root gate carry the intent, matching the sibling destructive flow).
- **FR-006**: An explicitly named machine MUST be validated with the shared naming rule; an invalid name MUST abort immediately with the naming rule restated and no re-prompt. An unknown name MUST be reported as not found with the affected name and a pointer to creation.
- **FR-007**: In interactive mode, after name resolution and elevation but before the SDK delete call, the command MUST show one explicit confirmation naming the resolved machine with a permanent-loss warning (defaulting to decline), and MUST delete only on explicit approval. Declining or interrupting the confirmation MUST cancel with no deletions and a cancelled report.
- **FR-008**: After confirmation (or directly in non-interactive mode), the command MUST invoke the single SDK delete operation with the resolved machine name and MUST NOT implement lifecycle behavior, file removal, network release, or state persistence itself.
- **FR-009**: While the SDK delete operation is in flight, the command MUST show a live spinner or status line consistent with the `start`/`stop` flows, then replace it with the deleted report on success. The indicator MUST NOT invent progress values.
- **FR-010**: On success the command MUST report the machine as deleted naming the deleted machine, confirm that its owned traces are gone (record, volume, owned network attachment), note that shared kernels and images are preserved, point to the creation command for making a new machine, and exit with a success status.
- **FR-011**: Expected failures (unknown machine, invalid name, running-machine refusal, delete-operation failure, retryable partial failure) MUST explain what happened, why the machine was not deleted, and what the operator can do next, with a nonzero status and no unexplained failure output. A running machine MUST point to the exact stop command for that machine; an unknown machine MUST point to `microvm new` without offering a creation shortcut; a retryable failure MUST point to fixing the cause and retrying the same delete.
- **FR-012**: Deleting one machine MUST NOT touch any other machine: the command resolves, confirms, deletes, and reports exactly one machine by name.
- **FR-013**: The interactive presentation MUST follow the restrained neutral visual language used by the established flows: compact information density, clear hierarchy, semantic status emphasis, visible keyboard focus, progressive disclosure, immediate feedback, and calm failure states. State MUST NOT be communicated by color alone.
- **FR-014**: The command MUST resolve the host-local base directory according to the existing CLI policy and pass that explicit path to the SDK; it MUST NOT create a second home-directory or artifact-storage policy.
- **FR-015**: The command MUST NOT change existing SDK behavior, lifecycle semantics, error contracts, or persistence layout; any SDK change is allowed only as a strictly minimal additive extension, and none is expected.

### Key Entities *(include if feature involves data)*

- **Delete Request**: The single machine name to delete, however supplied (positional argument, `--name` flag, or interactive selector choice over all stored machines).
- **Machine Choice**: The operator-selected machine (selector path) or the directly named machine (named path), resolved to the unique name passed to the SDK delete operation.
- **Deletion Confirmation**: The explicit interactive approval naming the resolved machine with a permanent-loss warning; declining or interrupting cancels with no deletions.
- **Delete Result**: The SDK delete outcome: the deleted machine name, rendered as a deleted report confirming owned traces are gone and shared artifacts are preserved.
- **Delete Guidance**: The failure and empty-state pointers shown instead of a deletion: creation hint for unknown machines, stop-then-retry hint for running machines, creation hint when nothing exists, retry hint for retryable failures.
- **Elevation Gate**: The existing privilege flow reused unchanged: interactive prompt-driven escalation or non-interactive hard error, securing root before any deletion attempt and again before the SDK call with the resolved name.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In usability tests with at least three machines in mixed states, an operator can run bare `microvm delete`, see every stored machine with state labels in a selector identical in behavior to the `ssh` selector, select one using only the keyboard, approve elevation, confirm the deletion prompt, and reach the deleted report in under 60 seconds.
- **SC-002**: In 100% of named-delete tests against a stopped machine, the command deletes that exact machine with no selector shown, elevation requested through the existing flow when unprivileged, one explicit confirmation approved, a loading indicator while deleting, and the deleted report naming the machine.
- **SC-003**: In 100% of non-interactive tests with an explicit name and root access, the command performs no prompts and produces a deterministic outcome and summary; in 100% of tests with the name omitted or root access missing, it reports the missing requirement and deletes nothing.
- **SC-004**: In 100% of declined-confirmation and interrupted tests (selector, elevation, confirmation, in-flight), the command deletes nothing and reports cancellation.
- **SC-005**: In 100% of running-machine tests, the command deletes nothing and the output names the machine, states the stop-first requirement, and gives the exact stop command for that machine.
- **SC-006**: In 100% of failure-path tests (unknown machine, invalid name, delete-operation failure, retryable partial failure), the command returns a nonzero status, identifies the failure, deletes nothing it should not, and gives a concrete retry or repair action.
- **SC-007**: In 100% of regression checks, previously available behaviors (machine creation, start/stop flows, `ssh` flow including its selector and privilege behavior, `ls`, prune) behave exactly as before this feature; the only new observable capability is the `delete` command itself.

## Assumptions

- The SDK delete operation from the `017-delete-microvm` feature exists with its contract unchanged: socket-governed liveness with stop-first refusal, whole volume-directory removal, owned network release, record deleted last with retry convergence, absent-owned-items convergence, and a result carrying the deleted name; this command reuses all of it and adds no lifecycle semantics.
- The selector lists all stored machines in any state with state labels (it does not filter to stopped machines): enforcement of the running guard stays in the SDK, so the CLI never duplicates the liveness decision and a selected running machine surfaces the stop-first refusal with its next step.
- The explicit confirmation is required in interactive mode (delete is destructive over persistent data, per the project confirmation rule) and skipped in non-interactive mode, where the explicit name plus the up-front root gate carry the intent — matching the sibling destructive flow.
- The elevation prefix and re-execution mechanics are owned entirely by the existing privilege flow and unchanged here; the delete child command carries the resolved name in non-interactive form.
- Every delete call completes within the SDK delete bound (under 2 minutes on a capable host); full disk and network verification beyond the SDK result is out of scope.
- Deleting a machine ends with no machine under that name; starting it again, recreating it, and inspecting it are out of scope except for guidance pointers.
- All operator-facing text for this feature is English; localization is out of scope.
