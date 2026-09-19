# Feature Specification: CLI `new` Command for Guided MicroVM Creation

**Feature Branch**: `007-microvm-new-command`

**Created**: 2026-09-19

**Status**: Draft

**Input**: User description: "Add a CLI `new` command that creates a MicroVM through the SDK creation operation. The command must be beautiful, clear, and interactive like the artifacts download flow, with a non-interactive mode as well. It accepts an optional positional name, prompts for any missing value (every prompt also accepted as a `--flag`), resolves home like the download command, offers a single-select image list showing per-image downloaded state, asks disk size in GB (minimum: registry image size), RAM as xMB/xGB converted to bytes (minimum: registry minimum), vCPU count (minimum: registry minimum), LAN exposure choice, and a final creation confirmation. After confirmation it provisions runtime binaries and the image plus its required default kernel with live per-step progress (reusing verified artifacts without fresh transfers, no download plan shown), then calls creation with live progress mapped from its events."

## Clarifications

### Session 2026-09-19

- Q: How should non-interactive mode identify the selected image? → A: Exactly one `--image DISTRIBUTION=IMAGE` argument, same form as `artifacts download`.
- Q: How should scripts explicitly request host-only networking when LAN exposure is optional? → A: Absent `--expose-lan` means host-only; no negative or value flag.
- Q: What should the interactive flow do when the operator types an invalid name, disk, memory, or vCPU value? → A: Abort immediately with the validation error; no re-prompt and no VM created.
- Q: Should help text and errors show the disk minimum and RAM minimum converted into the same units the operator types? → A: Yes, show disk minimum in GB and memory minimum in MB/GB matching the accepted input forms.
- Q: Should fractional memory amounts like 1.5GB be accepted, or only whole numbers? → A: Accept decimals for memory (e.g. 1.5GB), converting to bytes with rounding up.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Guided Interactive Creation (Priority: P1)

As a host operator, I want a guided `microvm new` flow that asks me for each creation value in turn so that I can create a correctly sized MicroVM without memorizing registry identifiers or byte counts.

**Why this priority**: The guided flow is the primary value of the command. It turns six technical decisions (name, image, disk, memory, vCPUs, network mode) into plain prompts with enforced minimums and a final confirmation.

**Independent Test**: Run the command in an interactive terminal against a deterministic registry fixture using only keyboard input: accept or type a name, pick exactly one image, enter disk, memory, and vCPU values, answer the network question, and confirm. Verify the confirmation summary matches the entered values before any transfer begins.

**Acceptance Scenarios**:

