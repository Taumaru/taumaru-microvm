# Data Model: Image Artifact Download

## Persistence boundary

This feature adds no SQLite tables and no migration. The CLI does not open the database
directly. It passes the resolved home path to `MicroVmSdk`, which owns the existing
normalized inventory and its migration lifecycle.

The SDK remains responsible for durable records such as:

- physical verified files and their absolute/relative paths, expected and actual size,
  SHA-256, verification status, and timestamps;
- kernel records and their download relation;
- binary packages, binary components, and their download relations;
- distributions, distribution images, boot metadata, image download relations, and
  distribution-to-kernel compatibility with exactly one default per distribution;
- MicroVM records referencing `distribution_id`, `image_id`, and the resolved
  `kernel_id` at creation time.

Single-image acquisition writes exactly one `distribution_images` row (plus the shared
distribution, boot-args, and kernel-reference upserts that row already performs) keyed
by the existing `distribution_image:distro:image` artifact key. Whole-distribution
acquisition writes one row per image through the same path. The two operations converge
on identical rows; verification is per file, so no cross-image state leaks.

The CLI's model is an ephemeral plan and result model. It exists only for one command
invocation, so a cancelled or failed plan never creates a second source of truth about
installed artifacts.

## In-memory entities

### `ImageSelection`

One operator-chosen image with its resolved default kernel. The pair
`(distribution.id, image.id)` is the plan key; display names never identify a row.

| Field | Type | Meaning |
|---|---|---|
| `distribution` | `Distribution` | Host-compatible published distribution owning the image |
| `image` | `DistributionImage` | Host-compatible (via distribution) published image |
| `kernel` | `Kernel` | That distribution's published default kernel, host-compatible |
| `expected_bytes` | `u64` | `image.size_bytes` (checked into the plan total) |

Selections are deduplicated by `(distribution.id, image.id)` and sorted by that pair
before review. `kernel` is resolved, never chosen: it always equals
`distribution.default_kernel` as published.

### `RuntimeBinarySelection`

Unchanged from the prior feature: the automatically selected host runtime packages.
Each required component (`firecracker`, `firectl`) resolves separately; one package is
reused when it provides both, while split registry packages are retained separately.

| Field | Type | Meaning |
|---|---|---|
| `packages` | `Vec<BinaryPackage>` | Highest valid host-compatible package per required component, deduplicated by package ID |
| `file_count` | `usize` | All files the SDK will acquire from all selected packages |
| `expected_bytes` | `u64` | Checked sum of all selected package file sizes |

Additional package files are included because `download_binary` operates on complete
packages. Runtime package IDs are sorted before they enter the execution plan.

### `DownloadPlan`

The immutable plan created before confirmation and executed only after confirmation.

| Field | Type | Meaning |
|---|---|---|
| `runtime` | `RuntimeBinarySelection` | First execution stage |
| `selections` | `Vec<ImageSelection>` | One row per unique selected image, sorted by `(distribution.id, image.id)` |
| `unique_kernels` | `Vec<Kernel>` | Deduplicated resolved default kernels, sorted by kernel ID |
| `members` | `Vec<PlanMember>` | Explicit runtime, kernel, and per-image execution order |
| `expected_bytes` | `u64` | Runtime files + unique default kernels + selected images only |

Plan construction uses checked arithmetic. Overflow, empty selections, unknown
distributions or images, image-outside-distribution mismatches, incompatible
architectures, unresolvable or incompatible default kernels, and missing runtime
components are validation failures before confirmation.

### `PlanMember`

The executable unit in the plan.

```text
RuntimeBinary { package_id, expected_bytes }
Kernel { kernel_id, expected_bytes }
DistributionImage { distribution_id, image_id, expected_bytes }
```

Runtime members come first (sorted by package ID), then unique default-kernel members
(sorted by kernel ID), then one member per selected image (sorted by distribution,
then image). Each image member maps to exactly one
`download_distribution_image(distribution_id, image_id)` call.

### `DownloadProgressView`

Unchanged: the renderer's normalized view of an SDK callback.

