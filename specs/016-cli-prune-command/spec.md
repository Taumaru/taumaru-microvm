# Feature Specification: CLI `artifacts prune` Command

**Feature Branch**: `016-cli-prune-command`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Create the CLI command for the prune capability: it must be reachable as `microvm artifacts prune`."

## Clarifications

### Session 2026-09-22

- Q: Should `microvm artifacts prune` ask for confirmation before deleting in interactive mode? → A: Confirm with preview — show the reclaimed set preview and require an explicit yes before deleting; declining cancels with no deletions.
- Q: What should the interactive deletion preview list before asking for confirmation? → A: Identities plus freed space — every kernel and image identity to delete plus the estimated freed space.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Reclaim Disk With One Command (Priority: P1)

As a host operator, I want to run `microvm artifacts prune` and reclaim every downloaded kernel and image that no existing machine references, so that I free disk with one command without checking which artifact belongs to which machine.

**Why this priority**: This is the whole value of the feature. One command turns the existing SDK prune capability into disk space, with a preview plus one explicit confirmation and no machine-by-machine reasoning.

**Independent Test**: With several downloaded kernels and images where only a subset is referenced by existing machines (covering running, stopped, and never-started owners), run `microvm artifacts prune`, approve elevation when asked, confirm the shown preview, and verify every unreferenced artifact is gone, every referenced artifact is intact, and the output reports what was removed with the freed space.

**Acceptance Scenarios**:

1. **Given** downloaded kernels and images where some are referenced by at least one existing machine record and others are referenced by none, **When** the operator runs `microvm artifacts prune`, reviews the preview of the reclaimed set, and confirms, **Then** every unreferenced kernel and image is deleted, every referenced kernel and image is left fully intact, and the command reports the removed kernels, the removed images, the count removed per kind, and the freed space per kind and in total, with a success exit status.
2. **Given** a kernel shared by two existing machines where one of those machines was later deleted, **When** the operator runs `microvm artifacts prune`, **Then** the shared kernel is kept because one remaining machine still references it, and the output distinguishes kept artifacts from removed ones only by listing what was removed.
3. **Given** a machine that exists but is stopped, was never started, or whose process died outside the CLI, **When** the operator runs `microvm artifacts prune`, **Then** the artifacts it references are treated as in use and are never deleted.
4. **Given** a successful prune, **When** the operator later creates, starts, or lists any remaining machine, **Then** that machine resolves its kernel and image exactly as before, with no re-download required.

---

### User Story 2 - Privileged Prune Through the Existing Elevation Flow (Priority: P1)

As a host operator, I want `microvm artifacts prune` to secure root access through the same privilege flow the sibling commands use, so that artifact deletion always runs with the rights it needs and I get one consistent elevation experience across commands.

**Why this priority**: Artifact directories and the local inventory may require root. Without the gate, the command either fails obscurely or deletes half a plan. Reusing the established flow avoids a second escalation mechanism.

**Independent Test**: Run `microvm artifacts prune` unprivileged in an interactive terminal and verify an elevation prompt appears and the prune follows after approval; run it unprivileged with `--non-interactive` and verify it fails fast with rerun guidance before any deletion attempt.

**Acceptance Scenarios**:

1. **Given** the operator runs `microvm artifacts prune` without root in an interactive terminal, **When** the flow begins, **Then** it requests elevation through the existing CLI privilege flow (prompt-driven escalation, same cases and behavior as the sibling commands) before attempting any deletion.
2. **Given** the operator declines elevation or interrupts the prompt, or declines the deletion preview, or interrupts the confirmation, **When** the command exits, **Then** nothing is deleted, the command reports the operation as cancelled, and no host state changes.
3. **Given** the command runs without root in a non-interactive context, **When** it starts, **Then** it fails before any deletion attempt with guidance to rerun with root, and never prompts.

---

### User Story 3 - Honest Empty and Failure States (Priority: P2)

As a host operator with nothing to reclaim (or with a broken local inventory), I want `microvm artifacts prune` to tell me plainly what happened and what to do next, so that an empty result is never confused with a broken command and a partial failure never hides what still needs repair.

**Why this priority**: Prune will often run on a clean host, and deletions can fail for host reasons (permissions, locks, I/O). A calm no-op plus an honest partial-failure report keep the command trustworthy to run blindly.

