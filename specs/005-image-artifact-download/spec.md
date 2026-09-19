# Feature Specification: Image Artifact Download

**Feature Branch**: `005-image-artifact-download`

**Created**: 2026-09-19

**Status**: Draft

**Input**: User description: "Refactor the SDK to also offer distribution download per image, not only per distribution, and change the download flow so the operator selects the distribution image they want instead of the whole distribution. The operator can select more than one image per distribution from a single image list where each entry names the distribution it belongs to. There is no custom kernel option; the downloaded kernel is always the distribution default that creation uses. Rename the `download` command to `artifacts download`. In short: a single image select."

## Clarifications

### Session 2026-09-19

- Q: How should operators identify images when scripting the command without a terminal? → A: Repeatable `--image DISTRIBUTION=IMAGE`; remove `--distribution` and `--kernel` flags entirely.
- Q: How should the command handle the same image being selected more than once in one plan? → A: Deduplicate repeated image selections and transfer each unique image once.
- Q: When images are chosen across several distributions, what order should review and progress follow? → A: Sort images by distribution name, then image name, for review and transfer.


## User Scenarios & Testing *(mandatory)*

### User Story 1 - Pick Images From One List (Priority: P1)

As a host operator, I want a single selection list of distribution images where each entry shows which distribution it belongs to, so that I can prepare exactly the images I intend to use without choosing a distribution first and without choosing a kernel.

**Why this priority**: Direct image selection is the core user value of this change. It removes the confusing kernel prompt and stops forcing whole-distribution downloads.

**Independent Test**: Point the command at a deterministic registry fixture with several distributions that each publish several images, use only keyboard input to select images from more than one distribution (including two images from the same distribution), and verify that a review step shows only the selected images before any transfer begins.

**Acceptance Scenarios**:

1. **Given** a registry with host-compatible images across several distributions, **When** the operator starts `microvm artifacts download`, **Then** the command presents one keyboard-friendly multi-select image list where every entry identifies its parent distribution and allows one or more selections.
2. **Given** the image list is shown, **When** the operator browses it, **Then** each entry exposes enough identity to decide (image name, parent distribution, variant or capabilities, and size) with no kernel choice at any point.
3. **Given** a complete image selection, **When** the operator reaches the review step, **Then** the command shows every selected image with its parent distribution, the automatically resolved default kernel per affected distribution, the required runtime bundle, and the estimated total size before asking for confirmation.
4. **Given** the operator cancels before confirmation or selects nothing, **When** the command exits, **Then** no artifact transfer starts and the command reports that the operation was cancelled.

---

### User Story 2 - Download Only Selected Images With Default Kernels (Priority: P1)

As a host operator, I want the confirmed plan to fetch only my selected images plus each affected distribution's default kernel, so that I stop paying bandwidth and disk for images I never asked for.

**Why this priority**: Selective transfer is the operational payoff. A selection the operator trusts must translate into exactly that set of local artifacts.

**Independent Test**: Confirm a fixture plan containing two images from one distribution and one image from another distribution whose distributions share nothing except possibly a kernel, then verify that every selected image is acquired and verified, no unselected image of any affected distribution is stored, and each unique default kernel is requested at most once.

**Acceptance Scenarios**:

1. **Given** a confirmed plan, **When** downloading starts, **Then** the command acquires every required runtime package first and only proceeds to kernel and image artifacts after they all succeed.
2. **Given** selected images whose distributions resolve to the same default kernel, **When** the plan executes, **Then** that kernel is requested once and reused for every dependent image.
3. **Given** a confirmed plan covering several images, **When** the plan finishes successfully, **Then** each selected image is verified locally and the command reports success summarizing runtime packages, default kernels, and the selected images.
4. **Given** a successful plan, **When** the operator later creates a MicroVM from a downloaded image, **Then** the kernel already present is the one creation resolves, with no extra kernel step required.

---

### User Story 3 - Reuse Artifacts and Report Progress (Priority: P1)

