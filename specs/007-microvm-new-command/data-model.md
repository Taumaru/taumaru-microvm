# Data Model: CLI `new` Command for Guided MicroVM Creation

## Persistence boundary

This feature adds no SQLite tables and no migration. The CLI does not open
the database directly. It passes the resolved home path to `MicroVmSdk`,
which owns the existing normalized inventory and its migration lifecycle.

The SDK remains responsible for durable records such as:

- physical verified files and their absolute/relative paths, expected and
  actual size, SHA-256, verification status, and timestamps;
- kernel records and their download relation;
- binary packages, binary components, and their download relations;
- distributions, distribution images, boot metadata, image download relations,
  and distribution-to-kernel compatibility with exactly one default per
  distribution;
- MicroVM records referencing `distribution_id`, `image_id`, the resolved
  `kernel_id`, runtime package IDs, volume paths, network attachment,
  credentials (path-only references), and lifecycle state.

The new SDK query (`is_distribution_image_ready`) reads the existing
per-image inventory row plus a file-integrity recheck and returns a boolean;
it writes nothing. A cancelled or failed `new` invocation never creates a
second source of truth about installed artifacts or VMs.

The CLI's model is an ephemeral request and result model. It exists only for
one command invocation.

## In-memory entities

### `NewVmRequest`

The collected creation inputs for one invocation, however supplied (prompted
or explicit). Immutable once confirmed (interactive) or validated
(non-interactive).

| Field | Type | Meaning |
|---|---|---|
| `name` | `String` | Path-friendly VM identifier: 1–64 ASCII, starts alphanumeric, rest alphanumeric/`-`/`_` |
| `distribution_id` | `String` | Parent distribution of the selected image |
| `image_id` | `String` | The single selected image |
| `disk_size_bytes` | `u64` | Converted GB value, at least the image registry `size_bytes` |
| `memory_bytes` | `u64` | Converted MB/GB value, at least the distribution minimum |
| `vcpu_count` | `u32` | Requested vCPUs, at least the distribution minimum |
| `expose_on_lan` | `bool` | `true` only with `--expose-lan` or an affirmative LAN answer |

The pair `(distribution_id, image_id)` is the image key; display names never
identify a row. `lan_address` is always `None` and `volume_path` is always
`None` in this feature (managed defaults); both fields exist on the SDK
request but are never set by this command.

### `ImageChoice`

The single operator-chosen image with its resolved context.

| Field | Type | Meaning |
|---|---|---|
| `distribution` | `Distribution` | Host-compatible published distribution owning the image |
| `image` | `DistributionImage` | Host-compatible (via distribution) published image |
| `kernel` | `Kernel` | That distribution's published default kernel, host-compatible |
| `downloaded` | `bool` | Optimistic presence from one local inventory query, or still needs fetching |
| `expected_bytes` | `u64` | `image.size_bytes` (checked into the provisioning total) |

Choices are ordered by `(distribution.id, image.id)` in the selector. There
is exactly one per invocation; an empty selection is a cancellation, never a
default.

### `ResourceSizes`

The user-entered values plus their byte conversions sent to creation.

| Field | Type | Meaning |
|---|---|---|
| `disk_gb_text` | `String` | Raw disk input (e.g. `20.5`), echoed in review |
| `disk_size_bytes` | `u64` | Ceiling of GB × 1024³, checked arithmetic |
| `memory_text` | `String` | Raw memory input (e.g. `1.5GB`), echoed in review |
| `memory_bytes` | `u64` | Ceiling of MB × 1024² or GB × 1024³, checked arithmetic |
| `vcpu_count` | `u32` | Parsed integer count |

GB means gibibytes and MB means mebibytes. Memory matching is
case-insensitive with optional spacing; fractional disk and memory values are
accepted with fractional byte results rounded up. Overflow, non-numeric
input, unknown units, and sub-minimum values are validation failures before
any transfer, with the applicable minimum shown in input units (GB for disk,
MB/GB for memory).

### `ProvisioningStep`

One per-step progress row for a prerequisite member or the creation stages.

```text
RuntimeBinary { package_id, expected_bytes }
Kernel { kernel_id, expected_bytes }
Image { distribution_id, image_id, expected_bytes }
Creation { stage, completed_steps, overall_percent, outcome? }
```

Runtime members come first (sorted by package ID), then the single resolved
default kernel, then the single image. Each member maps to exactly one
cancellation-aware SDK download call. The `Creation` rows carry the live
`CreationProgress` fields: stage identity, step counters out of 6, overall
percent, in-stage phase, optional byte counters, and the single terminal
outcome. Progress derives only from SDK counters plus member metadata —
never from elapsed time or assumed rates.

