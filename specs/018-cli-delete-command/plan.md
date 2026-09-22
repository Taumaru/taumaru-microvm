# Implementation Plan: CLI `delete` Command for Deleting a MicroVM

**Branch**: `018-cli-delete-command` | **Date**: 2026-09-22 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/018-cli-delete-command/spec.md`

## Summary

Add a thin CLI `microvm delete [NAME]` command over the existing SDK
`delete_microvm(name)` operation, with no new SDK surface. The command resolves
one machine (positional/`--name` flag, or an interactive single-select over
all stored machines with state labels, exactly like the `ssh` selector but
unfiltered), reuses the existing `privilege.rs` escalation flow unchanged with
a double gate (pre-selector escalation on the bare path plus a mandatory
pre-call gate carrying the resolved name, mirroring the `start`/`stop`
re-execution pattern), requires one explicit permanent-loss confirmation in
interactive mode (skipped non-interactive), shows a `DeleteSpinner` while the
SDK deletion runs, invokes the single SDK delete operation with the resolved
name, and renders a deleted report naming the machine with owned-traces-gone
plus shared-artifacts-preserved lines and a `microvm new` next step. Running
machines surface the stop-first refusal with the exact stop command; unknown
names map to a creation-pointing triplet.

## Technical Context

**Language/Version**: Rust 2024 edition, repository stable toolchain (workspace `edition = "2024"`).

**Primary Dependencies**: Existing `clap` 4.6 (derive), `inquire` 0.9
(`Select`, `Confirm`), `indicatif` (spinner), `tokio`; SDK dependency is the path crate
`taumaru-microvm` (`delete_microvm`, `list_microvms`, `MicroVmState`,
`MicroVmDeleteResult`, `SdkError`). No new crates.

**Storage**: Existing SQLite inventory under the explicit SDK home; no schema
migration. The CLI never opens the database: the selector reads names through
`list_microvms()` and the deletion flows through `delete_microvm(name)`, which owns
all liveness, file, network, and persistence mechanics.

**Testing**: `cargo test` suites: CLI unit tests (`commands::delete` arg
resolution, escalated child argv, delete-report rendering, error-triplet
mapping, confirmation default), CLI surface tests (`crates/cli/tests/command_surface.rs`),
output format tests for the new report. Real deletions stay out of the default suite
(require rights, owned files, a terminal); live paths are covered by manual
quickstart runs.

**Target Platform**: Linux hosts; deleting requires root access (existing
privilege flow). Interactive selector and confirmation use requires an interactive
terminal; `--non-interactive` with an explicit name may run without one.

**Project Type**: CLI presentation feature in `crates/cli` over one existing
SDK operation in `crates/sdk`. No lifecycle engine in the CLI, no SDK
changes.

**Performance Goals**: One inventory listing per invocation on the selector
path; exactly one SDK delete call per invocation. Named non-interactive path
makes one SDK call total. The spinner runs for the SDK-bounded deletion
(under 2 minutes on a capable host); the CLI adds no wait of its own. No registry
access, no file hashing.

**Constraints**: CLI must not implement lifecycle behavior, file removal,
network release, or state persistence; must not open SQLite directly; must
not print key contents, socket paths, or process identities; `--non-interactive`
performs zero prompts; the SDK delete call itself always runs privileged;
deleting one machine never touches another; confirmation defaults to decline.

**Scale/Scope**: Multiple independent VMs per host are first-class; the
selector lists all stored rows in any state, ordered by name. Out of scope:
`ssh`/`list`/`status`/`inspect`/`new` commands (referenced only as pointers),
kernel override, custom volume path, connection-hint rendering, and any SDK
operation or contract change.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle; CLI stays thin | PASS: existing delete op reused, no new SDK surface | PASS: `delete.rs` resolves names, escalates, confirms, calls `delete_microvm` once; no file/network/state logic in CLI |
| Public SDK silent, typed, panic-free | PASS: no SDK changes at all | PASS: CLI maps typed `SdkError` to triplets; no new error variants |
| Domain independent from infrastructure | PASS: no domain changes | PASS: no domain/ports/adapters touched |
| SQLite is local source of truth | PASS: CLI reads via SDK only | PASS: selector via `list_microvms`, delete via `delete_microvm`; no CLI database access, no migration |
| Firecracker/firectl stay behind runtime port | PASS: untouched | PASS: no runtime changes |
| Multiple MicroVMs supported | PASS: all stored rows listed, per-name delete | PASS: no shared CLI state; exactly one machine resolved, confirmed, and deleted |
| Public contracts documented | PASS: contract/data-model/quickstart planned | PASS: artifacts below; new CLI items follow existing patterns |
| Calm, accessible CLI; English only | PASS: reuse prompt/confirm/error/spinner patterns; text-first deleted report | PASS: delete-specific triplets, non-color-safe report, `130` on cancel |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/018-cli-delete-command/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── cli-delete.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
└── cli/
    ├── src/
    │   ├── cli.rs                    # DeleteArgs + Command::Delete
    │   ├── commands/
    │   │   ├── mod.rs                # Route Command::Delete
    │   │   └── delete.rs             # Name resolution, selector, escalation, confirm, delete call
    │   ├── error.rs                  # Delete-specific triplets (additive constructors + 130 arm)
    │   └── output/
    │       └── human.rs              # DeleteSpinner + format_delete_result/write_delete_result
    └── tests/
        └── command_surface.rs        # delete help, non-interactive guards
```