As a host operator, I want repeated runs to be safe and visible so that I can retry an interrupted preparation without wasting transfers or wondering whether the command is still working.

**Why this priority**: Image transfers can be large and retried. Trustworthy per-image progress and idempotent reuse are required for an infrastructure command.

**Independent Test**: Execute the same confirmed image plan twice, alter one cached image file between runs, and inspect the displayed statuses and transfer counts for reused, repaired, and failed members.

**Acceptance Scenarios**:

1. **Given** a previously verified image or kernel is still valid, **When** the same plan runs again, **Then** the command reuses it without a second transfer and labels it as already available.
2. **Given** a cached image is missing or has incorrect contents, **When** the same plan runs, **Then** the command delegates reconciliation to the existing artifact behavior, repairs the file when possible, and never reports it ready before verification.
3. **Given** an image transfer is in progress, **When** bytes arrive, **Then** the command immediately shows the image identity (distribution and image), current stage, received bytes, expected total, and aggregate progress without inventing progress values.
4. **Given** a cached file is reused, **When** its step completes, **Then** the command shows that outcome distinctly from a fresh transfer while keeping the overall plan understandable.

---

### User Story 4 - Recover Calmly From Failure (Priority: P2)

As a host operator, I want a failed preparation to explain what happened and what remains usable so that I can correct the problem and retry without losing successful work.

**Why this priority**: Registry and filesystem failures are expected operational conditions. A calm, actionable result is safer than a misleading success or an opaque terminal error.

**Independent Test**: Run the command with a fixture that fails one image or kernel transfer, then verify the exit status, failure summary, retained successful artifacts, and retry guidance.

**Acceptance Scenarios**:

1. **Given** any required runtime package cannot be acquired, **When** the command starts the transfer phase, **Then** it reports the failure and stops before starting kernel or image downloads.
2. **Given** one selected image or its default kernel fails after another selection succeeds, **When** the command finishes, **Then** it continues independent selections, preserves the verified successful artifacts, and reports successful and failed groups separately with safe-retry guidance.
3. **Given** the registry is unavailable, malformed, or incompatible, **When** the command needs registry data, **Then** it exits with a non-success status and explains the cause and the next action without an unexplained failure.
4. **Given** the operator presses interrupt during a transfer, **When** the command stops, **Then** it cancels in a controlled way, removes the partial file without publishing it as verified, preserves already verified artifacts, summarizes successful, failed, and cancelled groups, and returns exit code `130`.

---

### User Story 5 - Automate Explicit Image Selections (Priority: P2)

As an automation operator, I want to provide image selections explicitly so that I can use the same command without an interactive selector.

**Why this priority**: The command is an operational tool and must remain usable in scripts while the interactive flow stays the default for people at a terminal.
**Acceptance Scenarios**:

1. **Given** valid explicit image selections in the form `--image DISTRIBUTION=IMAGE` (repeatable, including more than one image of the same distribution), **When** the command runs without a terminal, **Then** it builds the same plan the interactive flow would build and proceeds without prompts.
2. **Given** an explicit selection names an unavailable image, an image outside its named distribution, or a host-incompatible image, **When** the command validates the plan, **Then** it reports the invalid selection and starts no transfer.
3. **Given** the operator uses a removed `--distribution` or `--kernel` flag, **When** the command parses arguments, **Then** it rejects the invocation and points to the `--image DISTRIBUTION=IMAGE` form.
4. **Given** explicit selections complete successfully, **When** the command exits, **Then** its status and concise output are deterministic and suitable for a script to consume.

---

### Edge Cases

