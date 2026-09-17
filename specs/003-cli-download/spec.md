# Feature Specification: CLI Artifact Download

**Feature Branch**: `003-cli-download`

**Created**: 2026-09-17

**Status**: Draft

**Input**: User description: "Add a CLI `download` command that lets an operator select one or
more distributions, choose one compatible kernel for each distribution, and then downloads the
required Firecracker/firectl binaries plus the selected distribution and kernel artifacts through
the existing SDK. The command should be intelligent, modern, and follow the Taumaru Design System."

## Clarifications

### Session 2026-09-17

- Q: When a selected distribution's kernel download fails, should the command skip that distribution's images and continue with other distributions? → A: Yes. Skip only the affected distribution's images, continue the other distributions, and report at the end which groups succeeded and which failed.
- Q: How should the command behave when the operator presses `Ctrl-C` during a transfer? → A: Cancel in a controlled way, clean up the partial file, preserve verified artifacts, return exit code `130`, and summarize successful and failed groups.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Build a Download Plan (Priority: P1)

As a host operator, I want to choose one or more compatible distributions and one kernel for each
so that I can prepare the exact MicroVM artifacts I intend to use without typing registry IDs.

**Why this priority**: Selecting a complete artifact plan is the primary user value of the
command. It prevents incompatible distribution/kernel combinations and makes a multi-distribution
host setup efficient.

**Independent Test**: Run the command against a deterministic registry fixture, use only keyboard
input to select at least two distributions, choose a kernel for each, and verify that a review step
shows the complete plan before any transfer begins.

**Acceptance Scenarios**:

1. **Given** a registry with multiple host-compatible distributions, **When** the operator starts
   `microvm download`, **Then** the command presents a keyboard-friendly multi-select list with
   each distribution's name, version, and architecture and allows one or more selections.
2. **Given** one or more distributions are selected, **When** the operator continues, **Then** the
   command presents a separate kernel choice for each distribution using only its published
   compatible kernels and clearly marks the registry default.
3. **Given** a complete distribution/kernel selection, **When** the operator reaches the review
   step, **Then** the command shows every selected distribution, chosen kernel, required runtime
   binary bundle, image count, and estimated artifact size before asking for confirmation.
4. **Given** the operator cancels before confirmation, **When** the command exits, **Then** no
   artifact transfer is started and the command reports that the operation was cancelled.

---

### User Story 2 - Download the Planned Artifacts (Priority: P1)

As a host operator, I want the confirmed plan to acquire the required runtime binaries, kernels,
and distribution images in a predictable order so that the host is ready for later MicroVM
operations.

**Why this priority**: A plan has no operational value until all selected artifacts are available
and verified locally.

**Independent Test**: Confirm a fixture plan containing multiple distributions, including two that
share a kernel, and verify that all selected runtime packages are handled first, each unique kernel
is handled once, and every selected distribution image is acquired before success is reported.

**Acceptance Scenarios**:

1. **Given** a confirmed plan and compatible runtime packages that collectively provide Firecracker
   and firectl, **When** downloading starts, **Then** the command acquires every required runtime
   package first and only proceeds to distribution and kernel artifacts after they all succeed.
2. **Given** two selected distributions use the same kernel, **When** the plan is executed, **Then**
   that kernel is requested once and is reused for both distribution selections.
3. **Given** a selected distribution has multiple published images, **When** its download runs,
   **Then** every image belonging to that distribution is acquired and represented in the final
   result.
4. **Given** all required members are successfully verified, **When** the command finishes, **Then**
   it reports success and summarizes the acquired or reused runtime binaries, kernels, and
   distribution images.

---

### User Story 3 - Reuse Artifacts and Report Progress (Priority: P1)

As a host operator, I want repeated downloads to be safe and visible so that I can retry an
interrupted preparation without wasting transfers or wondering whether the command is still
working.

**Why this priority**: Artifact acquisition can be slow and may be retried. Trustworthy progress
and idempotent reuse are required for an infrastructure command.

**Independent Test**: Execute the same confirmed plan twice, alter one cached file between runs,
and inspect the displayed statuses and transfer counts for skipped, adopted, replaced, and failed
members.

