# Feature Specification: CLI `ls` Command for Listing MicroVMs

**Feature Branch**: `014-cli-ls-command`

**Created**: 2026-09-21

**Status**: Draft

**Input**: User description: "atualmente já existe função para listar as microvms da pessoa e o estado delas pelo sdk, desenvolva um comando na CLI que é o microvm ls (que pode ser chamado pelo alias de microvm list também) que basicamente vai mostrar para a pessoa de uma forma moderna todas as microvms dele, o estado, tamanho, infos... capacidades... é um comando para ver os detalhes da microvm. Assim que a pessoa execultar ele tem que ter a lógica de sudo para pedir ao usuário o acesso root caso seja nessesário, veja como funciona essa questão do acesso root no comando de stop. Talvez seja preciso mecher no SDK para a rota listar mais algumas infos das maquinas como as capacidades e mais... porém envite ao máximo mecher no sdk, e mecha o minimo possivel lá"

## Clarifications

### Session 2026-09-21

- Q: Should the sizes shown by `microvm ls` be the configured values chosen at creation, the actual measured usage on disk, or both? → A: Configured values only.
- Q: Should each `microvm ls` row also show the machine's network details (guest address, LAN address when present, network mode)? → A: Include network details.
- Q: Should `microvm ls` render all machines as a single compact table with one row per machine, or as detailed blocks with one section per machine? → A: Single compact table.
- Q: Should `microvm ls` rows also include SSH connection details (user, port, key path)? → A: No SSH details.
- Q: How should `microvm ls` render a degraded entry whose record is incomplete or whose state probe timed out? → A: Show row with dashes.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - See All Machines at a Glance (Priority: P1)

As a host operator, I want to run `microvm ls` and see every MicroVM I own in one modern overview — name, lifecycle state, capacity details (CPUs, memory, disk, image reference), and network details (guest address, LAN address when present, network mode) — so that I know what exists and what is running without memorizing names or running one command per machine.

**Why this priority**: This is the whole value of the feature. One read-only command answers "what do I have and what state is it in". It is independently useful with no other new behavior.

**Independent Test**: With several machines in mixed states (running, stopped), run `microvm ls` (and repeat with `microvm list`), approve elevation when asked, and verify every machine appears exactly once with its correct state and capacity details in a single readable view.

**Acceptance Scenarios**:

1. **Given** machines `web-01` (running) and `db-01` (stopped) exist, **When** the operator runs `microvm ls`, **Then** both machines appear exactly once with the correct state label each and their capacity details, ordered deterministically by name.
2. **Given** the operator runs `microvm list` instead of `microvm ls`, **When** the flow proceeds, **Then** the output and behavior are identical to `microvm ls` (pure alias, no separate behavior).
3. **Given** machines exist in mixed states, **When** the listing renders, **Then** running versus stopped is distinguishable by explicit text labels (never by color alone) and each row carries the machine's configured capacities in human-readable units.

---

### User Story 2 - Privileged Listing Through the Existing Elevation Flow (Priority: P1)

As a host operator, I want `microvm ls` to secure root access through the same privilege flow the `stop` command uses, so that the listing (including live state verification) always runs with the rights it needs and I get one consistent elevation experience across commands.

**Why this priority**: State verification touches host-local runtime state that may require root. Without the gate, the command either fails obscurely or reports wrong states. Reusing the established flow avoids a second escalation mechanism.

**Independent Test**: Run `microvm ls` unprivileged in an interactive terminal and verify an elevation prompt appears and the listing follows after approval; run it unprivileged without a terminal (or with the non-interactive flag) and verify it fails fast with rerun guidance before any listing attempt.

**Acceptance Scenarios**:

1. **Given** the operator runs `microvm ls` without root in an interactive terminal, **When** the flow begins, **Then** it requests elevation through the existing CLI privilege flow (prompt-driven escalation, same cases and behavior as `stop`) before attempting any listing.
2. **Given** the operator declines elevation or interrupts the prompt, **When** the command exits, **Then** nothing is listed, the command reports the operation as cancelled, and no host state changes.
3. **Given** the command runs without root in a non-interactive or non-terminal context, **When** it starts, **Then** it fails before any listing attempt with guidance to rerun with root, and never prompts.

