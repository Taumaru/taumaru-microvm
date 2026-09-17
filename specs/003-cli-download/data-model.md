# Data Model: CLI Artifact Download

## Persistence boundary

This feature adds no SQLite tables and no migration. The CLI does not open the database directly.
It passes the resolved home path to `MicroVmSdk`, which owns the existing normalized inventory and
its migration lifecycle.

The SDK remains responsible for durable records such as:

- physical verified files and their absolute/relative paths, expected and actual size, SHA-256,
  verification status, and timestamps;
- kernel records and their download relation;
- binary packages, binary components, and their download relations;
- distributions, distribution images, boot metadata, and image download relations;
- distribution-to-kernel compatibility and default-kernel relationships.

The CLI's model is an ephemeral plan and result model. It exists only for one command invocation,
so a cancelled or failed plan never creates a second source of truth about installed artifacts.

## In-memory entities

### `RegistryCatalog`

The immutable data used to create a plan.

| Field | Type | Meaning |
|---|---|---|
| `kernels` | `Vec<Kernel>` | Current SDK registry listing |
| `binaries` | `Vec<BinaryPackage>` | Current SDK registry listing |
| `distributions` | `Vec<Distribution>` | Current SDK registry listing |
| `host_architecture` | `Architecture` | Architecture selected from the running host |
| `interactive` | `bool` | Whether prompt rendering is available |

The catalog is not persisted and must not contain a second copy of downloaded paths or checksums.
The SDK registry types remain the authoritative metadata types.

### `DistributionSelection`

One operator-selected distribution and its required kernel choice.

| Field | Type | Meaning |
|---|---|---|
| `distribution` | `Distribution` | Host-compatible published distribution |
| `kernel` | `Kernel` | Exactly one selected compatible kernel |
| `kernel_is_default` | `bool` | Whether the selected kernel is the distribution's published default |
| `image_count` | `usize` | Number of images the SDK will acquire for the distribution |
| `image_bytes` | `u64` | Checked sum of all image sizes |

The same kernel may appear in multiple selections. It is retained in every relationship but is
deduplicated in the execution members.

### `RuntimeBinarySelection`

The automatically selected host runtime packages. Each required component is resolved separately;
one package is reused when it provides both components, while split registry packages are retained
as separate runtime selections.

| Field | Type | Meaning |
|---|---|---|
| `packages` | `Vec<BinaryPackage>` | Highest valid host-compatible package for each required component, deduplicated by package ID |
| `required_components` | `Vec<String>` | `firecracker` and `firectl` |
| `file_count` | `usize` | All files the SDK will acquire from all selected packages |
| `expected_bytes` | `u64` | Checked sum of all selected package file sizes |

Additional package files are included in the count and byte estimate because
`download_binary` operates on complete packages. Runtime package IDs are sorted before they are
added to the execution plan.

### `DownloadPlan`

The immutable plan created before confirmation and executed only after confirmation.

| Field | Type | Meaning |
|---|---|---|
| `runtime` | `RuntimeBinarySelection` | First execution stage |
| `selections` | `Vec<DistributionSelection>` | One row per selected distribution, deterministic ID order |
| `unique_kernels` | `Vec<Kernel>` | Deduplicated selected kernels, deterministic ID order |
| `expected_bytes` | `u64` | Runtime files + unique kernels + all selected images |
| `expected_members` | `usize` | Physical file count represented by the plan |
| `execution_order` | `Vec<PlanMember>` | Explicit runtime, kernel, and distribution group order |

Plan construction uses checked arithmetic. Overflow, duplicate explicit IDs, empty selections,
missing relationships, incompatible architectures, unavailable kernels, and missing runtime
components are validation failures before confirmation.

### `PlanMember`

The executable group in the plan.

```text
RuntimeBinary { package_id }
Kernel { kernel_id }
DistributionImages { distribution_id }
```

Runtime is always the first member. Kernel members contain each unique kernel once. Distribution
members contain one distribution each; the SDK expands the distribution into all published images.

### `DownloadProgressView`

The renderer's normalized view of an SDK callback.