1. **Given** the operator starts `microvm new` with no arguments, **When** the flow begins, **Then** it prompts for the machine name first and accepts the typed name when it is valid.
2. **Given** the operator starts `microvm new "web-01"`, **When** the flow begins, **Then** it skips the name prompt and uses `web-01` directly.
3. **Given** a name is established, **When** the operator continues, **Then** the command presents a single-select image list where each entry identifies its parent distribution and visibly marks whether that image is already downloaded or still needs downloading, and exactly one image can be chosen.
4. **Given** an image is selected, **When** the operator continues, **Then** the command prompts for disk size in GB (converting to bytes, rejecting values below the image's registry size), memory as `xMB`/`xGB` (converting to bytes, rejecting values below the distribution minimum), vCPU count (rejecting values below the distribution minimum), and LAN exposure (yes or no, defaulting to no so the VM stays host-only unless the operator opts in), in that order.
5. **Given** some values were already supplied as arguments in an interactive terminal, **When** the flow runs, **Then** it prompts only for the remaining values and validates the supplied ones with the same rules before showing the review step.
6. **Given** all values are collected, **When** the operator reaches the review step, **Then** the command shows a creation summary with every chosen value plus the prerequisites that still need fetching and asks for explicit confirmation before any download or creation begins.
7. **Given** the operator declines or cancels at any prompt, **When** the command exits, **Then** no artifact transfer starts, no VM is created, and the command reports that the operation was cancelled.

---

### User Story 2 - Automatic Provisioning With Live Per-Step Progress (Priority: P1)
As a host operator, I want the confirmed command to fetch whatever prerequisites are missing and then create the VM while showing live progress per step so that I always know what is happening without reading a technical transfer plan.

**Why this priority**: Creation depends on runtime binaries, a kernel, and an image that may or may not be present. Silent provisioning hides failures; a plan table repeats download-command concerns. Per-step live rows are the right presentation for a creation flow.

**Independent Test**: Confirm a creation whose runtime bundle and image are missing, then confirm the same creation again with a warm cache. Verify the first run downloads prerequisites in dependency order with truthful byte progress and ends with a configured stopped VM, while the second run reuses verified artifacts without fresh transfers and still reports live creation progress.

**Acceptance Scenarios**:

1. **Given** a confirmed creation, **When** provisioning starts, **Then** the command acquires the required runtime binaries first and only proceeds to the kernel and image after they all succeed.
2. **Given** the selected image or its required default kernel is missing locally, **When** provisioning runs, **Then** the command fetches exactly that image and its distribution default kernel (no other images, no kernel choice) before creation.
3. **Given** a prerequisite is already verified locally, **When** provisioning reaches it, **Then** the command reuses it without a fresh transfer and labels the step as already available, distinctly from a fresh download.
4. **Given** provisioning and creation are running, **When** events arrive, **Then** the command shows one live row per step (each prerequisite member plus the creation stages) with the current item, stage, byte counters where applicable, and aggregate progress, without inventing progress values and without showing a download plan.
5. **Given** everything is ready locally, **When** creation runs, **Then** the command invokes the single creation operation with the collected values and maps its live progress events into the same per-step presentation until the VM reaches the configured and stopped state.

---

### User Story 3 - Non-Interactive Automation (Priority: P1)

As an automation operator, I want to provide every creation value explicitly so that scripts get the same provisioning and creation behavior with no prompts and deterministic output.

**Why this priority**: The CLI is an operational tool. Scripted creation must be possible, and missing values in scripts must fail fast with guidance instead of hanging on a prompt.

**Independent Test**: Invoke the command without a terminal with a complete explicit set (name, image, disk, memory, vCPUs, network choice), then repeat with one value omitted. Verify the first run provisions and creates with no prompts and deterministic output, while the second run errors before any transfer or mutation and explains the missing value.

**Acceptance Scenarios**:

1. **Given** a complete explicit set of values plus the non-interactive flag (name as positional or `--name`, exactly one `--image DISTRIBUTION=IMAGE`, `--disk-gb`, `--memory`, `--vcpus`, with `--expose-lan` present only for LAN exposure), **When** the command runs without a terminal, **Then** it performs no prompts, skips the interactive confirmation, provisions prerequisites, creates the VM, and exits with a deterministic status and concise summary.
2. **Given** any required value is missing in non-interactive mode, **When** the command validates input, **Then** it reports the missing value with usage guidance and starts no transfer and creates no VM.
3. **Given** an explicit value violates a minimum or format rule, **When** the command validates input, **Then** it reports the invalid value with the applicable minimum or expected format and starts no transfer.
4. **Given** the command runs without an interactive terminal and without a complete explicit set, **When** it starts, **Then** it exits before downloading and explains how to provide the missing values for automation.

---

### User Story 4 - Calm Failures and Safe Retry (Priority: P2)

As a host operator, I want a failed creation to explain what happened, what remains usable, and what to do next so that I can fix the problem and retry without losing verified prerequisites or harming an existing VM.

**Why this priority**: Registry, filesystem, network, and naming failures are expected operational conditions. A calm, actionable result with preserved work is safer than a partial VM or an opaque error.

**Independent Test**: Run the command with fixtures that fail the runtime bundle, fail the kernel, reuse an identical name, and collide with a different configuration. Verify exit statuses, failure summaries, preserved verified artifacts, and that no partial VM or duplicate VM remains.

**Acceptance Scenarios**:

1. **Given** any required runtime binary cannot be acquired, **When** provisioning starts, **Then** the command reports the failure and stops before kernel, image, or creation work.
2. **Given** the required kernel or the selected image cannot be acquired, **When** provisioning fails, **Then** the command reports which prerequisite failed, preserves previously verified members for safe retry, creates no VM, and explains the retry action.
3. **Given** a VM with the same name is already configured with identical settings, **When** creation is requested, **Then** the command reports the existing VM as already configured without creating duplicates.
4. **Given** a VM with the same name exists with different settings, **When** creation is requested, **Then** the command reports a naming conflict, leaves the existing VM unchanged, and creates nothing.
5. **Given** any expected failure occurs, **When** the command exits, **Then** it explains what happened, why the VM was not created, and what the operator can do next, with a nonzero status and no unexplained failure output.

---

### Edge Cases

- The name is empty, too long, contains path separators or whitespace, or is otherwise not path-friendly; the command rejects it before any transfer with the naming rule restated.
- A positional name and an explicit name flag are both supplied but disagree; the command reports the mismatch and starts no work.
- The registry returns no host-compatible images, the selected image's distribution has no host-compatible default kernel, or no host-compatible runtime bundle exists.
- The operator selects nothing, presses escape during the image selector, or declines the final confirmation; nothing is transferred and no VM is created.
- Disk input is non-numeric, zero, negative, or converts to fewer bytes than the selected image's registry size; the command rejects it showing the minimum in GB.
- Memory input uses an unsupported form (missing unit, unknown unit, non-numeric amount) or converts to fewer bytes than the distribution minimum; the command rejects it showing the minimum and the accepted `xMB`/`xGB` forms.
- vCPU input is non-integer, zero, or below the distribution minimum; the command rejects it showing the minimum.
- LAN exposure cannot be configured (no usable uplink, address conflict, insufficient permission); creation reports a typed network error and never silently falls back to host-only mode.
- The operator interrupts during a prompt or the confirmation; the command reports cancellation with no side effects.
- The operator interrupts during provisioning; the command cancels in a controlled way, removes the partial file without publishing it as verified, preserves already verified prerequisites, summarizes affected groups, and returns exit code `130`.
- The operator interrupts during the creation step itself; creation has no graceful cancel, so the command reports the interruption only after the operation settles, preserving its rollback behavior and never claiming a VM that was not configured.
- The terminal has no color support, is narrow, or cannot receive interactive input; focus, selection, downloaded markers, progress stages, and success/failure status remain distinguishable through text and symbols.
- The host-local base directory is unavailable or unwritable; the command explains the affected path and stops safely.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The CLI MUST expose the command as `microvm new` with an optional positional machine name and an equivalent `--name` flag. A bare `microvm new` MUST prompt for the name; `microvm new "some-name"` MUST skip the name prompt. When both the positional name and `--name` are supplied they MUST agree, otherwise the command MUST report the mismatch and start no work.
- **FR-002**: The command MUST resolve the host-local base directory according to the existing CLI policy (custom directory only through the environment, otherwise the user-home default) and pass that explicit path to the SDK operations. It MUST NOT expose the base directory through a command-line option and MUST NOT create a second home or storage policy.
- **FR-003**: Before presenting the image selection, the command MUST obtain the current published distributions, kernels, images, and binary packages and MUST reject invalid or unavailable registry data with an actionable failure. It MUST NOT begin any artifact transfer before the final confirmation (interactive) or before input validation completes (non-interactive).
- **FR-004**: The interactive flow MUST validate the machine name early (non-empty, path-friendly, within the supported length) and MUST abort with an actionable error restating the naming rule when the name is invalid; it MUST NOT re-prompt, transfer artifacts, or create a VM after an invalid name.
- **FR-005**: The interactive flow MUST present exactly one image selection step listing host-compatible images across host-compatible distributions ordered by distribution name then image name, with each entry identifying its parent distribution (image identity, variant or capabilities, and size) and a text marker showing whether that image is already downloaded or still needs downloading. The step MUST allow exactly one image with keyboard navigation, confirmation, and cancellation; an empty selection or cancellation MUST exit without downloading or creating.
- **FR-006**: The disk prompt MUST accept a positive numeric value in GB (decimal values allowed) and convert it to bytes as GB multiplied by 1024 cubed. The converted value MUST be at least the selected image's registry-reported size; on invalid or undersized input the command MUST abort with an actionable error showing the minimum in GB before any transfer, without re-prompting. The same value MUST be accepted through `--disk-gb`.
- **FR-007**: The memory prompt MUST accept decimal amounts in the forms `xMB` and `xGB` (case-insensitive, with or without a space between amount and unit, e.g. `512MB`, `512 MB`, `2GB`, `2 GB`, `1.5GB`) and convert them to bytes as megabytes multiplied by 1024 squared or gigabytes multiplied by 1024 cubed, rounding up to a whole byte. The converted value MUST be at least the parent distribution's minimum memory; on unsupported or undersized input the command MUST abort with an actionable error showing the minimum in MB/GB and the accepted `xMB`/`xGB` forms before any transfer, without re-prompting. The same value MUST be accepted through `--memory`.
- **FR-008**: The vCPU prompt MUST accept a positive integer count. On invalid input or values below the parent distribution's minimum vCPU count the command MUST abort with an actionable error showing the minimum before any transfer, without re-prompting. The same value MUST be accepted through `--vcpus`.
- **FR-009**: The network step MUST offer an explicit LAN-exposure choice defaulting to host-only (no LAN exposure). The `--expose-lan` flag MUST select LAN exposure when present; its absence MUST mean host-only. The choice MUST be persisted as the VM's network mode.
- **FR-010**: In an interactive terminal, the command MUST prompt only for values not already supplied through arguments and MUST validate explicitly supplied values with the same rules as prompted values. Explicit image selection MUST use exactly one `--image DISTRIBUTION=IMAGE` argument in the same form as `artifacts download`. In non-interactive mode (`--non-interactive`) the command MUST perform no prompts and MUST report the first missing or invalid value with usage guidance before any transfer or mutation.
- **FR-011**: Every interactive run MUST end the collection phase with a confirmation step summarizing the VM name, selected image with its parent distribution, disk, memory, vCPU count, network mode, and which prerequisites still need fetching, followed by an explicit yes/no creation question. A decline or cancellation MUST exit with no downloads and no VM. The confirmation MUST be skipped only in non-interactive mode.
- **FR-012**: After confirmation (or non-interactive validation), the command MUST provision prerequisites in dependency order: the automatically chosen host-compatible runtime bundle first (highest published version per required runtime component on the host architecture, reusing one package when it provides both components), then the selected image's distribution default kernel and the selected image. A runtime failure MUST stop all dependent work. All acquisition, integrity, cache, persistence, and compatibility behavior MUST be delegated to the existing artifact operations; the CLI MUST NOT implement its own verification or force fresh transfers of already verified artifacts, and MUST NOT present a download plan.
- **FR-013**: Only after every prerequisite is verified locally MUST the command invoke the single SDK creation operation with the derived distribution and image identifiers, disk and memory sizes in bytes, vCPU count, and LAN-exposure choice, using the default VM directory and the distribution default kernel. The command MUST NOT implement lifecycle behavior or invoke runtime mechanics directly.
- **FR-014**: The command MUST render live per-step progress with one row per prerequisite member plus the creation stages, fed by the artifact transfer events and the creation progress events. Progress MUST be truthful (no invented values, counters never exceeding totals) and MUST identify the current item, stage, byte counters where applicable, and aggregate progress using text labels or symbols in addition to any color.
- **FR-015**: The command MUST report success only after the creation operation returns a configured and stopped VM, summarizing its identity, network mode with address, volume location, resource sizes, and SSH connection reference (account, port, and private-key path; never key contents), and MUST exit with status zero.
- **FR-016**: Expected failures MUST explain what happened, why the VM was not created, and what the operator can do next; the command MUST use a nonzero exit status and MUST NOT expose an unexplained failure. Verified prerequisites MUST be preserved for safe retry, and failed versus successful groups MUST be identified separately.
- **FR-017**: A repeated request for an already-configured VM with identical settings MUST be reported as already configured without duplicate files, network resources, credentials, or runtime configuration. A request reusing an existing name with different settings MUST report a conflict and leave the existing VM unchanged.
- **FR-018**: The interactive presentation MUST follow the restrained neutral visual language used by the artifacts download flow: compact information density, clear hierarchy, semantic status emphasis, visible keyboard focus, progressive disclosure, immediate feedback, and calm failure states.
- **FR-019**: If the operator interrupts during provisioning, the command MUST cancel the in-flight transfer in a controlled way, start no subsequent members, remove the partial file without publishing it as verified, preserve already verified prerequisites, report the affected groups, and return exit code `130`.
- **FR-020**: This feature covers exactly one image per invocation with its distribution default kernel, the default VM directory, and automatic network addressing. A kernel override, custom volume path, explicit LAN address, multiple images, and start/stop/connect lifecycle operations are out of scope and MUST NOT be added in this feature.

### Key Entities *(include if feature involves data)*

- **New VM Request**: The collected creation inputs for one invocation: machine name, parent distribution with selected image, disk size, memory size, vCPU count, and network mode, however supplied (prompted or explicit).
- **Image Choice**: The single operator-chosen distribution image together with its parent distribution, the automatically resolved default kernel, the registry-reported image size governing the disk minimum, and the distribution minimums governing memory and vCPUs.
- **Resource Sizes**: The user-entered disk (GB) and memory (`xMB`/`xGB`) values plus their byte conversions sent to the creation operation; vCPU count as an integer.
- **Network Choice**: The explicit host-only or LAN-exposed mode selected for the VM; host-only is the default and there is no silent fallback between modes.
- **Provisioning Step**: One per-step progress row for a prerequisite member (runtime bundle, default kernel, selected image) or the creation stages, carrying its identity, stage, byte counters where applicable, and reuse-versus-transfer disposition.
- **Creation Summary**: The pre-confirmation plan (chosen values plus prerequisites still to fetch) and the post-creation result (identity, stopped configured state, network address, volume location, SSH connection reference) reported by the command.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In usability tests with at least three available distributions, an operator can type or accept a name, select exactly one image, enter disk, memory, and vCPU values, answer the network question, and reach confirmation using only the keyboard in under 120 seconds without selecting an incompatible option.
- **SC-002**: In 100% of repeated-creation tests for an unchanged selection with a warm cache, every verified prerequisite is reused without a fresh transfer while live creation progress is still reported through completion.
- **SC-003**: After confirmation, the command shows a running state within one second and keeps updating visible step or aggregate progress until every provisioning member and the creation operation reach a terminal state.
- **SC-004**: In 100% of non-interactive tests with a complete explicit set, the command performs no prompts and produces a deterministic exit status and summary suitable for automation; in 100% of tests with a value omitted, it reports the missing value and starts no transfer and creates no VM.
- **SC-005**: In 100% of validation tests, disk values below the image size, memory values below the distribution minimum, and vCPU values below the distribution minimum are rejected before any transfer with the applicable minimum shown.
- **SC-006**: In 100% of failure-path tests, the command returns a nonzero status, identifies the failed prerequisite or stage, preserves verified prerequisites for retry, creates no partial VM on provisioning failure, and gives a concrete retry or repair action.
- **SC-007**: In 100% of non-color and narrow-terminal tests, prompt focus, selected image, downloaded markers, progress stages, and success/failure status remain distinguishable through text or symbols.
- **SC-008**: In 100% of successful creation tests, exactly the selected image, its distribution default kernel, and the required runtime bundle are provisioned; no unselected image is stored as part of the operation.

## Assumptions

- The SDK already exposes the authoritative registry listing, single-image and kernel acquisition with progress, cache, integrity, compatibility, and local-inventory behavior, plus the creation operation with live progress events; this command reuses all of it and adds no new acquisition or lifecycle semantics.
- The resolved default kernel for the image's distribution is the same kernel creation resolves, so provisioning it during this flow always leaves creation satisfiable for the downloaded image.
- Runtime bundle selection keeps the existing policy: highest published version per required component on the host architecture, reusing one package when it provides both components.
- GB means gibibytes (1024 cubed bytes) and MB means mebibytes (1024 squared bytes) for both disk and memory inputs; memory matching is case-insensitive with optional spacing and fractional disk and memory values are accepted, with fractional byte results rounded up.
- The per-image downloaded marker is an optimistic presence hint from one local inventory query with no registry access and no file hashing; a stale or corrupted file can still show as downloaded, and provisioning revalidates integrity and repairs it before creation. Kernel and runtime readiness are resolved silently during provisioning rather than shown per image.
- Host-only networking is the default; LAN exposure is explicit opt-in with automatic addressing and no caller-supplied address in this feature.
- The VM directory always uses the managed default location for the given name; no custom volume path is offered in this feature.
- The CLI keeps the existing home policy: a custom base directory only through the environment, defaulting to the user-home path otherwise, passed explicitly to the SDK operations.
- Interrupting the creation step itself cannot cancel it gracefully; the command reports the interruption after the operation settles with its rollback behavior intact.
- All operator-facing text for this feature is English; localization is out of scope.