---

### User Story 3 - Honest Empty and Failure States (Priority: P2)

As a host operator with no machines yet (or with a broken local inventory), I want `microvm ls` to tell me plainly what happened and what to do next, so that an empty screen is never confused with a broken command.

**Why this priority**: Empty inventory is the first-run experience. A calm empty state pointing to creation converts confusion into action; honest failure states preserve trust in the tool.

**Independent Test**: Run `microvm ls` with an empty inventory and verify a calm "no machines" report pointing to the creation command; corrupt or lock the local inventory and verify a calm typed-error report with a concrete next step and nonzero exit.

**Acceptance Scenarios**:

1. **Given** no MicroVM exists in the local inventory, **When** the operator runs `microvm ls` privileged, **Then** the command reports that no machines exist yet, points to the creation command, and exits successfully without showing an empty table.
2. **Given** the local inventory is unavailable (missing base directory, unreadable database, locked state), **When** the operator runs `microvm ls`, **Then** the command reports what happened, why the listing could not be produced, and what the operator can do next, with a nonzero exit status.
3. **Given** scripted use with no interactive terminal, **When** the command runs privileged, **Then** it produces deterministic output with no prompts, spinners, or pager, suitable for scripts.

---

### Edge Cases

- The inventory is empty; the command reports "no machines" with a creation pointer instead of an empty table.
- The operator is unprivileged and declines elevation or interrupts; the command reports cancellation with no listing and no side effects.
- The command runs unprivileged in non-interactive or non-terminal mode; it fails fast with rerun-with-root guidance and never prompts.
- The local base directory is missing or unwritable, or the inventory database is unreadable or locked; the command surfaces the typed failure calmly with the affected path and a next step.
- A machine row is incomplete (interrupted creation) or its live-state probe times out; the listing still completes for all other machines and marks that entry honestly (stopped or degraded) instead of aborting the whole listing.
- The terminal is narrow, has no color support, or is non-interactive; state remains distinguishable through text labels and symbols, and scripted output stays deterministic.
- The listing is strictly read-only: running it never creates, starts, stops, deletes, or mutates any machine or host state.
- `ls` and `list` are a single command with an alias; no flag or environment difference may exist between the two spellings.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The CLI MUST expose the command as `microvm ls` with `microvm list` as an equivalent alias for the same command. Both spellings MUST produce identical behavior and output; the alias MUST NOT be a separate command path.
- **FR-002**: The command MUST be read-only over the local inventory: it MUST list machines and MUST NOT create, configure, start, stop, reboot, delete, or otherwise mutate any machine, runtime process, or persisted state.
- **FR-003**: The listing MUST reuse the existing SDK listing operation as its data source. SDK changes are allowed only as a strictly minimal additive extension (extra already-persisted capacity and network fields on the listing result) and MUST NOT change existing SDK behavior, lifecycle semantics, error contracts, or persistence layout.
- **FR-004**: Each listed entry MUST show the machine name, the call-time verified lifecycle state, the machine's configured capacities and identity details already stored at creation (virtual CPU count, memory size, disk size, and distribution/image reference), and the machine's stored network details (guest address, network mode, plus LAN address when present). SSH connection details (user, port, key paths) MUST NOT be shown. All sizes are the configured values chosen at creation, never live-measured usage. Sizes MUST render in human-readable units.
- **FR-005**: State MUST be shown with explicit text labels (running, stopped, plus any honestly-degraded marker the data source reports) and MUST NOT be communicated by color alone. Entries MUST appear exactly once, ordered deterministically by name.
- **FR-006**: When not already running with root access, the command MUST request elevation through the existing CLI privilege flow with the same cases and behavior as the `stop` command: prompt-driven escalation in interactive terminals before any listing attempt, hard error with rerun guidance in non-interactive or non-terminal contexts. The command MUST NOT implement its own escalation mechanism.
- **FR-007**: The command MUST support a non-interactive mode consistent with the established commands: no prompts of any kind, root required up front, deterministic output. Elevation prompting MUST NOT occur in non-interactive mode.
- **FR-008**: An empty inventory MUST produce a calm "no machines" report pointing to the creation command with a success exit status, not an empty table and not an error.
- **FR-009**: Expected failures (missing/unreadable inventory, database or base-directory failure, listing-operation failure) MUST explain what happened, why no listing was produced, and what the operator can do next, with a nonzero exit status and no unexplained output.
- **FR-010**: A single degraded entry (incomplete record, timed-out state probe) MUST NOT abort the whole listing: the command MUST still list every readable machine and keep the degraded row in place with dashes for any missing capacity or network value instead of omitting it or failing everything.
- **FR-011**: The interactive presentation MUST be a single compact table with one row per machine, following the restrained neutral visual language of the established flows: compact information density, clear hierarchy, semantic status emphasis, progressive disclosure, immediate feedback, and calm failure states. The command MUST NOT use pagers, interactive selectors, or destructive-action confirmations.
- **FR-012**: The command MUST NOT introduce new per-machine drill-down, filtering, sorting, or output-format flags beyond what is needed for parity with the established non-interactive behavior; any such capability is out of scope for this feature.

