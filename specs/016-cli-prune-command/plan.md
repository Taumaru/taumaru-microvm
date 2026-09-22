# Implementation Plan: CLI `artifacts prune` Command

**Branch**: `016-cli-prune-command` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/016-cli-prune-command/spec.md`

## Summary

Add a thin CLI `microvm artifacts prune` command over the existing SDK
`prune_unused_artifacts()` operation, with no new SDK surface. The command
exposes a flag-only `Prune(PruneArgs)` subcommand beside `Download` in the
existing `artifacts` group, reuses the unchanged `privilege.rs` escalation
flow with one gate (non-interactive root gate up front, otherwise
prompt-driven escalation whose child reruns `artifacts prune
--non-interactive`), derives a read-only preview from already-public SDK
queries (downloaded identities minus `list_microvms()` references) listing
every candidate identity plus estimated freed space, asks one explicit
confirmation in interactive mode (defaulting to no; declining cancels with
no deletions), makes exactly one SDK prune call, and renders a text-first
grouped report (removed with counts and freed space per kind and in total,
separate skipped group, per-cause failures on the partial path) through new
`output/human.rs` formatters. Nothing-to-prune succeeds with the calm
report and no confirmation. No migration, no registry work, and no other
command changes are part of this feature.

## Technical Context

**Language/Version**: Rust 2024 edition, repository stable toolchain (workspace `edition = "2024"`).

**Primary Dependencies**: Existing `clap` 4.6 (derive), `inquire` 0.9
(`Confirm`), `tokio`; SDK dependency is the path crate `taumaru-microvm`
(`prune_unused_artifacts`, `list_microvms`, `PruneSummary`, `PruneFailure`,
`PrunedImageId`, `SdkError::PruneIncomplete`). No new crates.

**Storage**: Existing SQLite inventory under the explicit SDK home; no
schema migration. The CLI never opens the database: the preview reads
identities through public SDK queries and the deletion flows through the
single SDK prune call, which owns all file and row mechanics.

**Testing**: `cargo test` suites: CLI unit tests (`commands::prune` arg
shape, escalated child argv, preview/result/empty/partial rendering,
error-triplet mapping), CLI arg tests in `cli.rs`, CLI surface tests
(`crates/cli/tests/command_surface.rs`), output format tests for the new
report. Real deletions stay out of the default suite (require rights and a
seeded inventory); live paths are covered by manual quickstart runs.

**Target Platform**: Linux hosts; pruning requires root access (existing
privilege flow). Interactive preview use requires an interactive terminal;
`--non-interactive` may run without one.

**Project Type**: CLI presentation feature in `crates/cli` over one
existing SDK operation in `crates/sdk`. No lifecycle engine in the CLI, no
SDK changes.

**Performance Goals**: One read-only SDK query set per invocation for the
preview plus exactly one SDK prune call. Named paths add no extra calls:
the flag-only command has no selector. The CLI adds no wait of its own;
the SDK call duration dominates. No registry access, no file hashing.

**Constraints**: CLI must not implement candidacy, reference, deletion, or
accounting logic; must not open SQLite directly; must not print key/socket
paths, digests, or process identities; `--non-interactive` performs zero
prompts; the SDK prune call itself always runs privileged; one invocation
never touches another command's behavior; state never communicated by
color alone.

**Scale/Scope**: Multiple independent artifacts per host are first-class;
the preview and report list every candidate identity. Out of scope:
per-artifact selection, dry-run flags, filtering/sorting flags, an `ls`
drill-down, and any SDK operation or contract change.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle; CLI stays thin | PASS: existing prune op reused, no new SDK surface | PASS: `prune.rs` gates, previews, confirms, calls once, renders; no candidacy/deletion logic in CLI |
| Public SDK silent, typed, panic-free | PASS: no SDK changes at all | PASS: CLI maps typed `SdkError` (incl. `PruneIncomplete`) to triplets; no new error variants |
| Domain independent from infrastructure | PASS: no domain changes | PASS: no domain/ports/adapters touched |
| SQLite is local source of truth | PASS: CLI reads via SDK only | PASS: preview via public queries, deletion via prune call; no CLI database access, no migration |
| Firecracker/firectl stay behind runtime port | PASS: untouched | PASS: no runtime changes |
| Registry and artifact boundaries explicit | PASS: no registry work in prune | PASS: CLI performs no acquisition, verification, or measurement |
| Multiple MicroVMs/artifacts supported | PASS: home-wide prune, no selection | PASS: flag-only command lists every identity; no single-artifact assumption |
| Public contracts and compatibility documented | PASS: contract/data-model artifacts planned | PASS: CLI contract, data-model, and quickstart are included |
| Calm, semantic, keyboard-friendly CLI | PASS: text-first groups, explicit labels planned | PASS: preview + confirm copy matches sibling flows; no pager/selector; `Confirm` default no |
| Project text is English | PASS | PASS |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/016-cli-prune-command/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── cli-prune.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── cli/
│   ├── src/
│   │   ├── cli.rs                # ArtifactsCommand::Prune + PruneArgs (--non-interactive)
│   │   ├── commands/
│   │   │   ├── mod.rs            # Dispatch ArtifactsCommand::Prune to prune::run
│   │   │   └── prune.rs          # Gate, preview, confirm, one SDK call, render
│   │   ├── error.rs              # prune_failed + prune_cancelled triplets, exit-130 prefix
│   │   └── output/
│   │       └── human.rs          # format/write prune preview, result, empty
│   └── tests/
│       └── command_surface.rs    # artifacts prune help/flag/privilege surface tests
└── sdk/
    └── (untouched by this feature)
```