- The registry returns no host-compatible images, or a selected image's distribution has no host-compatible default kernel.
- The operator selects the same image twice; the plan deduplicates it to one entry transferred once.
- The operator selects no images, presses escape during the selector, or interrupts the command during transfer; interruption must clean up the partial file, preserve verified artifacts, and produce a cancellation summary.
- Several selected images resolve to the same default kernel; the plan must fetch that kernel once without losing any image's dependency on it.
- One selected image fails while other images or other selections already completed; independent selections continue and the summary separates outcomes.
- A valid artifact already exists, is present but not recorded, is corrupted, or is missing; the command must expose the existing reuse and repair outcome rather than inventing its own cache rule.
- The previous command name is invoked; the command reports the renamed entry and how to reach the same flow.
- The terminal does not support color, has limited width, or cannot receive interactive input; labels and progress remain understandable without color or a full-screen UI.
- The home directory is unavailable or cannot be written; the command explains the affected local path and stops safely.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The command MUST be reachable as `microvm artifacts download`; the previous top-level download entry MUST be replaced by this path and its help MUST describe image-based preparation.
- **FR-002**: The command MUST resolve the host-local base directory according to the existing CLI policy and pass that explicit path to the artifact-management capability; it MUST NOT create a second home-directory or artifact-storage policy.
- **FR-003**: Before showing selections, the command MUST obtain the current published distributions, kernels, images, and binary packages and MUST reject invalid or unavailable registry data with an actionable failure.
- **FR-004**: The interactive flow MUST present exactly one image selection step listing host-compatible images across all host-compatible distributions, with each entry identifying its parent distribution; the flow MUST NOT present a distribution-first step and MUST NOT present any kernel choice. The step MUST allow one or more images, including several images from the same distribution, with keyboard navigation, selection, confirmation, and cancellation; an empty selection or cancellation MUST exit without downloading. Selecting the same image more than once MUST deduplicate to one entry transferred once.
- **FR-005**: The command MUST NOT begin an artifact transfer until the operator confirms a complete plan.
- **FR-006**: For every selected image, the command MUST resolve that image's distribution default kernel, verify host-architecture compatibility of the image, its distribution, and the resolved kernel, and reject an incompatible selection before transfer; there MUST be no operator override for a different kernel.
- **FR-007**: The review step MUST show every selected image, ordered by distribution name then image name, with its parent distribution, the resolved default kernel per affected distribution, the automatically chosen host-compatible runtime bundle, and the estimated total size of only the selected artifacts before confirmation.
- **FR-008**: The reusable artifact capability MUST offer single-image acquisition in addition to the existing whole-distribution acquisition; the existing whole-distribution behavior MUST keep working for its current consumers.
- **FR-009**: After confirmation, the command MUST acquire all required runtime packages first, then each unique resolved default kernel and each selected image; images MUST be ordered by distribution name then image name for progress and transfer, independent of selection order; a runtime-package failure MUST stop all dependent downloads.
- **FR-010**: If a resolved default kernel fails, the command MUST skip only the selected images depending on it, continue independent selections when all runtime packages succeeded, retain verified members for safe retry, and report successful and failed groups separately.
- **FR-011**: The command MUST delegate all acquisition, integrity, cache, persistence, and compatibility behavior to the existing artifact-management capability; it MUST NOT duplicate registry, transfer, checksum, or local-inventory logic.
- **FR-012**: The command MUST forward live transfer events into a compact progress presentation that identifies the current member (including distribution and image for image members), stage, current bytes, expected bytes, aggregate progress, and whether a member was transferred, reused, or repaired.
- **FR-013**: Progress MUST be truthful and readable in color and non-color terminals; operational state MUST also be communicated with text labels or symbols and MUST NOT rely on color alone.
- **FR-014**: The command MUST report success only after every required runtime package, resolved default kernel, and selected image has returned a verified result from the artifact capability; unselected images of an affected distribution MUST NOT be required and MUST NOT be stored as part of the plan.
- **FR-015**: The command MUST support non-interactive use through explicit repeatable image selections using the same validation and acquisition behavior as the interactive flow. Explicit selection takes the repeatable form `--image DISTRIBUTION=IMAGE`; the previous `--distribution` and `--kernel` flags MUST be removed and their use MUST be rejected with guidance toward the image form.
- **FR-016**: The interactive presentation MUST follow the restrained neutral visual language: compact information density, clear hierarchy, semantic status emphasis, visible keyboard focus, progressive disclosure, immediate feedback, and calm failure states.
- **FR-017**: The command MUST keep selection, review, progress, and result presentation separate from host lifecycle behavior; it MUST not create, start, stop, or configure a MicroVM as part of this feature.
- **FR-018**: If the operator interrupts during a transfer, the command MUST cancel the in-flight operation in a controlled way, start no subsequent plan members, remove the partial file without publishing it as verified, preserve already verified artifacts, report successful, failed, and cancelled groups, and return exit code `130`.