| Field | Type | Meaning |
|---|---|---|
| `member_label` | `String` | Human-readable current package, kernel, image, or distribution label |
| `stage` | `ProgressStage` | `Downloading`, `Verifying`, `Cancelled`, or another terminal stage |
| `current_bytes` | `u64` | Current member bytes reported by the SDK |
| `member_total_bytes` | `u64` | Current member expected bytes |
| `plan_completed_bytes` | `u64` | Expected bytes for verified members completed before this SDK operation |
| `plan_current_bytes` | `u64` | `plan_completed_bytes` plus the SDK operation aggregate |
| `plan_total_bytes` | `u64` | `DownloadPlan.expected_bytes` |
| `disposition` | `Option<DispositionLabel>` | `Downloaded`, `Adopted`, or `Already available` for terminal events |

The CLI derives aggregate progress only from SDK counters and plan metadata. It must not advance a
bar based on elapsed time or assumed network rate.

### `MemberOutcome`

The result of one plan member.

```text
Succeeded { label, disposition, verified_members, verified_bytes }
Failed { label, error, retained_verified_members }
```

An SDK operation may return a failure after earlier files in the same package or distribution were
verified. Those successful files remain represented in the outcome summary as retained work; the
CLI does not delete them or attempt to reconstruct their inventory.

### `DownloadOutcome`

The command-level terminal result.

```text
Succeeded { members, verified_bytes }
PartiallyFailed { successes, failures, retryable: true }
FailedBeforeTransfer { reason }
Cancelled { stage }
```

`Succeeded` is possible only when every runtime package, every unique kernel, and every selected
distribution image group return verified SDK results. Any failed required member produces a
non-success outcome. `FailedBeforeTransfer` covers invalid input, unavailable registry data, no
compatible package for a required runtime component, and empty or incomplete selection. `Cancelled` covers prompt or
confirmation cancellation before transfer or controlled `Ctrl-C` cancellation during transfer.
Transfer cancellation stops subsequent plan members, preserves previously verified groups, and
must leave no partial published artifact.

## State transitions

```text
Discovering
    -> SelectingDistributions
    -> SelectingKernels
    -> Reviewing
    -> AwaitingConfirmation
    -> DownloadingRuntime
    -> DownloadingKernels
    -> DownloadingDistributions
    -> Succeeded

Any pre-transfer state -> Cancelled | FailedBeforeTransfer
DownloadingRuntime -> FailedBeforeTransfer/Failed (runtime failure; no dependent downloads)
DownloadingKernels or DownloadingDistributions -> Cancelled | PartiallyFailed | Succeeded
```

Transition rules:

1. `Discovering` must complete all required SDK list calls before a selector or explicit-plan
   validation runs.
2. `SelectingDistributions` cannot transition to `SelectingKernels` with an empty selection.
3. `SelectingKernels` must produce exactly one compatible kernel for every selected distribution.
4. `Reviewing` computes all counts and checked byte totals before confirmation is offered.
5. No `Downloading*` state is reachable until confirmation succeeds.
6. A runtime package failure is terminal for the transfer phase because all later work depends on
   the complete runtime stage.
7. Kernel and distribution groups after a successful runtime stage are attempted in plan order;
   a kernel failure skips only the associated distribution image group, while unrelated groups
   continue and verified work is not erased.
8. A transfer cancellation signals the SDK, waits for the current operation's cleanup, prevents
   subsequent group execution, retains successful earlier groups, and removes the unverified
   temporary artifact.
9. Only a fully verified plan transitions to `Succeeded`.

## Validation invariants

- Every selected distribution and kernel is present in the same catalog snapshot.
- Every selected distribution matches the host architecture.
- Every selected kernel matches the host architecture and appears in the distribution's published
  supported-kernel IDs.
- A selected kernel mapping cannot name an unselected distribution or appear more than once.
- The runtime packages match the host architecture and collectively provide both required
  components; a package containing both is selected only once.
- Every registry ID is displayed and executed as data; it is never turned into a filesystem path by
  the CLI.
- Shared kernels are deduplicated by registry ID only within the current plan; the SDK still
  decides whether the physical file is downloaded, adopted, or skipped.
- A selection is not reported ready merely because a local path exists. Readiness comes only from a
  successful SDK result that includes integrity and inventory decisions.
