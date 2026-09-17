# Research: CLI Artifact Download

## Scope

This research covers the presentation and orchestration needed for the `microvm download`
command. The existing SDK remains the source of truth for registry access, artifact validation,
cache reconciliation, SHA-256 verification, SQLite persistence, and download progress callbacks.
The attached `Design System.md` is treated as a visual and interaction reference only; it does
not introduce web UI, branding assets, or unrelated product behavior into this feature.

## Decisions

### 1. Use `inquire` for the interactive selectors

Decision: use `inquire` for the distribution `MultiSelect`, per-distribution kernel `Select`,
confirmation, keyboard help, cancellation, and validation behavior.

The crate provides the interaction primitives needed by this feature without making the CLI own
raw terminal modes or cursor cleanup. The distribution list can use a compact custom item
formatter and a bounded page size, while the kernel selector can expose the compatibility and
default markers in the item text. Prompt cancellation maps to the command's cancellation result
before any transfer begins.

Reference: [`inquire` documentation](https://docs.rs/inquire/latest/inquire/).

Alternatives considered:

- Direct `crossterm` input handling would provide more control but would require the command to
  own raw-mode lifecycle, focus state, resize behavior, and cleanup.
- A full-screen `ratatui` interface would be a larger surface than the two selectors and review
  step require, and would make narrow or non-interactive terminal behavior harder to keep
  deterministic.
- `dialoguer` would cover basic prompts, but the planned filtering, compact item rendering, and
  validation behavior fit `inquire` more directly.

The CLI will not add a direct `crossterm` dependency. `inquire` owns prompt terminal mechanics;
the command only uses standard terminal detection for deciding whether prompts are available.

### 2. Use `indicatif` for truthful progress rendering

Decision: use `indicatif` for one aggregate progress view and one current-member status line.
The renderer receives the SDK's `DownloadProgress` events and never invents byte counts or
percentages. A terminal draw target is used only when progress can be rendered; non-TTY output
uses deterministic status lines with the same stage and byte fields.

The SDK emits both per-member and per-operation aggregate counters. The CLI adds the expected
sizes of members already completed in the current plan to the current operation's counters so the
aggregate view represents the whole plan. Cache terminal events are rendered as `Adopted` or
`Already available`, not as a fresh transfer.

Reference: [`indicatif` progress documentation](https://docs.rs/indicatif/latest/indicatif/struct.ProgressBar.html).

Alternatives considered:

- A hand-written carriage-return renderer would duplicate terminal-width, refresh throttling,
  hidden-output, and cleanup behavior.
- A spinner-only design would hide the exact byte progress required for large infrastructure
  artifacts and would violate the design requirement for immediate, truthful feedback.

### 3. Keep the command testable through an SDK-shaped download boundary

Decision: separate pure planning from execution and keep execution generic over a small internal
artifact client boundary. Production wiring adapts `MicroVmSdk`; unit tests provide a deterministic
fake that records list and download calls and emits controlled progress events.

The boundary is intentionally CLI-internal. It does not create a second artifact engine, cache
policy, checksum implementation, or database repository. It exists only so selection, plan
validation, execution ordering, failure continuation, and output formatting can be tested without
network access or a real terminal.

The production adapter will call the existing SDK operations:

- `list_distributions`, `list_kernels`, and `list_binaries` for one catalog snapshot;
- `download_binary` for the selected runtime package;
- `download_kernel` once per unique selected kernel;
- `download_distribution` once per selected distribution.

All returned `Downloaded*` values and `DownloadProgress` events remain owned by the SDK contract.

### 4. Select runtime binaries deterministically from host-compatible metadata

Decision: filter binary packages to the host architecture, require both `firecracker` and
`firectl` file components, and choose the highest valid semantic version. Ties are resolved by
ascending registry ID. A candidate with an unparseable version is not eligible for automatic
selection; if no valid candidate remains, the command fails before confirmation and transfer.

The host architecture is mapped from `std::env::consts::ARCH` to the SDK's `Architecture` enum.
The selection algorithm is pure and receives the binary collection, so it can be tested for
architecture filtering, required components, version ordering, and deterministic ties.

The complete selected package is passed to the SDK. If it contains additional files such as
`jailer`, those files are downloaded as part of the package because the SDK's package operation
owns package completeness and persistence.

The `semver` crate is used only for ordering published package versions; it does not become part
of the SDK public API. Reference: [`semver` documentation](https://docs.rs/semver/latest/semver/).

### 5. Treat the registry lists as an immutable planning snapshot

Decision: fetch each existing SDK collection once before selection, combine them into an in-memory
catalog, validate compatibility locally, and build a plan from that snapshot. The CLI never
opens the SDK database or reimplements registry validation.

The SDK download operation may fetch the manifest again when execution starts. If an artifact has
disappeared or changed between planning and execution, the SDK's typed error is surfaced as an
actionable CLI failure. The CLI does not silently substitute another distribution, kernel, or
binary package after confirmation.

### 6. Preserve SDK ownership of cache and integrity behavior

Decision: the CLI passes explicit IDs and progress callbacks to the SDK and renders the returned
disposition. It does not check file existence, calculate SHA-256, compare sizes, remove invalid
files, update inventory rows, or resolve binary paths itself.

The SDK already implements the required three-way rule:

| Physical file | Complete SDK inventory relationship | CLI-visible result |
|---|---|---|
| Missing or invalid | Present or absent | SDK replaces it and verifies before returning |
| Valid | Missing or incomplete | SDK adopts it without transfer |
| Valid | Complete | SDK skips it without transfer |

This keeps the CLI thin and ensures future SDK consumers receive exactly the same behavior.

### 7. Execute in a deterministic dependency order

Decision: after confirmation, run every selected runtime package first, then each unique kernel in
ascending registry ID order, then selected distributions in ascending distribution ID order. The
SDK retains registry order for files inside a package or distribution.

The complete runtime stage is a prerequisite for all dependent preparation, so any runtime package
failure stops the remaining artifact operations. After the runtime stage, a kernel is a
prerequisite for its associated distribution image group: a kernel failure skips only that image
group. Unrelated kernels and distributions are attempted sequentially. The final outcome is
non-success whenever any required group fails or is skipped, while verified work remains available
for a later retry through the SDK inventory.

### 8. Use two explicit CLI input modes

Decision: the command accepts repeatable explicit selections for automation:

```text
microvm download \
  --non-interactive \
  --distribution <distribution-id> \
  --kernel <distribution-id>=<kernel-id>
```

The `--distribution` and `--kernel` options may be repeated. Explicit selections always bypass
prompts and must be complete: every selected distribution has exactly one mapping, every mapping
belongs to a selected distribution, and every kernel is published as compatible with that
distribution and the host. `--non-interactive` makes the intent explicit and rejects missing
selections immediately. In a non-TTY environment, complete explicit selections are required even
when the flag is omitted.

With no explicit selections in an interactive terminal, the command uses the multi-select and
per-distribution kernel selectors. Partial explicit input is rejected rather than falling back to
a mixed prompt flow, which keeps automation deterministic and prevents an accidental transfer
from an incomplete plan.

### 9. Keep terminal output calm, compact, and accessible

Decision: follow the attached Design System's Zinc/neutral baseline through hierarchy, spacing,
text labels, and restrained semantic emphasis rather than adding a saturated brand palette.

The interaction has four visible stages:

1. Discovery status while the three SDK lists are loaded.
2. Selection prompts with visible focus, selection marks, and a default-kernel marker.
3. A compact review containing distribution/kernel pairs, runtime packages, image count, and total
   expected bytes, followed by one specific `Download` confirmation action.
4. A progress/result view with explicit labels such as `Downloading`, `Verifying`, `Downloaded`,
   `Adopted`, `Already available`, and `Failed`.

Progress and diagnostics are written to stderr so a successful summary on stdout remains usable by
scripts. Non-TTY or `NO_COLOR` output uses plain text and symbols; color is never the only state
signal. Narrow terminals receive one compact row per selection and truncated or wrapped labels,
not a wide table that hides the artifact identity.

### 10. Define cancellation and failure scope explicitly

Decision: selector, confirmation, and transfer cancellation are first-class CLI outcomes. The
CLI listens for `Ctrl-C` while awaiting each SDK download operation, signals cooperative
cancellation, waits for the SDK to finish cleanup, does not start subsequent plan members,
preserves already verified groups, and returns exit code `130` with a success/failure/cancelled
summary.

Because the current SDK methods do not expose cancellation yet, implementation will add a
`DownloadCancellation` token and cancellation-aware `download_*_with_cancellation` variants while
keeping the existing uncancelled public methods as compatible wrappers. The streaming path will
observe the token between response chunks and before the next member, return a typed cancellation
error, remove its temporary file before returning, never publish a partial target, and leave prior
committed inventory rows intact. Once a member has been atomically published and verified, it may
remain available and be reported as successful even if cancellation arrives before the next
member. Resumable transfers remain outside this feature.

Expected registry, selection, filesystem, SDK, and partial-plan failures are formatted with three
parts: what happened, why the preparation is incomplete, and what to do next. No raw stack trace,
panic, or SDK-internal output is printed by the CLI.

## Repository findings used by the plan

- `crates/cli/src/main.rs` is currently only a Clap bootstrap, so the command surface can be
  expanded without preserving a competing command architecture.
- `crates/cli/tests/command_surface.rs` already exercises help, version, no-argument, and invalid
  input behavior and will be extended for the new subcommand.
- `MicroVmSdk` already exposes the list and download operations required by this feature, and its
  callback exposes artifact kind, member identity, byte counters, phase, and cache disposition.
- SDK construction already receives the explicit home path and runs SQLite migrations before the
  client is returned. The CLI only needs to resolve `TAUMARU_HOME` or the existing default and
  pass the result into `MicroVmSdk::new`.
- Existing SDK tests already cover registry fixtures, cache decisions, integrity failures, and
  normalized persistence. CLI tests should prove delegation and presentation behavior rather than
  duplicate those SDK cases.