### Key Entities *(include if feature involves data)*

- **Image Selection**: One operator-chosen distribution image, identified by its image identity together with its parent distribution; several selections may share a distribution.
- **Download Plan**: The confirmed set of runtime packages, unique resolved default kernels, and selected images to acquire, including total expected size and deterministic execution order.
- **Default Kernel Resolution**: The mapping from each affected distribution to its published default kernel, validated for host compatibility with no operator override.
- **Runtime Binary Bundle**: The host-compatible published packages providing the required runtime components, chosen automatically by the existing highest-version policy.
- **Progress Event**: The current member, stage, byte counters, expected total, and reuse or transfer disposition displayed while the plan executes.
- **Download Outcome**: The final success, cancellation, partial failure, or validation failure reported by the command, with enough detail for a retry.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In usability tests with at least two distributions publishing at least two images each, an operator can select images from more than one distribution using only the keyboard and reach confirmation in under 60 seconds with no kernel decision.
- **SC-002**: In 100% of successful plan tests, only the selected images are stored and verified; no unselected image of any affected distribution appears locally as part of the plan.
- **SC-003**: In 100% of plans where several selected images share a default kernel, each unique kernel is transferred at most once while every dependent image is verified.
- **SC-004**: In 100% of repeated-download tests for an unchanged plan, every valid locally cached member is reused without a second transfer, while a missing or corrupted member is repaired by the existing artifact behavior.
- **SC-005**: After confirmation, the command shows a running state within one second and keeps updating visible member or aggregate progress until each transfer reaches a terminal state.
- **SC-006**: In 100% of successful plan tests, the command reports success only after the runtime bundle, each unique resolved default kernel, and every selected image has been verified.
- **SC-007**: In 100% of failure-path tests, the command returns a nonzero status, identifies the failed selection or stage, separates successful and failed groups, explains the impact, and gives a concrete retry or repair action.
- **SC-008**: In 100% of non-color and narrow-terminal tests, selection focus, selected state, progress stage, and failure or success status remain distinguishable through text or symbols.
- **SC-009**: In 100% of non-interactive tests with valid explicit image selections, the command performs no prompts and produces a deterministic exit status and summary suitable for automation.
- **SC-010**: In 100% of transfer-interruption tests, the command returns exit code `130`, publishes no partial artifact, preserves all previously verified groups, and identifies the cancelled, successful, and failed groups.

## Assumptions

- The existing artifact capability already owns registry listing, transfer, progress, cache, integrity, compatibility, and local-inventory behavior; this feature adds single-image acquisition beside whole-distribution acquisition and reuses everything else.
- The resolved default kernel per distribution is the same kernel later creation resolves, so preparing it during download always leaves creation satisfiable for the downloaded image.
- Runtime bundle selection keeps the existing policy: highest published semantic version per required component on the host architecture, reusing one package when it provides both components.
- Renaming the entry to `artifacts download` replaces the previous top-level download entry; the change is a deliberate breaking CLI rename documented with migration guidance rather than a hidden alias.
- The CLI keeps the existing home policy: a custom base directory only through the environment, defaulting to the user home path otherwise, passed explicitly to the artifact capability.
- All operator-facing text for this feature is English; localization is out of scope.
