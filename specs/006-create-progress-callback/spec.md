# Feature Specification: MicroVM Creation Progress Callback

**Feature Branch**: `006-create-progress-callback`

**Created**: 2026-09-19

**Status**: Draft

**Input**: User description: "The SDK create-microvm function works, but it exposes no callback or any way to see which step it is on or what percentage of the process is complete. There is no way to get real-time feedback, which makes building the CLI hard. The function should accept a callback, similar to the download callbacks, that it notifies with what is happening: status, percentage, and similar."
## Clarifications

### Session 2026-09-19

- Q: How should a caller attach the progress observer to MicroVM creation? → A: Change the existing `create_microvm` signature to accept the request plus an optional observer directly; backward compatibility is not required because the SDK is not yet published.
- Q: When creation is rejected because the name already exists with different settings (or a non-configured state), what should the observer receive? → A: A single terminal `failed` event attributed to the validation stage, with the existing typed error returned unchanged (no new terminal outcome).
- Q: What should the reported total step count include? → A: The total is the six stages; each stage event reports N/6 and the terminal event repeats 6/6 (or the steps finished before failure) plus its outcome.
- Q: When creation fails partway, what step counters should the terminal failed event carry? → A: Terminal-only failed event: the N finished stages already emitted their events, and the terminal `failed` event alone carries N/6 plus the failed stage (no extra per-stage event for the failure itself).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Observe Creation Stages Live (Priority: P1)

As an application developer, I want to pass a progress observer when requesting MicroVM creation so that my application (including the future CLI) receives live, ordered updates about which creation step is running and how far along the whole operation is.

**Why this priority**: This is the core value of the feature: creation is a long multi-stage operation (validation, artifact resolution, volume copy, credentials, networking, finalization), and without live feedback every consumer must either block silently or invent its own progress.

**Independent Test**: Run creation with a recording observer against fixture artifacts and assert the recorded stream contains ordered events for every creation stage plus exactly one terminal event, while the returned VM metadata matches a no-observer run field for field.

1. **Given** a valid creation request and an attached observer, **When** creation runs to success, **Then** the observer receives exactly one event per finished stage, in stage order (request validation, prerequisite resolution, volume and root-filesystem preparation, credential setup, network configuration, finalization), followed by exactly one terminal `completed` event.
2. **Given** an attached observer, **When** any creation event arrives, **Then** it identifies the finished stage plus completed steps and total steps, so the caller can render current status and overall fraction without guessing.
3. **Given** an attached observer, **When** creation finishes, **Then** the returned VM identity, volume, network, and SSH metadata are identical to what the same request returns without an observer.

---

### User Story 2 - Same Results Through the New Signature (Priority: P1)

As an SDK consumer, I want creation without an observer to produce the same results as before so that the only change for existing call sites is passing the new observer argument.

**Why this priority**: The SDK is not yet published, so the existing `create_microvm` signature may change without backward compatibility; all in-repo call sites are updated in the same change, and result, error, idempotency, and rollback behavior stay unchanged.

**Independent Test**: Run the full existing creation suite (success, idempotent repeat, conflict, and failure paths) through the new signature with no observer attached and verify zero behavior differences apart from the new argument.

**Acceptance Scenarios**:

1. **Given** no observer is passed through the new signature, **When** creation succeeds, **Then** the result, persisted state, and host effects are identical to current behavior.
2. **Given** no observer is passed through the new signature, **When** creation fails or conflicts, **Then** the typed error and rollback behavior are identical to current behavior.

---

### User Story 3 - Truthful Measurable Progress (Priority: P2)

As a CLI builder, I want every progress event to carry honest, checkable counters so that the CLI can render stage labels and completion fractions without inventing percentages.

**Why this priority**: Creation mixes discrete steps (validation, key generation, persistence) with byte-moving work (root-filesystem copy). Only truthful counters let the CLI show progress that never jumps backward or exceeds 100%.

**Independent Test**: Record the event stream of a successful observed creation and assert step counters are monotonic, byte counters never exceed their stated totals, and byte totals match already-verified sizes where reported.

**Acceptance Scenarios**:

1. **Given** an observed creation, **When** events arrive, **Then** completed-step counters never decrease within the operation and never exceed the reported total steps.
2. **Given** a stage that moves a known number of bytes (e.g. the root-filesystem copy), **When** its events arrive, **Then** they report bytes completed and expected bytes consistent with the verified artifact sizes.
3. **Given** a stage with no measurable byte total, **When** its events arrive, **Then** they report step counters only and never a fabricated percentage.

---

### User Story 4 - Failure Remains Visible and Safe (Priority: P2)

As an application developer, I want a failed observed creation to end its event stream deterministically while keeping the existing typed error and rollback so that my UI can stop its indicator and still handle the error normally.

**Why this priority**: A live stream that ends silently on failure leaves consumers hanging; a stream that changes error semantics breaks existing handling. Both must hold at once.

**Independent Test**: Inject a prerequisite failure (e.g. missing local artifact) with an observer attached and verify the stream ends with a terminal `failed` event naming the failed stage, the typed error matches the no-observer error, and no partial VM remains.

**Acceptance Scenarios**:

1. **Given** an attached observer, **When** creation fails, **Then** the stream ends with exactly one terminal `failed` event identifying the failed stage, and no `completed` or `already-configured` terminal event is emitted.
2. **Given** an attached observer, **When** creation fails, **Then** the returned typed error and host rollback are identical to the same failure without an observer.
3. **Given** an identical repeat of an already-configured VM with an observer, **When** creation returns the existing VM, **Then** the stream contains a single terminal `already-configured` event and performs no host changes.

