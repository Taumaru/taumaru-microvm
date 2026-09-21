# Feature Specification: CLI `stop` Command for Stopping a MicroVM

**Feature Branch**: `013-cli-stop-command`

**Created**: 2026-09-21

**Status**: Draft

**Input**: User description: "vamos construir o comando "microvm stop", ele deve ser chamado como "microvm stop" ou "microvm stop {nome da maquina}. A primeira coisa que ele deve fazer é pedir o acesso root igual ao comando de ssh faz, que é: caso ele não tenha acesso root e esteja no modo interativo ele pede o acesso, caso ele não tenha e não esteja ele da erro... Olha o comando de ssh que lá já faz isso. Depois, caso ele não tenha passado o nome no processo, ele tem que abrir uma lista (caso no modo interativo) para ele selecionar uma das maquinas que estão rodando! Caso não esteja no modo inteirativo ele da um erro, pq tem que ser rodado com o nome, mas uma vez para a parte de seleção de maquina rodando faça exatamente como o comando de ssh faz para selecionar. Uma vez que ela selecione a maquina rodando, ele tem que chamar a função de stop microvm do sdk, mas tome cuidado pq essa função precisa de acesso root, então caso não tenha pedido o acesso root com base na logica de não interativo ele tem que obrigatoriamente pedir aqui!"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Stop a Named Running Machine (Priority: P1)

As a host operator, I want to run `microvm stop web-01` and have that machine shut down so that reclaiming a known running machine takes one command with no menus.

**Why this priority**: The named form is the fastest path and the scriptable form (with a non-interactive mode that performs no prompts). It delivers the full value of the feature on its own: from an established machine name to a stopped machine with a clear graceful-versus-forced report.

**Independent Test**: With a machine running, run `microvm stop web-01`, approve elevation when asked, and verify the machine reaches stopped state and the success output names the stopped VM and reports whether forcing was used; repeat scripted with an explicit name and no terminal, and verify no prompts occur and the exit status reflects the stop result.

**Acceptance Scenarios**:

1. **Given** machine `web-01` is running and the operator runs `microvm stop web-01`, **When** the flow proceeds, **Then** it skips any machine selector, requests elevation through the existing privilege flow when not already privileged, invokes the single SDK stop operation with `web-01`, and reports the machine as stopped with the forcing outcome.
2. **Given** the named machine does not exist locally, **When** the command validates the name, **Then** it reports the machine as not found with the affected name, points to creation, and stops nothing.
3. **Given** the named machine exists but is already stopped, **When** the command runs, **Then** it still invokes the SDK stop operation (which succeeds idempotently) and reports the machine as stopped with no forcing used, stopping no process.
4. **Given** scripted use with an explicit name and no interactive terminal, **When** the command runs, **Then** it performs no prompts of any kind, root access is required up front instead of prompting for elevation, and any missing requirement fails before any stop attempt with usage guidance.

---

### User Story 2 - Pick a Running Machine Interactively (Priority: P1)

As a host operator, I want to run bare `microvm stop` and pick one of my running machines from a selector so that I do not need to memorize exact names.

**Why this priority**: Operators routinely run several machines. A selector restricted to running machines prevents the wasted round trip of picking a stopped machine and then stopping nothing, and it mirrors the established `ssh` selection behavior the operator already knows.

**Independent Test**: Run bare `microvm stop` with several machines in mixed states (running, stopped, being created), verify only running machines are offered with the same presentation and behavior as the `ssh` selector, select one with the keyboard, approve elevation, and verify that exact machine reaches stopped state.

**Acceptance Scenarios**:

1. **Given** the operator runs bare `microvm stop` in an interactive terminal, **When** the flow begins, **Then** it first passes through the privilege step (prompt-driven escalation when not already privileged, exactly like the `ssh` command), then presents a single-select list containing only running machines, with keyboard navigation, confirmation, and cancellation behaving exactly like the `ssh` selector.
2. **Given** no machine is currently running, **When** the selector source returns empty, **Then** the command reports that there is nothing to stop, points to the start command, and stops nothing.
3. **Given** the operator cancels the selector or declines elevation, **When** the command exits, **Then** nothing is stopped and the command reports that the operation was cancelled.

---

### User Story 3 - Privileged Stop With Honest Progress and Outcome (Priority: P2)

As a host operator stopping a machine, I want the command to guarantee it runs privileged before touching the machine, show honest progress while the guest shuts down, and tell me whether the shutdown was graceful or forced so that I know whether the guest cooperated.

**Why this priority**: The SDK stop operation requires root and can take up to a bounded wait (graceful window plus a forced fallback). A stop that runs unprivileged, hangs silently, or hides a forced kill leaves the operator unable to trust the result.

