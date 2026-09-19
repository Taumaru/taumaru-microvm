# Research: CLI `new` Command for Guided MicroVM Creation

## Scope

This feature adds a CLI-only guided creation flow (`microvm new`) plus one minimal
additive SDK read-only query for the per-image downloaded marker. All acquisition,
integrity, cache, persistence, compatibility, and lifecycle behavior is reused
unchanged: `list_*`, `download_*_with_cancellation`, `resolve_binary`,
`create_microvm` with its progress observer, and the existing SQLite inventory.
No new workspace dependencies, no schema migration, no lifecycle semantics change.

## Decisions

### 1. Command surface: `microvm new [NAME]` with per-value flags

Decision: add a top-level `Command::New(NewArgs)`:

```text
microvm new [NAME] [OPTIONS]

Positional:
    [NAME]                  Machine name; skips the name prompt when supplied.

Options:
    --name <NAME>           Equivalent to the positional name; must agree when both given.
    --image <DISTRIBUTION=IMAGE>
        Exactly one image selection, same scoped form as `artifacts download`.
    --disk-gb <GB>          Positive decimal gigabytes (e.g. 20, 20.5).
    --memory <MEM>          Decimal megabytes or gigabytes (e.g. 512MB, 1.5GB).
    --vcpus <N>             Positive integer vCPU count.
    --expose-lan            Opt in to LAN exposure; absent means host-only.
    --non-interactive       Disable all prompts; require the complete explicit set.
```

Explicit-mode trigger mirrors the download command: any explicit creation flag or
`--non-interactive` disables fallback prompts for the supplied values; in an
interactive terminal only missing values are prompted. `--non-interactive`
requires the full set (name, `--image`, `--disk-gb`, `--memory`, `--vcpus`) and
skips the confirmation step. No `--kernel`, `--volume-path`, or `--lan-address`
flags exist; out-of-scope requests are rejected by argument parsing, not by
runtime errors.

Alternatives considered:

- Separate `--distribution` + `--image` flags. Rejected by clarification: the
  scoped `DISTRIBUTION=IMAGE` form is unambiguous (image IDs are unique per
  distribution, not globally) and reuses the parsing operators already know.
- A `--network host-only|lan` value flag. Rejected by clarification: absent
  `--expose-lan` already means host-only, so no extra flag keeps the contract
  minimal.
- A negative `--no-expose-lan` flag. Rejected by clarification for the same
  reason; scripts pin host-only by omitting the flag.

### 2. One additive SDK read-only query for the image marker

Decision: add a single public read-only SDK method that reports whether the
selected image is already verified locally, reusing the existing private
`resolve_distribution_image` plus file-integrity recheck path:

```rust
impl MicroVmSdk {
    /// Returns true when the image is verified locally, false when absent or stale.
    pub async fn is_distribution_image_ready(
        &self,
        distribution_id: &str,
        image_id: &str,
    ) -> Result<bool, SdkError>;
}
```

Semantics: `Ok(true)` only when the inventory relationship exists and the file
is present with matching size and digest; `Ok(false)` for missing, incomplete,
or stale entries (same conditions that make creation return
`ArtifactPrerequisite`); `Err` for transport, filesystem, SQLite, and other
unexpected failures, which abort the flow rather than rendering a marker.
The method is silent (no stdout/stderr, logging, or global state), panic-free,
and performs no download, no mutation, and no cache repair. Kernel and runtime
readiness stay silent during provisioning per the spec assumptions, so no
kernel/runtime query is added; runtime bundle verification already has the
public `resolve_binary` precedent this method mirrors.

Alternatives considered:

- Derive the marker in the CLI by inspecting `<home>/artifacts/...` paths
  directly. Rejected: duplicates SDK-owned layout knowledge in the CLI and
  violates the home/database ownership boundary; paths are SDK internals.
- Call `download_distribution_image` speculatively and read its disposition.
  Rejected: starts a transfer before confirmation and confuses "check" with
  "acquire"; the marker must be side-effect free.