### Key Entities *(include if feature involves data)*

- **Machine Listing**: The full set of locally known MicroVMs shown by one `ls`/`list` invocation — one row per machine, ordered by name, each with name, verified state, stored capacity/identity details, and stored network details.
- **Listing Entry**: A single machine's overview: stable name, call-time verified lifecycle state, configured capacities (vCPUs, memory, disk), distribution/image reference, and stored network details (guest address, network mode, LAN address when present).
- **Empty Inventory**: The state where no machine exists locally; rendered as a creation-pointing report, not a table and not an error.
- **Elevation Gate**: The existing privilege flow reused unchanged: interactive prompt-driven escalation or non-interactive hard error, securing root before any listing attempt.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In usability tests with at least three machines in mixed states, an operator can run `microvm ls` and identify every machine's name, state, and capacities from the single rendered view in under 30 seconds without running any other command.
- **SC-002**: In 100% of alias tests, `microvm list` produces output identical to `microvm ls` for the same inventory and privilege context.
- **SC-003**: In 100% of listing tests against a known inventory, every stored machine appears exactly once, in name order, with the correct verified state and matching configured capacities.
- **SC-004**: In 100% of unprivileged interactive tests, the command prompts for elevation through the existing flow before listing and completes the listing after approval; in 100% of declined-elevation tests it reports cancellation with no listing and no host changes.
- **SC-005**: In 100% of unprivileged non-interactive tests, the command fails before any listing attempt with rerun guidance and performs no prompts.
- **SC-006**: In 100% of empty-inventory tests, the command reports "no machines" with a creation pointer and a success exit status; in 100% of broken-inventory tests it returns a nonzero status with the cause and a concrete next step.
- **SC-007**: In 100% of regression checks, previously available behaviors (creation, start, stop including its privilege behavior, `ssh` including its selector and privilege behavior, existing SDK listing results) behave exactly as before; the only new observable capability is the `ls`/`list` command, with SDK changes (if any) strictly additive.

## Assumptions

- The existing SDK `list_microvms` operation (name plus call-time verified state, ordered by name, probe failure resolving to stopped without aborting) exists unchanged and is the default data source; planning decides whether a minimal additive field extension is needed for capacities.
- All capacity, identity, and network details shown (vCPUs, memory, disk, distribution/image reference, guest address, network mode, LAN address when present) are already persisted at creation time; this feature introduces no new persistence, no new registry access, and no filesystem size probing.
- The elevation mechanics are owned entirely by the existing privilege flow and unchanged here; the `ls` child re-execution carries no machine name because the command takes none.
- The command takes no positional arguments and no per-machine selection; showing one machine's full detail beyond the row it already has in the overview is out of scope.
- Listing completes in under 30 seconds on a capable host for a typical inventory (tens of machines); full guest-OS introspection beyond stored configuration plus socket liveness is out of scope.
- All operator-facing text for this feature is English; localization is out of scope.
