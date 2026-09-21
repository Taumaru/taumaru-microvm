# Implementation Plan: CLI `stop` Command for Stopping a MicroVM

**Branch**: `013-cli-stop-command` | **Date**: 2026-09-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/013-cli-stop-command/spec.md`

## Summary

Add a thin CLI `microvm stop [NAME]` command over the existing SDK
`stop_microvm(name)` operation, with no new SDK surface. The command resolves
one machine (positional/`--name` flag, or an interactive single-select over
running machines filtered from the existing `list_microvms()`, exactly like
the `ssh` selector), reuses the existing `privilege.rs` escalation flow
unchanged with a double gate (pre-selector escalation on the bare path plus a
mandatory pre-call gate carrying the resolved name, mirroring the
`start`/`ssh` re-execution pattern), shows a `StopSpinner` while the SDK
shutdown/wait runs, invokes the single SDK stop operation with the resolved
name, and renders a stopped report that marks the graceful-versus-forced
outcome in text plus a `microvm start {name}` next step. Already-stopped
names succeed idempotently through the SDK; unknown names map to a
creation-pointing triplet.

## Technical Context

**Language/Version**: Rust 2024 edition, repository stable toolchain (workspace `edition = "2024"`).

**Primary Dependencies**: Existing `clap` 4.6 (derive), `inquire` 0.9
(`Select`), `indicatif` (spinner), `tokio`; SDK dependency is the path crate
`taumaru-microvm` (`stop_microvm`, `list_microvms`, `MicroVmState`,
`MicroVmStopResult`, `SdkError`). No new crates.

**Storage**: Existing SQLite inventory under the explicit SDK home; no schema
migration. The CLI never opens the database: the selector reads names through
`list_microvms()` and the stop flows through `stop_microvm(name)`, which owns
all socket/process/persistence mechanics.

**Testing**: `cargo test` suites: CLI unit tests (`commands::stop` arg
resolution, escalated child argv, stop-report rendering, error-triplet
mapping), CLI surface tests (`crates/cli/tests/command_surface.rs`), output
format tests for the new report. Real shutdowns stay out of the default suite
(require rights, a live guest, a terminal); live paths are covered by manual
quickstart runs.

**Target Platform**: Linux hosts; stopping requires root access (existing
privilege flow). Interactive selector use requires an interactive terminal;
`--non-interactive` with an explicit name may run without one.

**Project Type**: CLI presentation feature in `crates/cli` over one existing
SDK operation in `crates/sdk`. No lifecycle engine in the CLI, no SDK
changes.

**Performance Goals**: One inventory listing per invocation on the selector
path; exactly one SDK stop call per invocation. Named non-interactive path
makes one SDK call total. The spinner runs for the SDK-bounded shutdown
(60-second graceful window plus forced fallback, under 3 minutes); the CLI
adds no wait of its own. No registry access, no file hashing.

**Constraints**: CLI must not implement lifecycle behavior, socket messaging,
process signaling, or state persistence; must not open SQLite directly; must
not print key/socket paths or process identities; `--non-interactive`
performs zero prompts; the SDK stop call itself always runs privileged;
stopping one machine never touches another.

**Scale/Scope**: Multiple independent VMs per host are first-class; the
selector lists all running rows ordered by name. Out of scope: `ssh`/`list`/
`status`/`inspect` commands (referenced only as pointers), kernel override,
custom volume path, connection-hint rendering, and any SDK operation or
contract change.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle; CLI stays thin | PASS: existing stop op reused, no new SDK surface | PASS: `stop.rs` resolves names, escalates, calls `stop_microvm` once; no socket/signal/state logic in CLI |
| Public SDK silent, typed, panic-free | PASS: no SDK changes at all | PASS: CLI maps typed `SdkError` to triplets; no new error variants |
| Domain independent from infrastructure | PASS: no domain changes | PASS: no domain/ports/adapters touched |
| SQLite is local source of truth | PASS: CLI reads via SDK only | PASS: selector via `list_microvms`, stop via `stop_microvm`; no CLI database access, no migration |
| Firecracker/firectl stay behind runtime port | PASS: untouched | PASS: no runtime changes |
| Multiple MicroVMs supported | PASS: running rows listed, per-name stop | PASS: no shared CLI state; exactly one machine resolved and stopped |
| Public contracts documented | PASS: contract/data-model/quickstart planned | PASS: artifacts below; new CLI items follow existing patterns |
| Calm, accessible CLI; English only | PASS: reuse prompt/error/spinner patterns; text-first forcing report | PASS: stop-specific triplets, non-color-safe report, `130` on cancel |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/013-cli-stop-command/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── cli-stop.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
└── cli/
    ├── src/
    │   ├── cli.rs                    # StopArgs + Command::Stop
    │   ├── commands/
    │   │   ├── mod.rs                # Route Command::Stop
    │   │   └── stop.rs               # Name resolution, selector, escalation, stop call
    │   ├── error.rs                  # Stop-specific triplets (additive constructors + 130 arm)
    │   └── output/
    │       └── human.rs              # StopSpinner + format_stop_result/write_stop_result
    └── tests/
        └── command_surface.rs        # stop help, non-interactive guards
```

**Structure Decision**: Presentation lives in `crates/cli` following the constitution-mandated
layout (`cli.rs` definitions, one `commands/` module per command, `error.rs` triplets,
`output/human.rs` rendering; success reports through the shared format/write pair). The SDK
crate is untouched. No new top-level modules, crates, or dependencies.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Selector source is the existing `list_microvms()` filtered to `Running` —
   the same call, filter, and prompt shape the `ssh` selector uses — not the
   running-machines listing (which carries unneeded SSH material).