- Expose full `resolve_kernel` / `resolve_distribution_image` public methods
  returning paths. Rejected as oversized: the CLI only needs a boolean per
  image, and returning paths invites CLI path handling the constitution
  forbids. The boolean keeps the new surface minimal.
- No SDK change; show no marker. Rejected: contradicts FR-005, which requires
  the downloaded/needs-download marker on every image row.

### 3. Reuse the download catalog instead of duplicating it

Decision: reuse the registry-catalog logic (`host_arch` gating, compatible
image flattening sorted by `(distribution.id, image.id)`, `default_kernel`
resolution, highest-version runtime-bundle selection, `DISTRIBUTION=IMAGE`
parsing, checked byte totals) by widening the visibility of the existing items
in `commands/download.rs` to `pub(crate)` and importing them from the new
`commands/new.rs`. No behavior change to the download flow; the new command
calls `load_catalog`, `build_plan`-equivalents for exactly one image, and the
same `select_runtime_packages` policy.

Alternatives considered:

- Copy the catalog code into `new.rs`. Rejected: second implementation of
  compatibility and runtime-selection rules that would drift; the constitution
  forbids duplicating the engine-adjacent logic.
- Extract a shared `commands/catalog.rs` module now. Deferred: cleaner
  long-term, but a move touches the download module's 2000+ lines and its
  unit-test imports for no behavioral gain. Visibility widening reuses the
  code with a minimal diff; extraction remains a safe follow-up refactor.

### 4. `inquire` prompts with abort-on-invalid, no re-prompt loop

Decision: keep the existing `inquire` 0.9.x prompts and the download render
config (`prompt_render_config` reused): `Text` for name, disk, memory, and
vCPU; single `Select` for the image; `Confirm` (default `false`) for LAN
exposure; `Confirm` (default `false`) for the final creation question. Each
prompt is attempted once: invalid input aborts with an actionable error
showing the rule and minimum, per clarification. No retry loop, no default
value guessing. Explicitly supplied values are validated with the identical
functions as prompted values.

Prompt order is fixed: name → image → disk → memory → vCPUs → LAN exposure →
confirmation. The catalog loads before the image selector (registry failures
abort before any prompt except possibly the name, which is collected first per
FR-001/FR-004 ordering).

Alternatives considered:

- Re-prompt until valid (or up to N times). Rejected by clarification (option
  B): abort keeps interactive and scripted validation identical and avoids a
  session stuck on a typo; the error restates the rule so retry is one command
  away.
- `dialoguer` or raw `crossterm` handling. Rejected: larger surface and owned
  terminal lifecycle for no gain; `inquire` already handles TTY detection,
  cancellation mapping (`prompt_error` → `CliError::Prompt("cancelled")`),
  and deterministic non-TTY refusal.

### 5. Input parsing and unit conversion rules

Decision: pure, total parsing functions in `commands/new.rs`, unit-tested
without the network or terminal:

- Name: non-empty, 1–64 ASCII, starts alphanumeric, rest alphanumeric/`-`/`_`
  (mirrors the SDK `validate_vm_name` rule for early abort; the SDK remains
  authoritative and its typed error is surfaced unchanged on mismatch).
- Disk (`--disk-gb`, prompt): trimmed decimal `f64`, finite, greater than
  zero; bytes = ceiling of value × 1024³ with overflow check; must be at
  least the selected image's registry `size_bytes`; minimum displayed back in
  GB.
- Memory (`--memory`, prompt): trimmed case-insensitive `<decimal><unit>`
  with optional space, unit `MB` or `GB` only; bytes = ceiling of amount ×
  1024² (MB) or 1024³ (GB) with overflow check; must be at least
  `minimum_memory_bytes(distribution.requirements.min_memory_mb)`; minimum
  displayed in MB/GB; `1.5GB` accepted.
- vCPUs (`--vcpus`, prompt): trimmed `u32`, greater than zero; must be at
  least `distribution.requirements.min_vcpus`.