**Independent Test**: Stop a cooperative machine and an unresponsive machine (one that ignores the graceful request), and verify both runs show a live status indicator while stopping, then report stopped with forcing marked as not used (cooperative) and used (unresponsive) respectively.

**Acceptance Scenarios**:

1. **Given** a machine is resolved (typed or selected) and the flow has not yet secured elevation (for example a non-interactive-shaped path that reached this point without the up-front gate), **When** the command approaches the SDK stop call, **Then** it mandatorily requests elevation through the existing privilege flow before invoking the SDK operation, so the stop call itself always runs privileged.
2. **Given** the SDK stop operation is in flight, **When** the machine is shutting down, **Then** the command shows a live spinner or status line consistent with the `start` flow, then replaces it with the stopped report on success. The indicator MUST NOT invent progress values.
3. **Given** the machine exits on its own after the graceful request, **When** the command reports success, **Then** it reports stopped with forcing marked as not used.
4. **Given** the machine ignores the graceful request and the SDK force-terminates it, **When** the command reports success, **Then** it reports stopped with forcing marked as used, making the unclean shutdown visible to the operator.

---

### Edge Cases

- The name is empty, malformed, or not path-friendly; the command aborts immediately before any stop attempt with the naming rule restated and no re-prompt.
- A positional name and `--name` disagree; the command reports the mismatch and stops nothing.
- In `--non-interactive` mode the machine name is missing or root access is absent; the command fails before any stop attempt with usage guidance and never prompts.
- The named machine does not exist; the command reports it as not found with the affected name and points to creation without offering a creation shortcut.
- The named machine is already stopped; the command reports it as stopped with no forcing used (idempotent success, no error).
- The named machine is still being created (interrupted setup); the command surfaces the SDK lifecycle conflict calmly and stops nothing.
- No machines are running when the selector opens; the command reports that there is nothing to stop and suggests starting one first.
- The operator interrupts during the selector, elevation, or while the stop is in flight; selector/elevation interrupts report cancellation with no side effects and the standard interrupt exit status.
- The host-local base directory is unavailable or unwritable; the command explains the affected path and stops safely.
- The terminal has no color support, is narrow, or cannot receive interactive input; selector focus, selection, and success/failure status remain distinguishable through text and symbols.
- The stop targets exactly one machine; nothing else in the inventory is touched.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The CLI MUST expose the command as `microvm stop` with an optional positional machine name, an equivalent `--name` flag, and a `--non-interactive` flag, mirroring the `start` and `ssh` flag shapes. A bare `microvm stop` MUST open the running-machine selector; `microvm stop web-01` MUST skip the selector and use `web-01` directly. When both the positional name and `--name` are supplied they MUST agree, otherwise the command MUST report the mismatch and stop nothing.
- **FR-002**: In an interactive terminal without a name, the command MUST present a single-select step listing only running machines with keyboard navigation, confirmation, and cancellation, behaving exactly like the `ssh` command selector (same running filter, same prompt presentation, same cancellation semantics). Cancellation or an empty running set MUST exit without stopping anything and MUST explain the outcome (cancelled, or nothing running with a pointer to the start command).
- **FR-003**: When not already running with root access, the command MUST request elevation through the existing CLI privilege flow with the same cases and behavior as the `ssh` command: prompt-driven escalation in interactive terminals (including the pre-selector escalation on the bare path, so listing and selection happen privileged), hard error with rerun guidance in non-interactive or non-terminal contexts. The command MUST NOT implement its own escalation mechanism or rewrite privilege behavior.
- **FR-004**: After machine resolution (typed or selected) and immediately before the SDK stop call, the command MUST pass through the privilege gate again with the escalated child command carrying the resolved name (the `start`/`ssh` re-execution pattern), so that any path that reached this point without secured elevation still escalates before the stop call. The SDK stop call itself MUST always run privileged.
- **FR-005**: In non-interactive mode (`--non-interactive`) the command MUST perform no prompts of any kind: the machine name is required (positional or `--name`), root access is required up front, and any missing requirement MUST fail before any stop attempt with usage guidance. Elevation prompting MUST NOT occur in non-interactive mode.
- **FR-006**: An explicitly named machine MUST be validated with the shared naming rule; an invalid name MUST abort immediately with the naming rule restated and no re-prompt. An unknown name MUST be reported as not found with the affected name and a pointer to creation. A known-but-already-stopped machine MUST still flow into the SDK stop call (idempotent success) and be reported as stopped with no forcing used — unlike `ssh`, stop MUST NOT error on a not-running machine.
- **FR-007**: After name resolution and elevation, the command MUST invoke the single SDK stop operation with the resolved machine name and MUST NOT implement lifecycle behavior, socket messaging, process signaling, or state persistence itself.
- **FR-008**: On success the command MUST report the machine as stopped and MUST surface the SDK forcing outcome: a graceful stop reports stopped with no forcing used, a forced stop reports stopped with forcing marked as used (making the unclean shutdown explicit). The success output MUST include the command to start that machine again as the next step.
- **FR-009**: While the SDK stop operation is in flight, the command MUST show a live spinner or status line consistent with the `start` flow, then replace it with the stopped report on success. The indicator MUST NOT invent progress values.
- **FR-010**: Expected failures (unknown machine, invalid name, lifecycle conflict, stop-operation failure) MUST explain what happened, why the machine was not stopped, and what the operator can do next, with a nonzero exit status and no unexplained failure output. An unknown machine MUST point to `microvm new` without offering a creation shortcut.
- **FR-011**: Stopping one machine MUST NOT touch any other machine: the command resolves, stops, and reports exactly one machine by name.
- **FR-012**: The interactive presentation MUST follow the restrained neutral visual language used by the established flows: compact information density, clear hierarchy, semantic status emphasis, visible keyboard focus, progressive disclosure, immediate feedback, and calm failure states. State MUST NOT be communicated by color alone.