**Acceptance Scenarios**:

1. **Given** a previously verified artifact is still valid, **When** the same plan is run again,
   **Then** the command reuses it without a second transfer and labels it as already available.
2. **Given** a cached artifact is missing or has incorrect contents, **When** the same plan is run,
   **Then** the command delegates reconciliation to the existing artifact behavior, replaces the
   invalid file when possible, and does not report it ready before verification.
3. **Given** an artifact transfer is in progress, **When** bytes arrive, **Then** the command
   immediately shows the artifact identity, current stage, received bytes, expected total, and
   aggregate progress without inventing progress values.
4. **Given** a cached file is adopted or skipped, **When** its operation completes, **Then** the
   command shows that outcome distinctly from a fresh transfer while keeping the overall plan
   understandable.

---

### User Story 4 - Recover Calmly from Failure (Priority: P2)

As a host operator, I want a failed preparation to explain what happened and what remains usable
so that I can correct the problem and retry without losing successful work.

**Why this priority**: Registry and filesystem failures are expected operational conditions. A
calm, actionable result is safer than a misleading success or an opaque terminal error.

**Independent Test**: Run the command with a fixture that fails one binary or artifact transfer,
then verify the exit status, failure summary, retained successful artifacts, and retry guidance.

**Acceptance Scenarios**:

1. **Given** any required runtime binary package cannot be acquired, **When** the command starts
   the transfer phase, **Then** it reports the failure and stops before starting distribution or
   kernel downloads.
2. **Given** one selected distribution or its kernel fails after another selection succeeds,
   **When** the command finishes, **Then** it skips only the affected distribution's remaining
   image downloads, continues independent selections, reports a non-success result listing the
   successful and failed groups separately, preserves the verified successful artifacts, and
   explains that retrying is safe.
3. **Given** the registry is unavailable, malformed, or incompatible, **When** the command needs
   registry data, **Then** it exits with a non-success status and explains the cause and the next
   action without a panic or an unexplained stack trace.
4. **Given** the command is invoked without an interactive terminal and without complete explicit
   selections, **When** it starts, **Then** it exits before downloading and explains how to provide
   selections for automation.

---

### User Story 5 - Automate Explicit Selections (Priority: P2)

As an automation operator, I want to provide distribution and kernel identifiers explicitly so that
I can use the same command without an interactive selector.

**Why this priority**: The CLI is an operational tool and must remain usable in scripts while the
interactive flow remains the default for people at a terminal.

**Independent Test**: Invoke the command in a non-interactive environment with explicit distribution
IDs and one kernel mapping per distribution, then verify that it skips the selector, follows the
same download plan, and returns a deterministic success or failure status.

**Acceptance Scenarios**:

1. **Given** valid explicit distribution IDs and one compatible kernel ID for each, **When** the
   command runs without a terminal, **Then** it creates the same plan that the interactive flow
   would create and proceeds without prompts.
2. **Given** an explicit mapping names an unavailable or incompatible kernel, **When** the command
   validates the plan, **Then** it reports the invalid mapping and starts no transfer.
3. **Given** explicit selections complete successfully, **When** the command exits, **Then** its
   status and concise output are deterministic and suitable for a script to consume.

### Edge Cases

- The registry returns no host-compatible distributions, no compatible kernels for a selected
  distribution, or no host-compatible binary packages for one of the required runtime components.
- The registry returns duplicate identifiers, incomplete metadata, an unavailable response, or an
  unsupported schema; the command must fail before presenting or downloading unsafe selections.
- The operator selects no distributions, selects a distribution twice, presses escape during a
  selector, or interrupts the command during transfer; transfer interruption must clean up the
  partial file, preserve verified artifacts, and produce a cancellation summary.
- Several selected distributions share a kernel; the plan must deduplicate the kernel transfer
  without losing each distribution's selected relationship.
- If a selected kernel fails, the command must not download images for its associated distribution;
  it must continue other independent selections and summarize successful and failed groups at the
  end.
- The selected distribution contains multiple images, and one image fails while other images or
  other selections have already completed.
