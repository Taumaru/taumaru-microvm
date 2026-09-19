# CLI Contract: `microvm new`

## Command surface

```text
microvm new [NAME] [OPTIONS]

Positional:
    [NAME]                  Machine name; skips the name prompt when supplied.

Options:
    --name <NAME>           Equivalent to the positional name; must agree when both given.
    --image <DISTRIBUTION=IMAGE>
        Select the single published image; exactly one is required in non-interactive mode.
    --disk-gb <GB>          Positive decimal gigabytes (e.g. 20, 20.5).
    --memory <MEM>          Decimal megabytes or gigabytes (e.g. 512MB, 1.5GB).
    --vcpus <N>             Positive integer vCPU count.
    --expose-lan            Opt in to LAN exposure; absent means host-only.
    --non-interactive       Disable prompts and require the complete explicit set.
```

No `--kernel`, `--volume-path`, or `--lan-address` flags exist. No home
directory flag exists: home resolution follows the existing policy only. No
download plan flag exists: no plan is ever shown.

## Home resolution

Unchanged from the existing policy:

1. Use `TAUMARU_HOME` when it is set.
2. Otherwise use `~/.taumaru-microvm` from the user's home directory.
3. Pass the resolved path explicitly to `MicroVmSdk`.

The CLI does not open or modify the SDK database and does not create a
second artifact directory layout. Home resolution errors are returned before
registry selection or transfer.

## Input modes

### Interactive mode

When the terminal supports interactive input, values are collected in this
fixed order, prompting only for values not already supplied as arguments:

1. Machine name (skipped when the positional name or `--name` is present).
2. Single image select: one keyboard-friendly list of host-compatible
   images across host-compatible distributions, sorted by distribution ID
   then image ID. Each row names its parent distribution and shows image
   name, variant or capabilities, size, and a text marker (`downloaded` or
   `needs download`).
3. Disk size in GB (decimal allowed).
4. Memory as `xMB`/`xGB` (case-insensitive, optional space, decimals
   allowed, e.g. `512MB`, `1.5GB`).
5. vCPU count (positive integer).
6. LAN exposure confirmation (default no, host-only).
7. Final creation confirmation summarizing the VM name, selected image with
   its parent distribution, disk, memory, vCPU count, network mode, and
   which prerequisites still need fetching.

Invalid input at any prompt aborts with an actionable error showing the rule
and minimum; there is no re-prompt loop. Escape, an empty image selection,
or a negative confirmation starts no transfer, creates no VM, and returns
cancellation.

Supplied values are validated with the identical rules as prompted values.
A positional name and `--name` that disagree abort before any transfer.

### Explicit mode

The following is valid in a terminal or a non-TTY environment:

```text
microvm new web-01 \
  --non-interactive \
  --image ubuntu-24.04=ubuntu-24.04-docker \
  --disk-gb 20 \
  --memory 2GB \
  --vcpus 2
```

Add `--expose-lan` only for LAN exposure; its absence means host-only.
The positional name and `--name` are interchangeable; when both are present
they must agree.

Explicit mode must satisfy all of these before any transfer:

- a name is present (positional or `--name`) and path-friendly;
- exactly one `--image` is present in `distribution-id=image-id` form;
- `--disk-gb`, `--memory`, and `--vcpus` are present and parse;
- the distribution exists in the catalog snapshot and matches the host;
- the image is published by its named distribution;
- the distribution resolves a host-compatible published default kernel;
- converted disk bytes are at least the image registry size;
- converted memory bytes are at least the distribution minimum;
- vCPUs are at least the distribution minimum;
- runtime packages collectively provide both required components.

If any explicit option or `--non-interactive` is present, the command never
falls back to prompts for the supplied values. In a non-TTY environment,
missing explicit values are rejected with the required flag form and an
example. The command performs no prompts at all in `--non-interactive` mode
and reports the first missing or invalid value with usage guidance.

## Collection and execution