**Independent Test**: Run `microvm artifacts prune` with a fully-referenced inventory and with an empty inventory and verify a calm "nothing to prune" report with success status and no deletions; seed one undeletable unreferenced artifact and verify the command reclaims everything else, reports the removed set with freed space so far, names each failed artifact with its cause, and returns a nonzero status.

**Acceptance Scenarios**:

1. **Given** every downloaded kernel and image is referenced by an existing machine, or nothing is downloaded at all, **When** the operator runs `microvm artifacts prune` privileged, **Then** the command reports that nothing needed pruning, deletes nothing, and exits successfully.
2. **Given** one unreferenced artifact cannot be deleted while others can, **When** the command finishes, **Then** all other unreferenced artifacts are still deleted, no referenced artifact is modified, the output shows the removed set with the freed space so far plus each failed artifact with its cause and a repair-and-retry next step, and the exit status is nonzero.
3. **Given** a failed prune, **When** the operator fixes the cause and runs `microvm artifacts prune` again, **Then** the previously failed artifact is reclaimed and the retry reports success.
4. **Given** the local inventory is unavailable (missing base directory, unreadable database, locked state), **When** the operator runs `microvm artifacts prune`, **Then** the command reports what happened, why nothing was pruned, and what the operator can do next, with a nonzero exit status.
5. **Given** artifacts were skipped because of an actively in-progress transfer, **When** the command reports its result, **Then** the skipped artifacts appear as a separate group from removed and failed ones, so the operator can tell "kept busy" from "kept referenced".

---

### Edge Cases

- The inventory is empty or every artifact is referenced; the command reports "nothing to prune" with success status instead of an empty removal list and deletes nothing. No confirmation is shown in this case because there is nothing to delete.
- The operator is unprivileged and declines elevation or interrupts; the command reports cancellation with no deletions and no side effects.
- The command runs unprivileged with `--non-interactive`; it fails fast with rerun-with-root guidance and never prompts.
- The local base directory is missing or unwritable, or the inventory database is unreadable or locked; the command surfaces the typed failure calmly with the affected path and a next step.
- An artifact recorded in the inventory has no file on disk; the command drops the stale record, counts it as reclaimed with zero freed bytes, and does not treat it as a failure.
- A file sits in the artifact directories but has no inventory record; the command leaves it untouched.
- One deletion fails midway (permissions, lock, I/O error); the command continues with the remaining candidates and reports removed, skipped, and failed groups separately.
- An unreferenced artifact has an actively in-progress transfer while prune runs; the command keeps it fully intact and reports it in the skipped group.
- Repeated runs with no changes between them: every run after the first reports nothing to prune.
- The terminal has no color support, is narrow, or is non-interactive; removed, skipped, and failed groups remain distinguishable through text labels and symbols, and scripted output stays deterministic.
- The operator declines the deletion preview or interrupts the confirmation; the command reports cancellation with no deletions and no side effects.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The CLI MUST expose the command as `microvm artifacts prune` beside the existing `microvm artifacts download` entry, reusing the existing `artifacts` command group; no other spelling or alias is part of this feature.
- **FR-002**: The command MUST take no positional arguments and no selection flags; the only accepted flag MUST be the established `--non-interactive` flag with the same meaning as the sibling commands (no prompts of any kind, root required up front, deterministic output).
- **FR-003**: The command MUST delegate all candidacy, reference, deletion, accounting, and retry-safety behavior to the existing SDK prune operation; it MUST NOT duplicate registry, inventory, checksum, or lifecycle logic, and MUST NOT create, start, stop, or configure a MicroVM.
- **FR-004**: On success the command MUST report the removed kernels, the removed images, the count removed per kind, and the freed space per kind and in total, rendered in human-readable units, and exit with a success status.
- **FR-005**: When nothing is unreferenced, the command MUST report that nothing needed pruning with a success exit status and MUST NOT show an empty removal list as a failure.
- **FR-006**: When one or more deletions fail, the command MUST still show the removed set with the freed space so far, MUST show each failed artifact with its cause plus a repair-and-retry next step, and MUST exit with a nonzero status.
- **FR-007**: Artifacts skipped because of an actively in-progress transfer MUST appear as a group separate from removed and failed ones.
- **FR-008**: When not already running with root access, the command MUST request elevation through the existing CLI privilege flow with the same cases and behavior as the sibling commands: prompt-driven escalation in interactive terminals before any deletion attempt, hard error with rerun guidance in non-interactive or non-terminal contexts. The command MUST NOT implement its own escalation mechanism.
- **FR-009**: The command MUST support the established non-interactive mode: no prompts of any kind, root required up front, deterministic output. Elevation prompting MUST NOT occur in non-interactive mode.
- **FR-010**: Expected failures (missing/unreadable inventory, database or base-directory failure, prune-operation failure) MUST explain what happened, why nothing (or only part) was pruned, and what the operator can do next, with a nonzero exit status and no unexplained output.
- **FR-011**: State MUST be communicated with explicit text labels and MUST NOT be communicated by color alone. The presentation MUST follow the restrained neutral visual language of the established flows: compact information density, clear hierarchy, semantic status emphasis, immediate feedback, and calm failure states. The command MUST NOT use pagers or interactive selectors. In interactive mode the command MUST show a preview listing every kernel and image identity to delete plus the estimated freed space, and require one explicit confirmation before deleting (matching the review-then-confirm pattern of the sibling flows); declining or interrupting the confirmation MUST cancel with no deletions. In non-interactive mode the command MUST delete without prompting after the root gate.
- **FR-012**: The command MUST resolve the host-local base directory according to the existing CLI policy and pass that explicit path to the SDK; it MUST NOT create a second home-directory or artifact-storage policy.
- **FR-013**: The command MUST NOT change existing SDK behavior, lifecycle semantics, error contracts, or persistence layout; any SDK change is allowed only as a strictly minimal additive extension, and none is expected.