**Structure Decision**: Presentation lives in `crates/cli` following the constitution-mandated
layout (`cli.rs` definitions, one `commands/` module per command, `error.rs` triplets,
`output/human.rs` rendering; success reports through the shared format/write pair). The SDK
crate is untouched. No new top-level modules, crates, or dependencies.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Selector source is the existing `list_microvms()` with no state filter —
   all stored rows in any state with state labels — because deletion applies
   to stopped, never-started, and incomplete records alike. Enforcement of
   the running guard stays in the SDK (stop-first refusal with its next
   step); the CLI never duplicates the liveness decision.
2. Named path calls `delete_microvm(name)` directly after local validation;
   no pre-resolution against any listing. `NotFound` maps to the
   creation-pointing triplet; `LifecycleConflict` with running state maps to
   the stop-first triplet with the exact stop command.
3. Elevation mirrors `stop` via unchanged `privilege.rs`: non-interactive
   fails fast with an empty command; bare interactive path escalates first
   with `["delete"]`; every path re-gates pre-call with
   `["delete", name, "--non-interactive"]`. The escalated child never
   re-escalates and re-resolves through the SDK.
4. Confirmation is one `inquire::Confirm` (default `false`) naming the
   resolved machine with a permanent-loss warning, placed after elevation
   and before the SDK call — the prune/download review-then-confirm pattern
   adapted to a single machine. Declining or interrupting cancels with no
   deletions. Non-interactive mode skips the prompt entirely.
5. Progress is a `DeleteSpinner` mirroring `StopSpinner`; the report names
   the deleted machine, confirms owned traces are gone, notes shared
   artifacts are preserved, and points at `microvm new`.
6. Errors are additive `delete_*` constructors (`delete_not_found`,
   `delete_empty`, `delete_cancelled` with a new `130` exit-code arm,
   `delete_failed` mirroring `stop_failed`, plus a dedicated running-refusal
   triplet), reusing the `missing_value`/`privileged` shapes for
   non-interactive guards.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): CLI entities, the two existing SDK reads,
  the unchanged delete result, confirmation shape, report shape, and state/flow ownership.
- [contracts/cli-delete.md](./contracts/cli-delete.md): command surface, input
  modes, resolution, double-gate escalation, confirmation, execution, error table, exit
  codes, and out-of-scope bounds.
- [quickstart.md](./quickstart.md): runnable validation scenarios and quality
  gates.

### Command surface (`cli.rs`, `commands/mod.rs`)

Add `DeleteArgs { name: Option<String>, explicit_name: Option<String>,
non_interactive: bool }` mirroring `StopArgs` exactly (optional positional
name, equivalent `--name`, `--non-interactive`), plus `Command::Delete(DeleteArgs)` with
the doc line `Delete a MicroVM by name or interactive selection.`
Route it in `commands/mod.rs` alongside the existing arms.

### Delete command flow (`commands/delete.rs`)

Implement in this order, mirroring `commands/stop.rs` structure with the
prune-style confirmation inserted:

1. Validate `--non-interactive` prerequisites first: name present (positional
   or `--name`, via the shared `resolve_name`) else missing-value error with
   usage (`Run microvm delete web-01 --non-interactive`); rights required up
   front via `require_privileged(..., non_interactive=true, home, &[],
   Vec::new(), retry_hint)` so missing rights fail before any deletion attempt
   with no prompt.
2. Bare interactive path: escalate first with child argv `["delete"]` (listing
   and selection happen privileged), then bail with the
   non-interactive-pointer triplet when the terminal is not interactive,
   fetch `sdk.list_microvms()` unfiltered, exit with `delete_empty`
   (points at `microvm new`) when none, else present an
   `inquire::Select` with the shared `prompt_render_config`
   (`Choose a MicroVM to delete`, each option labeled `name [state]`);
   cancel maps to `delete_cancelled` (exit `130`). Named interactive path
   skips the selector but keeps the flow.
3. Validate the resolved name with the shared rule (abort, no re-prompt),
   then pass the mandatory pre-call gate:
   `require_privileged(..., non_interactive=false, home, &[], child_argv,
   retry_hint)` where `child_argv` is
   `["delete", name, "--non-interactive"]`; return the child's exit code when
   it runs. The child (privileged, `TAUMARU_ESCALATED=1`) skips escalation
   and deletes directly.
4. Confirm: `inquire::Confirm` with `Delete {name}? This permanently removes
   its disk, credentials, and network attachment` (default `false`, shared
   render config); `false` or interrupt maps to `delete_cancelled` (exit
   `130`) with no deletion. Skipped entirely in non-interactive mode.
5. Execute: start `DeleteSpinner`, await the single
   `context.sdk.delete_microvm(&name)`, finish the spinner, then render:
   success → `write_delete_result(&result, terminal)` (exit `0`); `NotFound`
   → `delete_not_found(name)`; running `LifecycleConflict` →
   running-refusal triplet with `microvm stop {name}`; any other `SdkError` →
   `delete_failed("MicroVM delete failed", &error)` (retryable failures carry
   the fix-and-retry next step in the SDK message).

No registry access, no second SDK lookup, no lifecycle mutation beyond the one SDK call.

### Output and error design (`output/human.rs`, `error.rs`)

- `DeleteSpinner`: mirrors `StopSpinner` (spinner template, steady tick,
  non-interactive `eprintln!("·  Deleting MicroVM {name}")`, hidden bar).
- `format_delete_result(&MicroVmDeleteResult, TerminalCapabilities) -> String`
  mirroring `format_stop_result` structure: `\n✓ MicroVM {name} deleted\n`
  + divider + `Removed:` line (`record, volume, and owned network attachment`) +
  `Preserved:` line (`shared kernels and images`) + `Create:` line
  (`microvm new`). Text-first: no color-only or symbol-only distinction.
- `error.rs`: additive `delete_not_found` (points to `microvm new`),
  `delete_empty` (points to `microvm new`), `delete_cancelled` (exit `130`
  via a new `starts_with("MicroVM delete cancelled")` arm), `delete_failed`
  mirroring `stop_failed`, plus the running-refusal triplet naming the
  machine with `microvm stop {name}`; existing messages are not reworded.

### Test implementation

- CLI unit tests (`commands::delete`): name agreement/mismatch via the shared
  resolver; escalated child argv round-trip (`delete <name>
  --non-interactive`, bare `delete`); confirmation default-decline behavior;
  delete-report rendering (deleted title, removed/preserved lines, creation
  hint, non-color output); error-triplet mapping (`NotFound` → creation
  pointer, running conflict → stop pointer, other SDK errors →
  `delete_failed`, cancel → `130`).
- CLI surface tests: `delete --help` exposes `[NAME]`, `--name`,
  `--non-interactive` and no lifecycle/remote-command flags;
  `delete --non-interactive` without a name fails with the missing-name
  message and no prompt; non-interactive without rights reports
  elevated-rights (skipped when euid is 0, same as the existing privilege
  tests).
- Output tests: `format_delete_result` snapshots with color on/off.
- Manual (quickstart): interactive selector with mixed states, elevation
  round-trip on both bare and named paths, confirmation decline/interrupt
  with `130`, running-refusal report, retryable-failure report,
  cancellation with `130`, non-color/narrow-terminal readability — run with
  rights where required.