- A valid artifact already exists, is present but not recorded, is corrupted, or is missing; the
  command must expose the existing SDK reuse/replacement outcome rather than inventing its own
  cache rule.
- The terminal does not support color, has limited width, or cannot receive interactive input;
  labels and progress information must remain understandable without color or a full-screen UI.
- The home directory is unavailable or cannot be written; the command must explain the affected
  local path and stop safely.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The CLI MUST expose a `download` command from the existing `microvm` entrypoint.
- **FR-002**: The command MUST resolve the host-local base directory according to the existing
  CLI policy and pass that explicit path to the artifact-management capability; it MUST NOT create a
  second home-directory or artifact-storage policy.
- **FR-003**: Before showing selections, the command MUST obtain the current published collections
  for distributions, kernels, and binary packages and MUST reject invalid or unavailable registry
  data with an actionable failure.
- **FR-004**: The interactive flow MUST provide a multi-select distribution step that allows one
  or more host-compatible distributions, visibly identifies the focused and selected rows, and
  supports keyboard navigation, selection, confirmation, and cancellation.
- **FR-005**: The command MUST NOT begin an artifact transfer until the operator confirms a complete
  plan; an empty selection or cancellation MUST exit without downloading.
- **FR-006**: For every selected distribution, the command MUST require exactly one kernel from
  that distribution's published compatible-kernel set, mark the registry default, and reject an
  incompatible or unavailable explicit choice before transfer.
- **FR-007**: The review step MUST show the selected distribution/kernel pairs, the automatically
  chosen host-compatible runtime binary packages, the number of distribution images, and the
  estimated total size before confirmation.
- **FR-008**: The command MUST automatically choose the highest published semantic-version binary
  package for each required runtime component on the host architecture. A package containing both
  components MUST be reused for both requirements; otherwise, one package per component MUST be
  selected. If the registry cannot provide every required component, it MUST fail before
  downloading any distribution or kernel artifact.
- **FR-009**: After confirmation, the command MUST acquire all selected runtime packages first,
  then acquire each unique selected kernel and every image belonging to each selected distribution
  whose selected kernel was verified, in a deterministic order; if a selected kernel fails, FR-014
  governs the associated distribution's image group.
- **FR-010**: The command MUST call the existing artifact-management operations for all acquisition,
  integrity, cache, persistence, and compatibility behavior; it MUST NOT duplicate registry,
  download, checksum, or local-inventory logic in the CLI.
- **FR-011**: The command MUST forward live download events into a compact progress presentation
  that identifies the current member, stage, current bytes, expected bytes, aggregate progress,
  and whether a member was downloaded, adopted, or skipped.
- **FR-012**: Progress MUST be truthful and readable in color and non-color terminals; operational
  state MUST also be communicated with text labels or symbols and MUST NOT rely on color alone.
- **FR-013**: The command MUST report success only after every required runtime package, selected
  kernel, and selected distribution image has returned a verified result from the artifact
  capability.
- **FR-014**: If a selected kernel fails, the command MUST skip image downloads for the associated
  distribution, continue other independent selections when all runtime packages succeeded, retain
  successful verified members for safe retry, and report successful and failed groups separately;
  a runtime-package failure MUST stop all dependent downloads.
- **FR-015**: Expected failures MUST explain what happened, why the requested preparation is
  incomplete, and what the operator can do next; the command MUST use nonzero exit status for an
  incomplete plan and MUST NOT expose an unexplained panic or stack trace.
- **FR-016**: The command MUST support non-interactive use through explicit repeatable distribution
  selections and one distribution-to-kernel mapping per selected distribution, using the same
  validation and acquisition behavior as the interactive flow.
- **FR-017**: The interactive presentation MUST follow the Taumaru Design System's restrained
  neutral visual language: compact information density, clear hierarchy, semantic status emphasis,
  visible keyboard focus, progressive disclosure, immediate feedback, and calm failure states.
- **FR-018**: The command MUST keep selection, review, progress, and result presentation separate
  from host lifecycle behavior; it MUST not create, start, stop, or configure a MicroVM as part of
  this feature.