### Key Entities

- **Prune Run**: One `microvm artifacts prune` invocation — elevation gate, a single SDK prune call, and a rendered result with removed, skipped, and failed groups.
- **Removed Set**: The reclaimed kernels and images shown on success: identities per kind, counts per kind, and freed space per kind and in total.
- **Failed Set**: The candidates that could not be deleted: each artifact identity with its cause, shown with the removed set so far and a repair-and-retry next step.
- **Skipped Set**: The unreferenced artifacts kept intact because of an actively in-progress transfer, shown as a group separate from removed and failed.
- **Nothing-to-prune**: The state where no kernel or image is unreferenced; rendered as a calm report with success status, not an error.
- **Elevation Gate**: The existing privilege flow reused unchanged: interactive prompt-driven escalation or non-interactive hard error, securing root before any deletion attempt.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In usability tests with a mixed inventory (referenced plus unreferenced kernels and images), an operator can run `microvm artifacts prune` and identify what was removed and how much space was freed from the single rendered result in under 30 seconds without running any other command.
- **SC-002**: In 100% of mixed-ownership tests, the command deletes every unreferenced kernel and image, leaves every referenced one intact, and the reported removed set plus freed space match the actual disk and inventory change.
- **SC-003**: In 100% of unprivileged interactive tests, the command prompts for elevation through the existing flow before deleting and completes the prune after approval; in 100% of declined-elevation tests it reports cancellation with no deletions and no host changes.
- **SC-004**: In 100% of unprivileged non-interactive tests, the command fails before any deletion attempt with rerun guidance and performs no prompts.
- **SC-005**: In 100% of nothing-to-prune tests (fully referenced or empty inventory), the command reports nothing to prune with a success exit status and changes nothing on disk or in inventory.
- **SC-006**: In 100% of single-undeletable-file tests, the command still reclaims every other unreferenced artifact, touches no referenced artifact, shows the removed set with freed space so far plus the named failure with a repair step, and exits nonzero.
- **SC-007**: In 100% of regression checks, previously available behaviors (existing `artifacts download` flow, creation, start, stop, `ssh`, `ls` including their privilege behavior, existing SDK prune results) behave exactly as before; the only new observable capability is the `artifacts prune` command.

## Assumptions

- The existing SDK prune operation (existence-based references, orphan and stale handling, active-transfer skip list, partial-failure payload with removed plus causes, idempotent no-op) exists unchanged and is the sole deletion engine; the CLI adds presentation and elevation only.
- Prune is destructive, so interactive mode shows a preview of the reclaimed set and requires one explicit confirmation before deleting (declining cancels with no deletions); in non-interactive mode the command deletes without prompting after the root gate. No per-artifact selection is offered.
- Freed space renders in human-readable units converted from the byte counters the SDK returns; the CLI performs no independent size measurement.
- The command takes no positional arguments and no per-artifact selection; selective pruning or dry-run previews are out of scope.
- All operator-facing text for this feature is English; localization is out of scope.