2. Named path calls `stop_microvm(name)` directly after local validation;
   no pre-resolution against a running listing (which would wrongly reject
   already-stopped names). `NotFound` maps to the creation-pointing triplet.
3. Elevation mirrors `start` via unchanged `privilege.rs`: non-interactive
   fails fast with an empty command; bare interactive path escalates first
   with `["stop"]`; every path re-gates pre-call with
   `["stop", name, "--non-interactive"]`. The escalated child never
   re-escalates and re-resolves through the SDK.
4. Progress is a `StopSpinner` mirroring `StartSpinner`; the report is a
   text-first graceful/forced line plus the `microvm start {name}` next step.
5. Errors are additive `stop_*` constructors (`stop_not_found`, `stop_empty`,
   `stop_cancelled` with a new `130` exit-code arm, `stop_failed` mirroring
   `start_failed`), reusing the `missing_value`/`privileged` shapes for
   non-interactive guards.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): CLI entities, the two existing SDK reads,
  the unchanged stop result, report shape, and state/flow ownership.
- [contracts/cli-stop.md](./contracts/cli-stop.md): command surface, input
  modes, resolution, double-gate escalation, execution, error table, exit
  codes, and out-of-scope bounds.
- [quickstart.md](./quickstart.md): runnable validation scenarios and quality
  gates.

### Command surface (`cli.rs`, `commands/mod.rs`)

Add `StopArgs { name: Option<String>, explicit_name: Option<String>,
non_interactive: bool }` mirroring `StartArgs` exactly (no trailing command
words — stop takes no remote command), plus `Command::Stop(StopArgs)` with
the doc line `Stop a running MicroVM by name or interactive selection.`
Route it in `commands/mod.rs` alongside the existing arms.

### Stop command flow (`commands/stop.rs`)

Implement in this order, mirroring `commands/start.rs` and `commands/ssh.rs`
structure:

1. Validate `--non-interactive` prerequisites first: name present (positional
   or `--name`, via the shared `resolve_name`) else missing-value error with
   usage (`Run microvm stop web-01 --non-interactive`); rights required up
   front via `require_privileged(..., non_interactive=true, home, &[],
   Vec::new(), retry_hint)` so missing rights fail before any stop attempt
   with no prompt.
2. Bare interactive path: escalate first with child argv `["stop"]` (listing
   and selection happen privileged), then bail with the
   non-interactive-pointer triplet when the terminal is not interactive,
   fetch `sdk.list_microvms()`, filter to `Running`, exit with `stop_empty`
   (points at `microvm start`) when none, else present an
   `inquire::Select` with the shared `prompt_render_config`
   (`Choose a MicroVM to stop`); cancel maps to `stop_cancelled` (exit
   `130`). Named interactive path skips the selector but keeps the flow.
3. Validate the resolved name with the shared rule (abort, no re-prompt),
   then pass the mandatory pre-call gate:
   `require_privileged(..., non_interactive=false, home, &[], child_argv,
   retry_hint)` where `child_argv` is
   `["stop", name, "--non-interactive"]`; return the child's exit code when
   it runs. The child (privileged, `TAUMARU_ESCALATED=1`) skips escalation
   and stops directly.
4. Execute: start `StopSpinner`, await the single
   `context.sdk.stop_microvm(&name)`, finish the spinner, then render:
   success → `write_stop_result(&result, terminal)` (exit `0`); `NotFound`
   → `stop_not_found(name)`; any other `SdkError` →
   `stop_failed("MicroVM stop failed", &error)` (lifecycle conflicts carry
   their state in the SDK message; invalid names cannot reach here past the
   shared validator).

No registry access, no second SDK lookup, no confirmation prompt, no
lifecycle mutation beyond the one SDK call.

### Output and error design (`output/human.rs`, `error.rs`)

- `StopSpinner`: mirrors `StartSpinner` (spinner template, steady tick,
  non-interactive `eprintln!("·  Stopping MicroVM {name}")`, hidden bar).
- `format_stop_result(&MicroVmStopResult, TerminalCapabilities) -> String`
  mirroring `format_start_result` structure: `\n✓ MicroVM {name} stopped\n`
  + divider + `Shutdown:` line (`graceful — the guest exited on its own` vs
  `forced — the guest did not exit and was force-terminated`) + `Start:`
  line (`microvm start {name}`). Text-first: no color-only or symbol-only
  distinction.
- `error.rs`: additive `stop_not_found` (points to `microvm new`),
  `stop_empty` (points to `microvm start`), `stop_cancelled` (exit `130`
  via a new `starts_with("MicroVM stop cancelled")` arm), `stop_failed`
  mirroring `start_failed`; existing messages are not reworded.

### Test implementation

- CLI unit tests (`commands::stop`): name agreement/mismatch via the shared
  resolver; escalated child argv round-trip (`stop <name>
  --non-interactive`, bare `stop`); stop-report rendering (graceful vs
  forced text, restart hint, non-color output); error-triplet mapping
  (`NotFound` → creation pointer, other SDK errors → `stop_failed`,
  cancel → `130`).
- CLI surface tests: `stop --help` exposes `[NAME]`, `--name`,
  `--non-interactive` and no lifecycle/remote-command flags;
  `stop --non-interactive` without a name fails with the missing-name
  message and no prompt; non-interactive without rights reports
  elevated-rights (skipped when euid is 0, same as the existing privilege
  tests).
- Output tests: `format_stop_result` snapshots for graceful and forced
  outcomes with color on/off.
- Manual (quickstart): interactive selector with mixed states, elevation
  round-trip on both bare and named paths, already-stopped idempotent
  success, forced-path report distinction, cancellation with `130`,
  non-color/narrow-terminal readability — run with rights where required.