The command builds one immutable request from the SDK listing results. It
filters distributions to the host architecture, resolves the single image
pair, resolves the affected distribution's default kernel (validated
host-compatible, no override), and selects the highest semantic-version
binary package for each required `firecracker` and `firectl` file component.
A package containing both components is selected once. If the catalog cannot
provide every required component or the default kernel, the command fails
before confirmation and before any artifact transfer.

After confirmation, calls are made through the SDK in this order:

1. `download_binary(runtime_package_id, callback)` once per selected runtime
   package (sorted) — cancellation-aware variant with the shared token;
2. `download_kernel(default_kernel_id, callback)` once — cancellation-aware
   variant with the shared token;
3. `download_distribution_image(distribution_id, image_id, callback)` once —
   cancellation-aware variant with the shared token;
4. `create_microvm(request, Some(observer))` once, with `lan_address: None`
   and `volume_path: None`.

The CLI uses the cancellation-aware SDK download variants with one shared
cancellation token for steps 1–3. Step 4 has no cancellation token: a Ctrl-C
during creation sets an interrupt flag while the command awaits the
operation to settle, then reports the settled outcome plus the interruption
note.

The CLI forwards SDK callbacks to the renderer. It does not calculate
checksums, inspect files, decide cache reuse, update SQLite, or resolve
binary paths. The SDK's returned dispositions render as `Downloaded`,
`Adopted`, or `Already available`.

If any runtime package call fails, kernel, image, and creation calls are not
started. A kernel or image failure creates no VM. An identical repeat of an
already-configured VM reports the existing VM without duplicate work; a name
conflict with different settings reports the conflict and leaves the
existing VM unchanged. Verified successful work remains available for a later
retry through the SDK. If the operator interrupts during any SDK download,
the CLI signals the shared token and waits for cleanup; subsequent members
are not started, the SDK removes the partial temporary artifact, and the
command returns cancellation with previously verified work preserved.

No download plan table is rendered at any point.

## Progress and output

Provisioning progress uses a compact aggregate view plus a current-member
line. Creation progress uses a percent bar driven by `overall_percent` plus
a current-stage line. Displayed content:

- artifact identity: `runtime/{package}[/{file}]`, `kernel/{id}`,
  `image/{distribution}/{image}`, or `creation/{stage}`;
- `Downloading`, `Verifying`, `Started`, `InProgress`, `Finished`, or
  terminal stage;
- current and expected member bytes where applicable;
- aggregate current and expected provisioning bytes during provisioning;
- step counters (`N/6`) and outcome (`Completed`, `AlreadyConfigured`,
  `Failed`) during creation;
- terminal cache disposition for reused prerequisites.

Progress and diagnostics go to stderr. The final creation summary goes to
stdout: identity, network mode with address, volume location, resource
sizes, and SSH connection reference (account, port, private-key path; never
key contents). Non-TTY output is line-oriented and deterministic; `NO_COLOR`
and terminal color limitations never remove the text stage, byte counters,
or outcome labels. Narrow terminals use compact rows and may truncate
display names, but retain stable IDs and state labels.

Failure messages contain:

1. what happened;
2. why the VM was not created;
3. what the operator can do next, usually correcting the value or retrying
   the command (verified prerequisites are reused).

No SDK debug output, panic, or stack trace is emitted by the CLI.

## Exit status

| Code | Meaning |
|---:|---|
| `0` | The VM was created (or the identical VM was already configured) and the summary was reported. |
| `1` | Validation, registry, filesystem, provisioning, creation, or conflict failure; no new VM was configured by this invocation. |
| `130` | The operator cancelled a prompt, the confirmation, or provisioning. |

The command never reports success for an invocation whose prerequisites are
not all verified or whose creation did not return a configured VM. A
creation-phase interrupt follows the settled creation outcome, never a
fabricated cancellation.

## Out of scope

- kernel override, custom volume path, explicit LAN address, multiple
  images;
- starting, stopping, rebooting, connecting to, or deleting a MicroVM;
- opening or changing the SDK's SQLite schema from the CLI;
- authentication or custom registry selection;
- resumable transfers;
- interactive selection of the runtime binary version;
- adding a second cache, checksum, or local-inventory implementation.