- Image (`--image`): exactly one `DISTRIBUTION=IMAGE` value; both sides
  non-blank; resolved against the same catalog snapshot as the selector.

All parsing rejects `NaN`, infinities, negative values, empty strings, and
unknown units before any transfer. Byte math uses checked arithmetic;
overflow is a validation error, never a wrap.

Alternatives considered:

- Integer-only memory. Rejected by clarification: `1.5GB` is a reasonable
  size and disk already accepts decimals; consistency wins.
- Accepting `KB`/`TB`/`KiB` forms. Rejected: spec fixes the vocabulary to
  GB for disk and `xMB`/`xGB` for memory; extra units widen the contract and
  its tests for no requested value.

### 6. Provisioning order and creation invocation

Decision: after confirmation (or non-interactive validation), execute
strictly in dependency order through the cancellation-aware SDK variants with
one shared `DownloadCancellation`: runtime packages (sorted) →
default kernel → selected image → `create_microvm`. A runtime failure stops
all dependent work; kernel/image failures report which prerequisite failed
and create no VM. Only after every prerequisite returns verified does the
command call:

```rust
CreateMicroVmRequest {
    name,
    distribution_id,   // parent of the selected image
    image_id,          // the selected image
    disk_size_bytes,   // converted GB value
    vcpu_count,
    memory_bytes,      // converted MB/GB value
    expose_on_lan,     // from Confirm or --expose-lan
    lan_address: None, // automatic addressing; out of scope otherwise
    volume_path: None, // managed default; out of scope otherwise
}
```

with `Some(observer)` mapping `CreationProgress` events into the renderer.
`AlreadyConfigured` (identical repeat) reports the existing VM without
duplicate work; `ConfigurationConflict` / `LifecycleConflict` (same name,
different settings) reports the conflict and leaves the existing VM
unchanged. No download plan table is rendered at any point.

Alternatives considered:

- Provisioning kernel/image before the runtime bundle. Rejected: creation
  prerequisites resolve the runtime pair too, and the download command's
  established order (runtime first) keeps failure semantics consistent
  across commands.
- Calling the non-cancellation download variants. Rejected: loses the
  controlled Ctrl-C path the download command already proves; the shared
  token plus `call_with_signal` is reused unchanged.

### 7. Progress presentation: sequential truthful phases, no invented aggregate

Decision: reuse the existing `indicatif` aggregate renderer for provisioning
(total = runtime + kernel + image expected bytes from registry sizes, with
`ProgressForwarder`/`normalize_progress` unchanged), then drive a second
percent bar from `CreationProgress::overall_percent` (0–100) with step
labels from `CreationStage` display strings plus phase and outcome. Step rows
for each prerequisite member and each creation stage are emitted as text
lines (identity, stage, byte counters where applicable, disposition or
outcome) so non-TTY, `NO_COLOR`, and narrow terminals stay fully
informative. No cross-phase aggregate percent is invented: the two phases
are sequential bars, each truthful within its own totals. New formatting
helpers (`format_new_review`, `format_new_progress_line`,
`format_new_summary`) live in `output/human.rs` beside the download ones and
share `paint`, `divider`, and `format_bytes`.

Alternatives considered:

- One combined 0–100 percent across provisioning and creation. Rejected:
  requires inventing a weight for byte work versus creation stages; any
  weight is a fabricated value the spec forbids.
- Reusing the download review/summary writers verbatim. Rejected: they
  render a multi-image plan table and group outcomes the spec explicitly
  excludes ("no download plan shown"); the creation summary (identity,
  network address, volume, SSH reference) has no download equivalent.

### 8. Interruption semantics split by phase

Decision: reuse `call_with_signal` + shared token for provisioning exactly
like the download command (cancel in-flight transfer, remove the partial
file via the SDK, preserve verified prerequisites, report affected groups,
exit `130`). Prompts and confirmation map `inquire` cancellation to
`CliError::Prompt("cancelled")` (exit `130`, no side effects). During the
`create_microvm` call itself there is no cancellation token: a Ctrl-C sets an
interrupted flag while the command awaits the operation to settle, then
reports the creation outcome (success with its VM summary, or the typed
failure with rollback intact) plus the interruption note. Exit status follows
the creation result, never a fabricated cancel: a configured VM is reported
as configured even if the operator pressed Ctrl-C mid-creation.

