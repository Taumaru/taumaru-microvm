# Phase 0 Research: CLI `artifacts prune` Command

## Research goals

Resolve the implementation questions for one thin CLI `microvm artifacts prune`
command over the existing SDK `prune_unused_artifacts()` operation: which Clap
shape fits the existing `artifacts` group, how the privilege gate and
escalated child argv work for a flag-only subcommand, where the
review-then-confirm preview gets its data without deleting anything, and how
the result/error rendering reuses the established output and triplet helpers.
No SDK behavior change is expected.

All findings below were verified against the current sources (`crates/cli/src/
cli.rs`, `commands/mod.rs`, `commands/ls.rs`, `commands/stop.rs`,
`commands/download.rs`, `output/human.rs`, `error.rs`, `privilege.rs`, plus
the SDK `PruneSummary`/`PruneFailure`/`PruneIncomplete` surface).

## Findings

### Clap shape: new `Prune` variant beside `Download`, one flag-only args struct

- Decision: extend the existing `ArtifactsCommand` enum with
  `Prune(PruneArgs)` and add `PruneArgs { non_interactive: bool }`, mirroring
  the `LsArgs` flag-only shape. Dispatch in `commands/mod.rs` beside the
  `Download` arm. No alias, no positional, no selection flags per FR-002.
- Rationale: `ArtifactsArgs` already owns the `artifacts` subcommand group,
  so `microvm artifacts prune` falls out of the existing group with no new
  top-level command and no second home-directory policy. The existing
  `ls_parser_*` unit tests plus `command_surface.rs` help tests give the exact
  assertion pattern for the new parse tests.
- Alternatives considered: a top-level `microvm prune` command. Rejected: the
  spec fixes `microvm artifacts prune` beside `artifacts download`, and a
  second top-level entry would split artifact inventory UX across two groups.

### Privilege gate: single pre-deletion gate, `ls`-style child argv

- Decision: reuse `privilege::require_privileged` with `SystemPrivilege`
  unchanged, in the `ls` shape — non-interactive root gate up front, otherwise
  one prompt-driven escalation gate before any preview or deletion — with
  `escalated_child_command()` returning `["artifacts", "prune",
  "--non-interactive"]` (the `start`/`stop` re-execution pattern: the
  escalated child always runs non-interactively).
- Rationale: prune deletes files below the SDK home and mutates the
  inventory, so like every sibling command the SDK call itself must always
  run privileged. A flag-only command has nothing to carry into the child
  (the `ls` precedent: bare `["ls"]`; here the group prefix plus the
  non-interactive flag), so one gate suffices — there is no selector path
  needing the stop-style double gate.
- Alternatives considered: the `stop` double gate (pre-selector plus
  pre-call). Rejected: the second gate exists to protect the selector's SDK
  listing plus the named re-execution; prune has no selector and no name to
  carry, so one gate covers every path.

### Preview without deletion: list candidates read-only before confirming

- Decision: the interactive flow needs the unreferenced set *before* the
  confirmation, but the SDK exposes no dry-run — it only has the deleting
  `prune_unused_artifacts()`. The CLI therefore derives the preview from a
  read-only combination of already-public SDK queries: downloaded kernels and
  images (the existing per-kind download/listing surface) minus the
  identities referenced by `list_microvms()` rows, reusing the same
  existence-based reference rule the SDK documents. The preview lists every
  kernel and image identity plus the estimated freed space; the operator
  confirms with one `Confirm` prompt (default `false`, same copy pattern as
  the download/new review steps). Declining or interrupting cancels with no
  deletions. Non-interactive mode skips preview and confirmation entirely.
- Rationale: keeps the SDK untouched (FR-013 expects no SDK change) while
  honoring the clarified confirm-with-preview requirement. Because the SDK is
  the sole deletion engine, the preview is explicitly an estimate: a
  concurrent download or creation between preview and deletion can change the
  outcome, and the final report (from the real `PruneSummary`) is
  authoritative, not the preview.
- Alternatives considered: (a) adding an SDK dry-run operation. Rejected:
  new SDK surface contradicts FR-013 and duplicates candidacy logic across
  the boundary. (b) Deleting first and treating the result as the preview.
  Rejected: violates the clarification (declining must delete nothing).
  (c) No preview, elevation as the only safeguard. Rejected: the
  clarification explicitly chose confirm-with-preview.

### Freed-space formatting: reuse the existing binary-unit helper

- Decision: render every byte counter with the existing `format_bytes`-style
  binary helper (`B`/`KiB`/`MiB`/`GiB`, one decimal above bytes — the same
  function body already exists in both `output/human.rs` and
  `commands/download.rs::format_image_bytes`). Zero-byte stale entries
  render as `0 B`.
- Rationale: the spec requires human-readable units but no new measurement;
  both existing helpers already implement exactly that scale. The prune
  report reuses the helper rather than adding a third copy.
- Alternatives considered: GB-only rendering like the `new` flow. Rejected:
  reclaimed sets are often a few MiB/KiB (fixture images are bytes), so GB
  would render as `0 GB` and hide the very information the command exists
  to show.

### Result rendering: text-first grouped report in `output/human.rs`

- Decision: add `format_prune_preview`, `format_prune_result`,
  `format_prune_empty`, and `write_*` thin wrappers in `output/human.rs`
  next to the `ls`/`stop` formatters, using the same `paint`/`divider`
  helpers: a `✓`/`○`/`×` marker plus explicit text labels (`removed`,
  `skipped`, `failed`, `nothing to prune`), never color alone. The result
  shows kernels then images (SDK order), counts per kind, freed space per
  kind and in total, the skipped group, and — on partial failure — each
  failed artifact key with its cause. No key paths, socket paths, digests,
  or process identities are printed.
- Rationale: FR-011 mandates the restrained neutral visual language with
  text-first state; placing formatters beside the sibling ones keeps the
  `commands/prune.rs` module to orchestration (gate → preview → confirm →
  one SDK call → render) per the CLI ownership rules.
- Alternatives considered: a table layout like `ls`. Rejected: variable
  identity lengths plus three groups plus byte counters fit the stacked
  `stop`-style report better than fixed columns.

### Error mapping: one `prune_failed` triplet plus the existing gates

- Decision: add `CliError::prune_failed(what, &SdkError)` and
  `CliError::prune_cancelled()` constructors following the `stop_failed` /
  `stop_cancelled` shape (`what\u{1f}{error}\u{1f}next`), extend the
  `exit_code` cancelled-prefix match with the prune cancellation prefix,
  and map `SdkError::PruneIncomplete { summary, failures }` to a partial
  report (removed set with freed space so far, skipped group, each failure
  with cause, repair-and-retry next step) with exit `1` — not to a generic
  triplet. All other SDK errors map through `prune_failed`. Declined
  preview/confirmation maps to `prune_cancelled` (exit `130`), same as the
  download `Prompt("cancelled")` path.
- Rationale: the constitution requires what/why/next triplets with calm
  failure states; the partial case needs the structured summary the generic
  triplet would flatten, so it gets the dedicated branch while everything
  else reuses the uniform shape.
- Alternatives considered: routing `PruneIncomplete` through the generic
  `Sdk(error)` arm. Rejected: that arm says "Artifact preparation failed"
  with download retry guidance — wrong operation, and it would hide the
  per-artifact causes the spec requires callers to see.

No unresolved technical questions remain for Phase 1 design.