| Field | Type | Meaning |
|---|---|---|
| `artifact_kind` | `ArtifactKind` | `Binary`, `Kernel`, or `DistributionImage` |
| `artifact_id` | `String` | Package, kernel, or distribution ID |
| `member_name` | `Option<String>` | File or image ID for multi-file artifacts |
| `stage` | `ProgressStage` | `Downloading`, `Verifying`, `Cancelled`, or another terminal stage |
| `current_bytes` | `u64` | Current member bytes reported by the SDK |
| `member_total_bytes` | `u64` | Current member expected bytes |
| `plan_completed_bytes` | `u64` | Expected bytes for verified members completed before this SDK operation |
| `plan_current_bytes` | `u64` | `plan_completed_bytes` plus the SDK operation aggregate |
| `plan_total_bytes` | `u64` | `DownloadPlan.expected_bytes` |
| `disposition` | `Option<DispositionLabel>` | `Downloaded`, `Adopted`, or `Already available` for terminal events |

Image events arrive as `(DistributionImage, distro, Some(image))` and render as
`distribution/{distro}/{image}` with no renderer change. Aggregate progress derives
only from SDK counters plus plan metadata — never from elapsed time or assumed rate.

### `MemberOutcome`

The result of one plan member.

```text
Verified { label, availability }
Failed { label, reason }
Skipped { label, reason }
Cancelled { label }
```

`Skipped` covers images whose resolved default kernel failed: the image is never
attempted, the kernel failure is reported once, and each dependent image records the
dependency. An SDK operation may verify its file before a later member fails; those
files stay represented as retained work and are never deleted by the CLI.

### `DownloadOutcome`

The command-level terminal result.

```text
Succeeded { members, verified_bytes }
PartiallyFailed { successes, failures, retryable: true }
FailedBeforeTransfer { reason }
Cancelled { stage }
```

`Succeeded` is possible only when every runtime package, every unique resolved
default kernel, and every selected image returns a verified SDK result. Any failed
required member produces a non-success outcome. `FailedBeforeTransfer` covers invalid
input, unavailable registry data, no compatible package for a required runtime
component, unresolvable default kernels, and empty selections. `Cancelled` covers
prompt or confirmation cancellation before transfer and controlled `Ctrl-C`
cancellation during transfer: signal the SDK, wait for cleanup, start no further
members, preserve verified groups, remove the unverified temporary artifact, and
return exit code `130`.

## State transitions

```text
Discovering
    -> SelectingImages
    -> Reviewing
    -> AwaitingConfirmation
    -> DownloadingRuntime
    -> DownloadingKernels
    -> DownloadingImages
    -> Succeeded

Any pre-transfer state -> Cancelled | FailedBeforeTransfer
DownloadingRuntime -> FailedBeforeTransfer/Failed (runtime failure; no dependent downloads)
DownloadingKernels or DownloadingImages -> Cancelled | PartiallyFailed | Succeeded
```

Transition rules:

1. `Discovering` must complete all required SDK list calls before the selector or
   explicit-plan validation runs.
2. `SelectingImages` cannot transition to `Reviewing` with an empty selection; repeats
   collapse to one entry before review.
3. `Reviewing` computes sorted order, default-kernel resolution, and all checked byte
   totals before confirmation is offered.
4. No `Downloading*` state is reachable until confirmation succeeds.
5. A runtime package failure is terminal for the transfer phase because all later work
   depends on the complete runtime stage.
6. Kernel and image groups after a successful runtime stage are attempted in plan
   order; a kernel failure skips only its dependent images, while unrelated groups
   continue and verified work is not erased.
7. A transfer cancellation signals the SDK, waits for the current operation's cleanup,
   prevents subsequent group execution, retains successful earlier groups, and removes
   the unverified temporary artifact.
8. Only a fully verified plan transitions to `Succeeded`.

## Validation invariants

- Every selected pair names a distribution present in the same catalog snapshot and an
  image published by that distribution.
- Every selected distribution matches the host architecture (images inherit it).
- Every resolved default kernel is present in the catalog, matches the host
  architecture, and equals the distribution's published `default_kernel`; there is no
  override path.
- A repeated pair deduplicates; it is never a validation error and never transfers
  twice.
- Every registry ID is displayed and executed as data; it is never turned into a
  filesystem path by the CLI.
- The runtime packages match the host architecture and collectively provide both
  required components; a package containing both is selected only once.
- Shared default kernels are deduplicated by registry ID within the current plan; the
  SDK still decides whether the physical file is downloaded, adopted, or skipped.
- A selection is not reported ready merely because a local path exists. Readiness
  comes only from a successful SDK result that includes integrity and inventory
  decisions.