### `NewVmOutcome`

The command-level terminal result.

```text
Created { result }
AlreadyConfigured { result }
FailedBeforeTransfer { reason }
FailedDuringProvisioning { successes, failures }
FailedDuringCreation { reason }
Cancelled { stage }
```

- `Created` is possible only when `create_microvm` returns `Ok` with the VM
  in the configured and stopped state; carries the full
  `MicroVmCreationResult` for the summary.
- `AlreadyConfigured` carries the returned existing VM for an identical
  repeat; no duplicate files, network resources, credentials, or runtime
  configuration are created.
- `FailedBeforeTransfer` covers invalid input, unavailable registry data,
  unresolvable default kernel, missing runtime components, name conflicts
  detectable before transfer, and prompt/confirmation cancellation before
  transfer.
- `FailedDuringProvisioning` names the failed prerequisite, preserves
  verified members for retry, and creates no VM. A runtime failure stops all
  dependent work; kernel/image failures report the affected member.
- `FailedDuringCreation` carries the typed SDK error (conflict, network,
  storage, integrity, or other) with rollback intact; a name conflict with
  different settings leaves the existing VM unchanged.
- `Cancelled` covers prompt/confirmation cancellation (no side effects) and
  controlled provisioning cancellation (shared token, partial file removed,
  verified work preserved, exit `130`). A creation-phase interrupt is
  reported only after the operation settles, preserving its outcome and
  rollback behavior.

## State transitions

```text
ResolvingName
    -> Discovering
    -> SelectingImage
    -> AskingDisk
    -> AskingMemory
    -> AskingVcpus
    -> AskingNetwork
    -> Reviewing
    -> AwaitingConfirmation
    -> ProvisioningRuntime
    -> ProvisioningKernel
    -> ProvisioningImage
    -> Creating
    -> Created | AlreadyConfigured
```

Transition rules:

1. `ResolvingName` uses the positional name or `--name` when present and
   agreeing; otherwise prompts once. Mismatch or invalid input aborts before
   discovery.
2. `Discovering` must complete all required SDK list calls before the image
   selector runs. Registry failures abort before further prompts (except the
   already-collected name).
3. `SelectingImage` cannot transition to `AskingDisk` with an empty
   selection; cancellation exits with no transfer and no VM.
4. `AskingDisk`, `AskingMemory`, `AskingVcpus` validate against the
   selection-derived minimums immediately; any invalid input aborts with the
   minimum shown, without re-prompting.
5. In an interactive terminal only missing values are prompted; supplied
   values are validated with the identical rules. In non-interactive mode no
   prompt state is reachable; the first missing or invalid value aborts.
6. No `Provisioning*` state is reachable until confirmation succeeds
   (interactive) or validation completes (non-interactive).
7. A runtime failure is terminal for the provisioning phase because all later
   work depends on the complete runtime stage.
8. `Creating` is entered only after every prerequisite returns a verified
   SDK result. Creation-phase interruption waits for the operation to settle
   and reports its outcome; the transition never fabricates a cancellation.
9. Only a fully verified provisioning plus a successful creation transitions
   to `Created`; an identical repeat transitions to `AlreadyConfigured`.

## Validation invariants

- The positional name and `--name`, when both present, must agree exactly;
  disagreement aborts before any transfer.
- Every name satisfies the path-friendly rule before discovery output beyond
  the name prompt is acted on.
- The single `--image` value uses the `distribution=image` form with both
  sides non-blank; exactly one is required in non-interactive mode.
- The selected pair names a distribution present in the same catalog
  snapshot and an image published by that distribution; both match the host
  architecture (images inherit their distribution's).
- The resolved default kernel is present in the catalog, matches the host
  architecture, and equals the distribution's published `default_kernel`;
  there is no override path.
- Disk bytes are at least the selected image's registry `size_bytes`; memory
  bytes are at least the distribution minimum; vCPUs are at least the
  distribution minimum.
- The runtime packages match the host architecture and collectively provide
  both required components (`firecracker`, `firectl`); a package containing
  both is selected only once.
- The downloaded marker is `true` for pairs in the SDK presence list (verified
  inventory rows, no file hashing); SDK errors abort the flow rather than
  rendering a marker.
- A request is never reported ready merely because a local path exists.
  Creation readiness comes only from verified SDK results during provisioning.
- Every registry ID is displayed and executed as data; it is never turned
  into a filesystem path by the CLI.
