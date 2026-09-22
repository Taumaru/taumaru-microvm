# Feature Specification: Prune Unused Artifacts

**Feature Branch**: `015-prune-unused-artifacts`

**Created**: 2026-09-22

**Status**: Draft

**Input**: User description: "Develop an SDK function that prunes downloaded artifacts. When executed, the SDK looks for downloaded distro kernels and images that are not in use by any MicroVM, and deletes every such image and kernel that is not linked to any existing MicroVM. What matters is whether the MicroVM exists in the database, not whether it is running."

## Clarifications

### Session 2026-09-22

- Q: Should prune delete only fully downloaded and verified kernels/images, or also incomplete or failed download records that no MicroVM references? → A: Include incomplete/failed — any kernel/image record (complete, incomplete, or failed) with zero MicroVM references is a deletion candidate.
- Q: If a download is actively transferring an unreferenced kernel/image while prune runs, should prune skip it or delete it? → A: Skip active transfers — artifacts with an actively in-progress transfer are left fully intact.
- Q: When some deletions succeed and others fail, should prune return the partial success list inside the error or discard it? → A: Error carries partial results — the typed failure includes the removed list, freed bytes so far, and per-artifact failure causes.
- Q: Should the prune result list actively-skipped transfers separately from removed and failed artifacts? → A: Separate skipped list — the result exposes removed, failed, and skipped (active transfers) as three distinct lists.
## User Scenarios & Testing *(mandatory)*

### User Story 1 - Reclaim disk from unused kernels and images (Priority: P1)

An application developer calls the prune operation against the explicit SDK home. The operation finds every locally downloaded distro kernel and distro image that no existing MicroVM record references, deletes those files and their inventory records, keeps every artifact referenced by at least one existing MicroVM, and returns a summary of what was removed and how much space was freed.

**Why this priority**: This is the entire value of the feature: safe disk reclamation without threatening any existing machine. Everything else is a boundary or failure path around this flow.

**Independent Test**: Seed a home with several downloaded kernels and images, create MicroVMs referencing only a subset of them (covering stopped, running, and never-started machines), run prune once, and verify that exactly the unreferenced artifacts are gone from disk and inventory while every referenced artifact is intact.

**Acceptance Scenarios**:

1. **Given** downloaded kernels and images where some are referenced by at least one existing MicroVM record and others are referenced by none, **When** the caller runs prune, **Then** every unreferenced kernel and image is deleted from disk and its inventory record removed, every referenced kernel and image is left fully intact, and the result lists what was removed with freed space per kind and in total.
2. **Given** a kernel shared by two existing MicroVMs where one of those MicroVMs is later deleted, **When** the caller runs prune, **Then** the shared kernel is kept because one remaining MicroVM still references it.
3. **Given** a MicroVM that exists but is stopped, was never started, or whose process died outside the SDK, **When** the caller runs prune, **Then** the artifacts it references are treated as in use and are never deleted.
4. **Given** a successful prune, **When** the caller inspects any remaining MicroVM, **Then** that MicroVM can still resolve its kernel and image exactly as before prune ran.

---

### User Story 2 - Safe no-op when everything is in use (Priority: P1)

An application developer runs prune on a home where every downloaded kernel and image is referenced by at least one existing MicroVM, or where nothing is downloaded at all. The operation changes nothing and reports success with an empty removal list.

**Why this priority**: Prune will often run on a clean host (for example on a schedule or before a download). A safe, honest no-op makes it trustworthy to call blindly; deleting or failing when there is nothing to do would make it dangerous.

**Independent Test**: Run prune on a home with fully referenced artifacts and on an empty home, and verify both calls succeed, change nothing on disk or in inventory, and report zero removals.

**Acceptance Scenarios**:

1. **Given** downloaded kernels and images that are all referenced by existing MicroVMs, **When** the caller runs prune, **Then** the operation succeeds, deletes nothing, and reports zero removals with zero freed space.
2. **Given** a home with no downloaded kernels or images, **When** the caller runs prune, **Then** the operation succeeds with zero removals and makes no inventory or filesystem changes.

---

### User Story 3 - Survive partial deletion failures without threatening machines (Priority: P2)

An application developer runs prune where one unreferenced artifact cannot be deleted (for example a permission problem or a file locked by the host). The operation still reclaims every other unreferenced artifact, never touches referenced ones, and reports the failure with enough detail to repair and retry.

**Why this priority**: Artifact directories live on real hosts with real permission and locking surprises. One stubborn file must not block all reclamation or, worse, cause referenced artifacts to be touched.

**Independent Test**: Seed several unreferenced artifacts, make exactly one of them undeletable, run prune, and verify the remaining unreferenced artifacts are reclaimed, all referenced artifacts are intact, and the outcome identifies the failed artifact and preserves the list of successful removals.

**Acceptance Scenarios**:

1. **Given** several unreferenced artifacts where one deletion fails, **When** the caller runs prune, **Then** all other unreferenced artifacts are still deleted, no referenced artifact is modified, and the caller receives a typed failure identifying the failed artifact while retaining the record of what was successfully removed.
2. **Given** a failed prune, **When** the caller fixes the cause and runs prune again, **Then** the previously failed artifact is reclaimed and the retry succeeds.

---

### Edge Cases

- No MicroVM record exists in the local inventory: every downloaded kernel and image is unreferenced, so prune removes all of them.
- An artifact is recorded in the inventory but its file is already absent from disk: prune drops the stale record and reports it as reclaimed with zero freed bytes, rather than failing.
- A file sits in the artifact directories but has no inventory record: prune leaves it untouched; only inventory-recorded artifacts are in scope.
- A MicroVM record references a kernel or image that was never downloaded: prune ignores that reference for deletion purposes and changes nothing for it.
- The same kernel or image is referenced by several MicroVMs: a single remaining reference is enough to keep it.
- A MicroVM is created or deleted concurrently with prune: artifacts referenced by any MicroVM present during the operation are never deleted.
- The home directory is invalid, unreadable, or its inventory cannot be opened: prune returns a typed error and deletes nothing.
- A deletion fails midway (permissions, lock, I/O error): prune continues with the remaining candidates and reports the failure without touching referenced artifacts.
- An unreferenced artifact has an actively in-progress transfer while prune runs: prune skips it, leaves it fully intact, and reports it as skipped rather than removed or failed.
- Repeated prune calls with no changes between them: every call after the first reports zero removals.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The prune operation MUST accept the explicit SDK home directory from its caller and MUST NOT require any other input.
- **FR-002**: The operation MUST enumerate locally recorded distro kernels and distro images as recorded in the host-local inventory, regardless of download completeness status (complete, incomplete, or failed).
- **FR-003**: The operation MUST build the referenced set from every MicroVM record present in the local inventory, regardless of lifecycle state (running, stopped, never started, or process gone); running state MUST NOT influence the decision.
- **FR-004**: Every recorded kernel or image with zero references from existing MicroVM records MUST be deleted from disk and have its inventory record removed, including incomplete or failed download records and their partial files, except artifacts with an actively in-progress transfer, which MUST be skipped and left fully intact.
- **FR-005**: Every downloaded kernel or image referenced by at least one existing MicroVM record MUST be left fully intact, including when the referencing MicroVM is stopped, never started, or its process is gone.
- **FR-006**: The operation MUST NOT delete, modify, or otherwise touch runtime binary packages, supporting artifacts, tool binaries, the state database itself, or any MicroVM volume, key, socket, or runtime file.
- **FR-007**: On success the operation MUST return a summary listing the removed kernels, the removed images, the count removed per kind, the freed bytes per kind and in total, and the skipped active-transfer identities as a separate list.
- **FR-008**: When no downloaded kernel or image is unreferenced, the operation MUST succeed with zero removals and make no filesystem or inventory changes.
- **FR-009**: The operation MUST be idempotent: repeating it with no intervening changes MUST succeed and report zero further removals.
- **FR-010**: When one or more deletions fail, the operation MUST continue with the remaining candidates, MUST NOT touch referenced artifacts, and MUST return a typed failure carrying the partial results: the removed kernel and image identities, the freed bytes so far (per kind and in total), the skipped active-transfer identities as a separate list, and the per-artifact failure causes.
- **FR-011**: When an inventory-recorded kernel or image has no corresponding file on disk, the operation MUST drop the stale record, report it as reclaimed with zero freed bytes, and MUST NOT treat it as a failure.
- **FR-012**: Files inside the artifact directories with no inventory record MUST be left untouched.
- **FR-013**: Concurrent MicroVM creation, deletion, or artifact acquisition MUST NOT cause prune to delete an artifact referenced by any MicroVM present while the operation runs.
- **FR-014**: All expected failure paths (invalid or unreadable home, unreadable inventory, deletion failures) MUST be returned as typed errors; the operation MUST NOT panic, terminate the calling process, write to standard output or error, emit logs, or hide behavior in global side effects.