- **FR-019**: If the operator presses `Ctrl-C` during a transfer, the command MUST cancel the
  in-flight operation in a controlled way and must not start subsequent plan members, ensure that
  the partial file is removed and never published as verified, preserve already verified artifacts,
  report successful and failed groups plus the cancellation, and return exit code `130`.

### Key Entities *(include if feature involves data)*

- **Download Plan**: The confirmed set of runtime binary packages, unique kernels, and distribution
  image groups to acquire, including total expected size and deterministic execution order.
- **Distribution Selection**: A published distribution chosen by the operator, including its
  selected compatible kernel and all images that belong to the distribution.
- **Kernel Selection**: The one published kernel associated with a distribution selection; the same
  kernel may be shared by multiple distribution selections and downloaded once per plan.
- **Runtime Binary Packages**: The host-compatible published packages that provide the Firecracker
  and firectl components required by the host-local MicroVM workflow. The registry may publish one
  package for both components or separate packages for each component.
- **Progress Event**: The current member, stage, byte counters, expected total, and cache/download
  disposition displayed while the plan executes.
- **Download Outcome**: The final success, cancellation, partial failure, or validation failure
  reported by the command, with enough detail for a retry; transfer cancellation includes the
  preserved verified groups and any group interrupted before verification.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In usability tests with at least three available distributions, an operator can
  select two distributions and one kernel for each using only the keyboard and reach confirmation
  in under 60 seconds without selecting an incompatible kernel.
- **SC-002**: In 100% of repeated-download tests for an unchanged plan, every valid locally cached
  member is reused without a second transfer, while a missing or corrupted member is repaired by
  the existing artifact behavior.
- **SC-003**: After confirmation, the command shows an initial running state within one second and
  continues to update visible member or aggregate progress until each transfer reaches a terminal
  state.
- **SC-004**: In 100% of successful plan tests, the command reports success only after the runtime
  binary bundle, each unique selected kernel, and every image for every selected distribution has
  been verified.
- **SC-005**: In 100% of failure-path tests, the command returns a nonzero status, identifies the
  failed selection or stage, separates successful and failed groups in the final summary, explains
  the impact, and gives a concrete retry or repair action.
- **SC-006**: In 100% of non-color and narrow-terminal tests, selection focus, selected state,
  progress stage, and failure/success status remain distinguishable through text or symbols.
- **SC-007**: In 100% of non-interactive tests with valid explicit selections, the command performs
  no prompts and produces a deterministic exit status and summary suitable for automation.
- **SC-008**: A plan containing shared kernels downloads each unique kernel at most once while
  retaining the correct distribution-to-kernel choice for every selected distribution.
- **SC-009**: In 100% of transfer-cancellation tests, the command returns exit code `130`, publishes
  no partial artifact, preserves all previously verified groups, and identifies the cancelled,
  successful, and failed groups in its final summary.

## Assumptions

- The existing SDK already exposes the authoritative registry listing, download, progress, cache,
  integrity, compatibility, and local-inventory behavior required by this command.
- The CLI uses the existing host-home policy: `TAUMARU_HOME` selects a custom base directory when
  set, otherwise the default is `~/.taumaru-microvm`; the SDK receives the resolved path explicitly.
- Host-compatible means the architecture supported by the current host runtime. Cross-architecture
  prefetching is out of scope for this first CLI flow.
- When multiple compatible runtime binary packages are published, the highest published version
  containing the required Firecracker and firectl components is selected automatically; choosing a
  binary version interactively is out of scope.
- Selecting a distribution downloads every image published for that distribution because the
  existing SDK operation treats the distribution as an image set.
- Shared kernels are deduplicated within one command execution, and the SDK remains responsible for
  deciding whether a previously verified file is downloaded, adopted, or skipped.
- Authentication, custom registry selection, and resumable transfers remain outside this feature.
  Controlled transfer cancellation, cleanup of the partial artifact, and preservation of verified
  groups are in scope. VM creation/startup and new lifecycle commands remain outside this feature.
- The supplied Taumaru Design System is a visual and interaction reference for this CLI feature;
  it does not add unrelated web components, branding assets, or product workflows to the scope.