**Structure Decision**: All new behavior lives in the CLI crate behind the
existing `cli.rs` / `commands/` / `error.rs` / `output/` layout. The SDK is
consumed read-only (preview queries) plus the single deleting call. No new
top-level modules beyond `commands/prune.rs`, no SDK changes.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Clap shape is `Prune(PruneArgs)` beside `Download` with a flag-only
   `PruneArgs { non_interactive }` (the `LsArgs` precedent); dispatch beside
   the `Download` arm; no alias, no positionals, no selection flags.
2. One privilege gate in the `ls` shape (non-interactive root gate, else
   prompt-driven escalation), with the escalated child argv `["artifacts",
   "prune", "--non-interactive"]` (the `start`/`stop` re-execution
   pattern). No double gate: prune has no selector and no name to carry.
3. The interactive preview is derived read-only from already-public SDK
   queries (downloaded identities minus `list_microvms()` references, same
   existence rule), listing every identity plus estimated freed space, with
   one `Confirm` (default `false`, sibling copy pattern). The preview is an
   estimate; the `PruneSummary` is authoritative. No SDK dry-run is added.
4. Freed space reuses the existing binary-unit helper (`B`/`KiB`/`MiB`/
   `GiB`); `0 B` for stale entries; no third copy of the function.
5. Result rendering is a text-first stacked report in `output/human.rs`
   (`✓`/`○`/`×` plus explicit labels, kernels then images, counts and
   bytes, separate skipped group, per-cause failures), reusing
   `paint`/`divider`; no key/socket/digest/process data printed.
6. Errors gain `prune_failed` and `prune_cancelled` triplets plus the
   exit-130 cancelled prefix; `PruneIncomplete` gets the dedicated partial
   branch (removed so far, skipped, per-cause failures, retry step, exit
   `1`) instead of the generic download-flavored SDK arm.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): flag-only input, SDK-read records, report groups, state machine, and ownership rules.
- [contracts/cli-prune.md](./contracts/cli-prune.md): command surface, home resolution, input modes, escalation, execution, error contract, exit codes, and out-of-scope.
- [quickstart.md](./quickstart.md): CLI usage, host prerequisites, failure behavior, and gates.

### Public CLI boundary

Extend the `artifacts` group in `crates/cli/src/cli.rs` with:

- `ArtifactsCommand::Prune(PruneArgs)` documented as reclaiming kernels
  and images no existing machine references;
- `PruneArgs { non_interactive: bool }` with `--non-interactive`
  documented as disabling all prompts with root required up front.