### Key Entities

- **Downloaded kernel**: A distro kernel recorded in the host-local inventory with its identity and verified file; eligible for pruning only when no existing MicroVM record references it.
- **Downloaded image**: A distro image recorded in the host-local inventory with its identity, parent distro, and verified file; eligible for pruning only when no existing MicroVM record references it.
- **MicroVM reference**: The link between an existing MicroVM record and the kernel and image it was created from; existence of the record alone decides use, never its lifecycle state.
- **Prune summary**: The outcome of a successful run: removed kernel identities, removed image identities, counts per kind, freed bytes per kind and in total, and skipped active-transfer identities as a separate list; removed lists are empty when nothing was unreferenced.
- **Prune failure**: The typed outcome when at least one deletion fails: the removed kernel and image identities, the freed bytes so far, the skipped active-transfer identities as a separate list, and the per-artifact failure causes, so the caller can repair and retry.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In 100% of mixed-ownership tests (referenced plus unreferenced kernels and images, including stopped and never-started owners), a prune run deletes every unreferenced kernel and image and leaves every referenced one byte-identical and resolvable.
- **SC-002**: In 100% of fully-referenced and empty-home tests, prune succeeds, changes nothing on disk or in inventory, and reports zero removals.
- **SC-003**: A caller that seeds unreferenced artifacts, runs prune, and runs prune again with no changes in between observes all reclamation on the first run and zero removals on the second run.
- **SC-004**: In 100% of single-undeletable-file tests, prune still reclaims every other unreferenced artifact, touches no referenced artifact, and the failure outcome names the failed artifact and preserves the successful removals.
- **SC-005**: After any successful prune, every remaining MicroVM passes its normal artifact preflight (kernel and image resolve as before) with no re-download required.

## Assumptions

- Only distro kernels and distro images are in scope; runtime binary packages, supporting artifacts, tool binaries, and MicroVM volumes are never candidates, even when unreferenced.
- "In use" means referenced by a MicroVM record existing in the host-local inventory at operation time; lifecycle state, process liveness, and socket responsiveness are irrelevant to this decision.
- The host-local inventory is the source of truth for what counts as downloaded; stray files without an inventory record are out of scope and left untouched.
- The SDK receives the base directory explicitly from its caller and owns the layout below it; home resolution from environment or command-line options is a CLI concern and out of scope here.
- The operation deletes immediately with no preview or confirmation step; any dry-run or confirmation UX belongs to a future CLI feature, not this SDK capability.
- Freed-space accounting uses the verified file sizes observed at deletion time; files already absent contribute zero bytes.
- All operator- and developer-facing text for this feature is English; localization is out of scope.