---

### Edge Cases

- What happens when the same creation runs concurrently for different VMs? Events from each operation go only to that call's observer; streams never interleave across calls.
- What happens when creation is rejected as a name conflict (different settings, or an existing VM in a non-configured state)? The stream contains a single terminal `failed` event attributed to the validation stage, and the existing typed conflict error is returned unchanged.
- What happens when the operation is an idempotent repeat? A single terminal `already-configured` event is emitted and no stage events or host changes occur.
- How are progress events kept out of persisted state? Events are transient notifications only; nothing about the stream is stored in the local inventory.
- What is out of scope? Cancellation of creation, progress for `configure_network`, and any CLI rendering of the events.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `create_microvm` MUST accept the creation request plus a caller-provided progress observer; the observer is optional per call (callers that pass none receive no progress events). Backward compatibility of the previous signature is not required; all in-repo call sites are updated in the same change.
- **FR-002**: Creation with no observer passed MUST produce the same results, typed errors, idempotency and conflict behavior, and rollback as today, and emit no progress events.
- **FR-003**: With an observer attached, the SDK MUST emit exactly one event when each creation stage finishes, in stage order: request validation, prerequisite resolution, volume and root-filesystem preparation, credential setup, network configuration, and finalization. A successful creation therefore produces six stage events followed by exactly one terminal `completed` event (seven events total).
- **FR-004**: Every stage event MUST identify the stage just finished and report completed steps out of a fixed total of six (first stage reports 1/6 through sixth reporting 6/6); step counters MUST be monotonic within one operation. The terminal event MUST repeat the finished-step count out of six (6/6 on `completed`, steps finished before the failure on `failed`) plus its outcome.
- **FR-005**: The stage event for a stage that moves a known number of bytes MUST report final bytes completed and expected bytes consistent with verified artifact sizes; stage events for stages without a measurable total MUST report step counters only and MUST NOT fabricate percentages. No intermediate in-progress events are emitted inside a stage.
- **FR-006**: Every observed operation MUST end with exactly one terminal event: `completed` (new VM configured, emitted after the six stage events, carrying 6/6), `already-configured` (idempotent repeat, with no preceding stage events), or `failed` (carrying N/6 for the N stages finished before the failure, naming the failed stage, with no success terminal and no extra stage event for the failure itself).
- **FR-007**: The observer MUST be the only progress channel: the SDK MUST NOT write progress to stdout/stderr, emit logs, or use any other side channel to report creation progress.
- **FR-008**: Events from one creation operation MUST be delivered only to that call's observer, including when multiple creations run concurrently.
- **FR-009**: Rollback, typed-error, idempotency, and conflict behavior MUST remain unchanged; progress events MUST NOT be persisted as VM state and MUST NOT alter any persisted metadata.
- **FR-010**: The progress event shape (stage identity, step counters, optional byte counters, terminal outcome) MUST be part of the documented public SDK contract with compatibility notes, consistent with the existing download-progress callback convention.

### Key Entities *(include if feature involves data)*

- **Creation Progress Event**: A transient notification emitted once when a stage finishes (or once as the terminal outcome), carrying the finished stage's identity, completed steps, total steps, and optional final byte counters (bytes completed and expected bytes, only where measurable).
- **Creation Stage**: One named discrete phase of creation: request validation, prerequisite resolution, volume and root-filesystem preparation, credential setup, network configuration, finalization.
- **Progress Observer**: The caller-provided sink that receives the ordered event stream for exactly one creation call; it is infallible and non-blocking and cannot change the operation's outcome.
- **Terminal Outcome**: The single closing state of an observed operation: `completed`, `already-configured`, or `failed` (with the failed stage).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In 100% of observed test creations, the recorded stream contains exactly six ordered stage events (one per finished stage) followed by exactly one terminal `completed` event.
- **SC-002**: In 100% of comparison tests, the VM metadata returned with an observer attached is field-identical to the metadata returned without one.
- **SC-003**: In 100% of failure-path tests, the typed error and host rollback with an observer match the no-observer behavior, and no success terminal event is emitted.
- **SC-004**: In 100% of observed runs, step counters are monotonic and never exceed the stated total, and every reported byte counter stays within its stated expected total.
- **SC-005**: A reference renderer built using only the event stream displays a current-stage label and an overall completion fraction update for every event in 100% of recorded streams, with no additional SDK information.
- **SC-006**: All pre-existing creation scenarios (success, idempotent repeat, conflict, failure paths) pass through the new signature with no observer attached and produce unchanged results, demonstrating zero behavior change apart from the new argument.

## Assumptions

- The SDK is not yet published, so the `create_microvm` signature may change without preserving backward compatibility; all in-repo call sites are migrated in the same change, and no-observer behavior is otherwise unchanged with no caller migration beyond passing the new argument.
- The observer follows the existing download-callback convention: caller-owned, infallible, non-blocking, invoked from the operation itself, and unable to alter integrity, persistence, or error decisions.
- Total steps are fixed at six (one per creation stage); byte totals are reported only where the SDK already works with verified sizes (notably the root-filesystem copy).
- Cancellation of creation is out of scope; there is no cancellation token in this feature.
- Progress for the independent network-reconciliation operation is out of scope.
- CLI presentation of these events is out of scope for this feature; this feature only enables it.
- Concurrent creations are independent; per-call routing of events is sufficient and no cross-operation aggregation is provided.
