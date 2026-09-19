# CLI Contract: `microvm artifacts download`

## Command surface

```text
microvm artifacts download [OPTIONS]

Options:
    --image <DISTRIBUTION_ID=IMAGE_ID>
        Select one published image of one distribution. Repeat for multiple images,
        including several images of the same distribution.

    --non-interactive
        Disable prompts and require complete explicit image selections.
```

The previous top-level `microvm download` entry (with `--distribution` and `--kernel`)
is removed, not aliased. Its help text is replaced by image-based preparation help and
migration guidance toward `--image DISTRIBUTION=IMAGE`. No runtime-package selector is
exposed: the runtime bundle is chosen automatically for the host architecture. No
kernel selector or override is exposed: each affected distribution contributes its
published default kernel.

## Home resolution

Unchanged from the existing policy:

1. Use `TAUMARU_HOME` when it is set.
2. Otherwise use `~/.taumaru-microvm` from the user's home directory.
3. Pass the resolved path explicitly to `MicroVmSdk`.

The CLI does not open or modify the SDK database and does not create a second
artifact directory layout. Home resolution errors are returned before registry
selection or transfer.

## Input modes

### Interactive mode

When no explicit `--image` options are supplied and the terminal supports interactive
input:

1. Show a compact discovery status while the SDK lists distributions, kernels, and
   binary packages.
2. Show one keyboard-friendly multi-select containing host-compatible images across
   all host-compatible distributions, sorted by distribution ID then image ID. Each
   row names its parent distribution and shows image name, variant or capabilities,
   and size where space permits. No distribution-first step and no kernel step exist.
3. Show a review containing every selected image (same sort order) with its parent
   distribution, the resolved default kernel per affected distribution, all selected
   runtime packages and versions, and the estimated total bytes of selected artifacts
   only.
4. Offer one specific `Download` confirmation action. Escape, an empty selection, or a
   negative confirmation starts no transfer and returns cancellation.

A repeated pick of the same image deduplicates to one entry before review; it is
never an error and never transfers twice.

### Explicit mode

The following is valid in a terminal or a non-TTY environment:

```text
microvm artifacts download \
  --non-interactive \
  --image alpine-3.20=alpine-3.20-minimal \
  --image ubuntu-24.04=ubuntu-24.04-docker
```

Explicit mode must satisfy all of these before any transfer:

- at least one `--image` is present;
- every value uses the `distribution-id=image-id` form;
- every distribution exists in the catalog snapshot and matches the host;
- every image is published by its named distribution;
- repeated pairs deduplicate (not an error);
- every affected distribution resolves a host-compatible published default kernel;
- runtime packages collectively provide both required components.

If any explicit option is present, the command never falls back to prompts. In a
non-TTY environment, missing explicit selections are rejected with the required flag
form and an example. Use of removed `--distribution`/`--kernel` flags is rejected by
argument parsing with guidance toward `--image`.

## Planning and execution

The command builds one immutable plan from the SDK listing results. It filters
distributions to the host architecture, flattens and sorts images by
`(distribution.id, image.id)`, deduplicates repeats, resolves each affected
distribution's default kernel (validated host-compatible, no override), and selects
the highest semantic-version binary package for each required `firecracker` and
`firectl` file component. A package containing both components is selected once. If
the catalog cannot provide every required component or any default kernel, the
command fails before confirmation and before any artifact download.

After confirmation, calls are made through the SDK in this order:

1. `download_binary(runtime_package_id, callback)` once per selected runtime package
   (sorted);
2. `download_kernel(default_kernel_id, callback)` once per unique resolved default
   kernel (sorted);
3. `download_distribution_image(distribution_id, image_id, callback)` once per
   selected image (sorted by distribution, then image).

The CLI uses the cancellation-aware SDK variants with one shared cancellation token.
The existing whole-distribution SDK methods are not called by this command but remain
available to other consumers.

The CLI forwards SDK callbacks to the renderer. It does not calculate checksums,
inspect files, decide cache reuse, update SQLite, or resolve binary paths. The SDK's
returned dispositions render as `Downloaded`, `Adopted`, or `Already available`.

If any runtime package call fails, kernel and image calls are not started. After all
runtime package calls succeed, a kernel failure skips only its dependent images;
unrelated kernels and images are attempted in plan order. An image failure does not
affect its siblings. Verified successful work remains available for a later retry
through the SDK. If the operator interrupts during any SDK download, the CLI signals
the shared token and waits for cleanup; subsequent groups are not started, the SDK
removes the partial temporary artifact, and the command returns cancellation with
previously verified work preserved.

## Progress and output

Interactive progress uses a compact aggregate view plus a current-member line. It
displays:

- artifact identity: `runtime/{package}[/{file}]`, `kernel/{id}`, or
  `distribution/{distribution}/{image}`;
- `Downloading`, `Verifying`, or terminal stage;
- current and expected member bytes;
- aggregate current and expected plan bytes;
- terminal cache disposition.

Progress and diagnostics go to stderr. The final success or failure summary goes to
stdout when a summary is appropriate. Non-TTY output is line-oriented and
deterministic; `NO_COLOR` and terminal color limitations never remove the text stage,
byte counters, or outcome labels. Narrow terminals use compact rows and may truncate
display names, but retain stable IDs and state labels.

Failure messages contain:

1. what happened;
2. why the requested preparation is incomplete;
3. what the operator can do next, usually correcting the selection or retrying the
   command.

No SDK debug output, panic, or stack trace is emitted by the CLI.

## Exit status

| Code | Meaning |
|---:|---|
| `0` | Every required runtime file, unique resolved default kernel, and selected image was verified by the SDK. |
| `1` | Validation, registry, filesystem, runtime, kernel, or partial-image failure; the plan is incomplete. |
| `130` | The operator cancelled an interactive selector, confirmation, or transfer. |

The command never reports success for a plan with an unverified or failed required
member. Unselected images of an affected distribution are never required.

## Out of scope

- creating, configuring, starting, stopping, rebooting, or deleting a MicroVM;
- opening or changing the SDK's SQLite schema from the CLI;
- authentication or custom registry selection;
- resumable transfers;
- interactive selection of the runtime binary version;
- custom kernel selection or override;
- adding a second cache, checksum, or local-inventory implementation.