### Key Entities *(include if feature involves data)*

- **Stop Request**: The single machine name to stop, however supplied (positional argument, `--name` flag, or interactive selector choice over running machines).
- **Machine Choice**: The operator-selected running machine (selector path) or the directly named machine (named path), resolved to the unique name passed to the SDK stop operation.
- **Stop Result**: The SDK stop outcome: the final machine state (always stopped on success) plus whether forced termination was used, rendered as a graceful-versus-forced report.
- **Stop Guidance**: The failure and empty-state pointers shown instead of a stop: creation hint for unknown machines, start hint when nothing is running, start hint for restarting the stopped machine.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In usability tests with at least three machines in mixed states, an operator can run bare `microvm stop`, see only the running machines in a selector identical in behavior to the `ssh` selector, select one using only the keyboard, approve elevation, and reach the stopped report in under 60 seconds for a cooperative guest.
- **SC-002**: In 100% of named-stop tests against a running machine, the command stops that exact machine with no selector shown, elevation requested through the existing flow when unprivileged, and the stopped report naming the machine with the correct forcing outcome.
- **SC-003**: In 100% of non-interactive tests with an explicit name and root access, the command performs no prompts and produces a deterministic exit status and summary; in 100% of tests with the name omitted or root access missing, it reports the missing requirement and stops nothing.
- **SC-004**: In 100% of already-stopped tests, the command reports stopped with no forcing used, a nonzero-free (success) exit status, and zero host process changes.
- **SC-005**: In 100% of forced-stop tests against a machine that ignores the graceful request, the success output explicitly marks the stop as forced, distinguishable from a graceful stop by the report alone.
- **SC-006**: In 100% of failure-path tests (unknown machine, invalid name, lifecycle conflict, stop-operation failure), the command returns a nonzero status, identifies the failure, stops nothing it should not, and gives a concrete retry or repair action.
- **SC-007**: In 100% of regression checks, previously available behaviors (machine creation, start flow, `ssh` flow including its selector and privilege behavior) behave exactly as before this feature; the only new observable capability is the `stop` command itself.

## Assumptions

- The SDK stop operation from the `012-stop-microvm` feature exists with its contract unchanged: socket-governed liveness, idempotent already-stopped success, graceful request plus 60-second wait with SIGKILL fallback, and a result reporting stopped plus whether forcing was used; this command reuses all of it and adds no lifecycle semantics.
- Stopping a machine ends with a stopped (not deleted) machine; starting it again, deleting it, and inspecting it are out of scope except for the restart hint.
- No yes/no confirmation precedes the stop: elevation (when needed) is the only gate, consistent with `start` (immediate start, no confirmation) and with the project rule reserving strong confirmation for destructive actions over persistent data — stop preserves the machine and its data.
- The selector source reuses the same running-machines notion as the `ssh` selector (running filter over the local inventory); which SDK surface serves the listing is an implementation decision for planning.
- The elevation prefix and re-execution mechanics are owned entirely by the existing privilege flow and unchanged here; the stop child command carries the resolved name in non-interactive form.
- Every stop call completes within the SDK stop bound (graceful window plus forced fallback, under 3 minutes on a capable host); full guest-OS shutdown verification beyond the SDK result is out of scope.
- All operator-facing text for this feature is English; localization is out of scope.