Alternatives considered:

- Cancelling creation mid-flight and returning `130`. Rejected: creation
  has no cooperative cancel in the SDK; abandoning the await would orphan
  the operation and risk claiming "cancelled" while a VM was configured or
  rolled back unseen.
- Ignoring Ctrl-C during creation entirely. Rejected: the operator gets no
  feedback that the keypress registered; the flag-and-report approach
  acknowledges the signal without lying about the outcome.

### 9. Error presentation: command-scoped messages, download text untouched

Decision: extend `CliError` with new-command-aware messaging without changing
any existing download string (the `error.rs` unit test pins them). New
validation failures render as "MicroVM creation input is invalid" with the
rule/minimum as the reason and a next step pointing at the specific flag or
prompt form; new-command cancellation renders as "MicroVM creation
cancelled" with the appropriate next step (`microvm new` retry, not the
download command). SDK errors during provisioning/creation keep the typed
`SdkError` text as the reason with a creation-specific next step (retry
reuses verified prerequisites). All messages keep the three-part
what/why/next shape, text-plus-symbol status, and English-only text.

Alternatives considered:

- Reusing the download wording ("Download selection is invalid", "Run
  `microvm artifacts download` again"). Rejected: wrong command, wrong
  entity, wrong retry guidance; confusing in scripts.
- Generalizing the existing variants' text. Rejected: breaks the pinned
  download messages and their tests; additive variants are cheaper and safer.

### 10. Test strategy: fake clients plus fixture-backed SDK test

Decision: extend the existing seams, no new harnesses. CLI unit tests in
`commands/new.rs` cover name/disk/memory/vCPU parsing and minimums,
`--image` form validation, positional/`--name` agreement, catalog filtering
and single-image resolution, runtime-first ordering, and renderer line
formats — using the SDK-shaped `RecordingClient` pattern plus a recording
creation client fed with synthetic `CreationProgress` streams. SDK tests add
fixture-backed cases for `is_distribution_image_ready` (ready, missing,
stale, error propagation) using the existing `FixtureServer` + manifest.
Command-surface tests assert `microvm new --help` exposes the new flags,
`microvm new --non-interactive` without values fails without prompting, and
the package-boundary test still holds (no new CLI dependencies).

## Repository findings used by the plan

- `MicroVmSdk::create_microvm` (`crates/sdk/src/manager.rs`) takes
  `CreateMicroVmRequest` plus an optional `FnMut(CreationProgress)` observer;
  `CreationProgress` carries `stage`, `completed_steps`/`total_steps` (6),
  `overall_percent`, `phase`, optional byte counters, and optional terminal
  `outcome` — sufficient to drive a percent bar with no extra SDK data.
- `resolve_binary` is the only public local-inventory query; kernel and
  image resolution (`resolve_kernel`, `resolve_distribution_image`) are
  `pub(crate)` repository methods, so the CLI cannot answer "already
  downloaded?" without the new read-only method.
- `create_microvm` already enforces disk ≥ image size, memory ≥ distribution
  minimum, and vCPU ≥ distribution minimum, returning typed errors — CLI
  pre-validation is early UX, the SDK stays authoritative.
- `commands/download.rs` owns the reusable catalog, `call_with_signal`,
  `ProgressForwarder`, and the `RecordingClient` test fake; `output/human.rs`
  owns the renderer, review, and summary writers; `cli.rs`, `commands/mod.rs`,
  `context.rs` (home policy), and `error.rs` (exit codes, `130` on cancel)
  are the only other CLI touch points.
- Fixture manifest publishes test kernels, binary packages, and a
  distribution with servable payloads, so the new SDK query needs no new
  fixture data.
