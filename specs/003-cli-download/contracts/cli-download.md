# CLI Contract: `microvm download`

## Command surface

```text
microvm download [OPTIONS]

Options:
    --distribution <DISTRIBUTION_ID>
        Select a published distribution. Repeat for multiple distributions.

    --kernel <DISTRIBUTION_ID=KERNEL_ID>
        Select one compatible kernel for a distribution. Repeat once per selected distribution.

    --non-interactive
        Disable prompts and require complete explicit distribution/kernel selections.
```

The command keeps the existing top-level `microvm` help and version behavior. No binary package
selector is exposed: the runtime package is chosen automatically for the host architecture.

## Home resolution

The CLI resolves the home directory using the existing policy:

1. Use `TAUMARU_HOME` when it is set.
2. Otherwise use `~/.taumaru-microvm` from the user's home directory.
3. Pass the resolved path explicitly to `MicroVmSdk::new`.

The CLI does not open or modify the SDK database and does not create a second artifact directory
layout. Home resolution errors are returned before registry selection or transfer.

## Input modes

### Interactive mode

When no explicit selection options are supplied and the terminal supports interactive input:

1. Show a compact discovery status while the SDK lists distributions, kernels, and binary
   packages.
2. Show a keyboard-friendly multi-select containing host-compatible distributions. Each row shows
   display name, version, architecture, image count, and image size where space permits.
3. For each selected distribution, show a separate kernel selector containing only host-compatible
   IDs listed in that distribution's supported-kernel set. The published default is marked with
   text such as `(default)`.
4. Show a review containing every distribution/kernel pair, the selected runtime package and
   version, distribution image count, and estimated total bytes.
5. Offer one specific `Download` confirmation action. Escape or a negative confirmation starts no
   transfer and returns cancellation.

An empty distribution selection is invalid and returns before review or transfer.

### Explicit mode

The following is valid in a terminal or a non-TTY environment:

```text
microvm download \
  --non-interactive \
  --distribution alpine-3.20 \
  --distribution ubuntu-24.04 \
  --kernel alpine-3.20=linux-6.8-x86_64 \
  --kernel ubuntu-24.04=linux-6.8-x86_64
```

Explicit mode must satisfy all of these conditions before any transfer:

- at least one distribution is present;
- distribution IDs are unique and available in the catalog;
- exactly one kernel mapping exists for each selected distribution;
- every mapping uses the `distribution-id=kernel-id` form;
- no mapping names an unselected distribution;
- every kernel exists, matches the host architecture, and is in the distribution's supported set.

If any explicit option is present, the command does not switch to a mixed prompt flow. Partial or
duplicate input is a validation error. In a non-TTY environment, incomplete explicit input is
rejected with the required option form and an example.

## Planning and execution

The command builds one immutable plan from the SDK listing results. It filters distributions and
kernels to the host architecture and selects the highest semantic-version binary package that
contains both `firecracker` and `firectl` components. If no package qualifies, it fails before
confirmation and before any artifact download.

After confirmation, calls are made through the SDK in this order:

1. `download_binary(runtime_package_id, callback)`;
2. `download_kernel(kernel_id, callback)` once for each unique selected kernel;
3. `download_distribution(distribution_id, callback)` once for each selected distribution.

During transfer, the CLI uses the SDK's cancellation-aware variants of these operations with one
shared cancellation token. The existing methods remain the default path for callers that do not
need cancellation.

The CLI forwards SDK callbacks to the renderer. It does not calculate checksums, inspect files,
decide cache reuse, update SQLite, or resolve binary paths. The SDK's returned dispositions are
rendered as `Downloaded`, `Adopted`, or `Already available`.

If the runtime call fails, kernel and distribution calls are not started. After a successful
runtime call, a kernel failure skips only the associated distribution image group; unrelated
groups are reported and attempted in deterministic order. Verified successful work remains
available for a later retry through the SDK. If the operator presses `Ctrl-C` during any SDK
download, the CLI signals the shared cancellation token and waits for the current operation to
clean up; subsequent groups are not started, the SDK removes the partial temporary artifact, and
the command returns cancellation with previously verified work preserved.

## Progress and output

Interactive progress uses a compact aggregate view plus a current-member line. It displays:

- artifact identity and, for package/distribution operations, member identity;
- `Downloading`, `Verifying`, or terminal stage;
- current and expected member bytes;
- aggregate current and expected plan bytes;
- terminal cache disposition.

Progress and diagnostics go to stderr. The final success or failure summary goes to stdout when a
summary is appropriate. Non-TTY output is line-oriented and deterministic; `NO_COLOR` and terminal
color limitations never remove the text stage, byte counters, or outcome labels. Narrow terminals
use compact rows and may truncate display names, but retain stable IDs and state labels.

Failure messages contain:

1. what happened;
2. why the requested preparation is incomplete;
3. what the operator can do next, usually correcting the selection or retrying the command.

No SDK debug output, panic, or stack trace is emitted by the CLI.

## Exit status

| Code | Meaning |
|---:|---|
| `0` | Every required runtime file, unique kernel, and selected distribution image was verified by the SDK. |
| `1` | Validation, registry, filesystem, runtime, or partial-download failure; the plan is incomplete. |
| `130` | The operator cancelled an interactive selector, confirmation, or transfer. |

The command never reports success for a plan with an unverified or failed required member.

## Out of scope

- creating, configuring, starting, stopping, rebooting, or deleting a MicroVM;
- opening or changing the SDK's SQLite schema from the CLI;
- authentication or custom registry selection;
- resumable transfers;
- interactive selection of the runtime binary version;
- adding a second cache, checksum, or local-inventory implementation.