Add `commands/prune.rs` with `escalated_child_command()` returning the
`["artifacts", "prune", "--non-interactive"]` argv and `run(context,
arguments)` implementing gate → preview → confirm → one SDK call →
render. Wire the dispatch arm in `commands/mod.rs`. Do not add aliases,
positionals, selection flags, or any other command in this feature.

### Prune coordinator

Implement the flow in `commands/prune.rs` in this order:

1. Non-interactive path: root gate up front (`require_privileged` with
   `non_interactive: true`, empty child argv); on pass, skip preview and
   confirmation entirely and go to step 4.
2. Interactive path: one escalation gate (`require_privileged` with
   `non_interactive: false` and the escalated child argv) before any
   preview or deletion; declined/interrupted escalation returns the
   existing cancellation with no side effects.
3. Derive the preview read-only (downloaded identities minus
   `list_microvms()` references, same existence rule; estimated freed
   space from the counters the preview queries expose). Empty preview:
   render the calm nothing-to-prune report, exit `0`, no confirmation.
   Otherwise show the preview (every identity plus estimated freed
   space) and ask one `Confirm` (default `false`, sibling render config
   and help copy); declined/interrupted confirmation returns
   `prune_cancelled` with no deletions.
4. Make exactly one `sdk.prune_unused_artifacts()` call. On `Ok(summary)`
   render the result (removed with counts and bytes, skipped group when
   non-empty; empty removed lists render nothing-to-prune) and exit `0`.
   On `PruneIncomplete { summary, failures }` render the partial branch
   (removed so far with freed space so far, skipped group, each failure
   with cause, repair-and-retry next step) and exit `1`. All other SDK
   errors map through `prune_failed`.
5. Never open SQLite, never hash files, never contact the registry, and
   never measure sizes independently: every identity and byte counter
   shown comes from the SDK.

The coordinator must never treat the preview as authoritative. A fresh
escalated child reruns the whole flow non-interactively from current
inventory state.

### Presentation and error design

Add to `output/human.rs` beside the `ls`/`stop` formatters:

- `format_prune_preview(candidates, estimated_bytes, capabilities)` plus
  a `write_*` wrapper: every kernel and image identity plus estimated
  freed space, explicit labels, `divider` rule;
- `format_prune_result(&PruneSummary, capabilities)` plus wrapper:
  removed kernels/images with counts per kind, freed space per kind and
  total via the shared helper, skipped group when non-empty;
- `format_prune_empty(capabilities)` plus wrapper: calm nothing-to-prune
  report pointing at continued use (no creation pointer: the operator
  already has machines or downloads; the report simply states nothing
  needed pruning);
- partial rendering inside `format_prune_result` or a dedicated
  `format_prune_partial(&PruneSummary, &[PruneFailure], capabilities)`:
  removed so far, skipped group, each failed key with cause.

Add to `error.rs` following the `stop_failed` shape:

- `prune_failed(what, &SdkError)` triplet (`what\u{1f}{error}\u{1f}next`
  with a prune retry next step);
- `prune_cancelled()` triplet (nothing deleted, rerun-when-ready next
  step) plus the `exit_code` cancelled-prefix match so it exits `130`.

The command prints no key/socket paths, digests, or process identities,
uses no pager or selector, and communicates state with text labels that
read identically without color.

### Test implementation

Add or update CLI tests for:

- parse surface: `artifacts prune` accepts only `--non-interactive`;
  positionals and selection flags (`--image`, `--disk-gb`, `--memory`,
  `--vcpus`, `--expose-lan`, `--name`) are rejected; no home flag exists;
- escalated child argv renders exactly `artifacts prune --non-interactive`;
- preview/result/empty/partial rendering: identities plus bytes appear,
  groups are text-distinguishable without color, nothing-to-prune reads
  calm with no removal list, partial keeps the removed set with causes;
- error mapping: `PruneIncomplete` renders removed-so-far plus named
  failures with exit `1`; declined confirmation exits `130`;
- surface: `artifacts prune --help` exposes the flag-only surface;
  `non_interactive` without root reports the privilege error with no
  prompt;
- regression: existing `artifacts download` help/flow tests plus the
  sibling privilege tests behave exactly as before.
