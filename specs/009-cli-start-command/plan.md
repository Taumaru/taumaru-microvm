# Implementation Plan: CLI `start` Command for Launching a MicroVM

**Branch**: `009-cli-start-command` | **Date**: 2026-09-20 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/009-cli-start-command/spec.md`

## Summary

Add a thin CLI `microvm start [NAME]` command over the existing SDK `start_microvm(name)` operation
(spec 008, already implemented). The command resolves one machine name (positional/`--name` flag,
or an interactive single-select over locally created machines), reuses the existing
`privilege.rs` escalation flow unchanged (re-executes as a root child with
`start <name> --non-interactive`, mirroring the `new` command pattern but with no trusted-values
envelope since the SDK start takes only the name), calls `start_microvm` once, and prints a
running report with ordered copyable next steps (`microvm ssh`, direct ssh, LAN key-copy paragraph
when LAN-exposed, `microvm stop` last). The `new` command's success summary gains a
`microvm start {name}` line. One small additive SDK operation, `list_microvms`, backs the
selector; everything else is CLI presentation.

## Technical Context

**Language/Version**: Rust 2024 edition, repository stable toolchain (workspace `edition = "2024"`).

**Primary Dependencies**: Existing `clap` 4.6 (derive), `inquire` 0.9 (`Select`), `indicatif` 0.18
(spinner), `tokio`; no new crates. SDK dependency is the path crate `taumaru-microvm`.

**Storage**: Existing SQLite inventory under the explicit SDK home; no schema migration
(`microvms.name`/`state` columns already exist). Selector reads names through a new SDK operation,
never by opening the database from the CLI.

**Testing**: `cargo test` suites: CLI unit tests (`commands::start`, `output::human` format tests),
CLI surface tests (`crates/cli/tests/command_surface.rs`), SDK tests for the new listing operation.
Real launches stay out of the default suite (require KVM/root); start-execution paths are covered
by mapping/unit tests plus manual quickstart runs as root.

**Target Platform**: Linux hosts; start execution requires root (existing privilege flow) and the
same host prerequisites as SDK start (KVM, verified artifacts). Non-root interactive runs escalate;
non-interactive runs fail fast without root.

**Project Type**: CLI presentation feature in `crates/cli` plus one additive SDK read operation in
`crates/sdk`. No lifecycle engine in the CLI.

**Performance Goals**: Selector opens after home resolution with a single inventory query; no
registry access, no file hashing for listing. In-flight indicator is a zero-progress spinner
(never invented percentages). Repeated start against a running VM returns via one SDK call.

**Constraints**: CLI must not implement lifecycle behavior, process management, network repair, or
its own escalation; must not open SQLite directly; must not print key contents; `--non-interactive`
performs zero prompts; already-running start reports running without a second process.

**Scale/Scope**: Multiple independent VMs per host are first-class; the selector lists all rows
ordered by name. Out of scope: `ssh`/`stop` commands (documentation hints only), kernel override,
custom volume path, LAN address selection, `list` command (the new SDK listing serves only the
selector for now).

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle; CLI stays thin | PASS: single `start_microvm` call, no orchestration | PASS: `start.rs` resolves names, escalates, renders; launch/repair stay in SDK |
| Public SDK silent, typed, panic-free | PASS: listing returns `Result`, no output | PASS: `list_microvms` is a read-only typed query; error/output contracts documented |
| Domain independent from infrastructure | PASS: no domain changes needed | PASS: only ports/adapters/repository touched for listing |
| SQLite is local source of truth | PASS: selector reads via SDK, not files | PASS: `SELECT name, state ... ORDER BY name`; no migration |
| Firecracker/firectl stay behind runtime port | PASS: untouched | PASS: no runtime changes |
| Multiple MicroVMs supported | PASS: all rows listed, per-name execution | PASS: same locks as SDK start; no shared CLI state |
| Public contracts documented | PASS: contract/data-model/quickstart planned | PASS: artifacts below; new SDK items get Rustdoc |
| Calm, accessible CLI; English only | PASS: reuse prompt/spinner/error patterns | PASS: hint order, triplet errors, non-color-safe markers |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/009-cli-start-command/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── cli-start.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── src/
│   │   ├── lib.rs                    # Re-export MicroVmSummary
│   │   ├── manager.rs                # list_microvms coordinator
│   │   ├── domain/microvm.rs         # MicroVmSummary (additive)
│   │   ├── ports/repository.rs       # list_microvm_names internal method
│   │   └── adapters/persistence/sqlite.rs  # SELECT name, state ... ORDER BY name
│   └── tests/
│       └── (listing tests: names ordered, empty inventory, state values)
└── cli/
    ├── src/
    │   ├── cli.rs                    # StartArgs + Command::Start
    │   ├── commands/
    │   │   ├── mod.rs                # Route Command::Start
    │   │   └── start.rs              # Name resolution, selector, escalation, execution
    │   ├── output/human.rs           # format/write_start_result, StartSpinner, new Start line
    │   └── error.rs                  # Start-specific triplets (additive constructors)
    └── tests/
        └── command_surface.rs        # start help, non-interactive guards
```

**Structure Decision**: Presentation lives in `crates/cli` following the constitution-mandated
layout (`cli.rs` definitions, one `commands/` module per command, `output/` rendering,
`error.rs` triplets). The selector's data need is met by one additive SDK read operation reusing
the existing domain/ports/adapters layering. No new top-level modules, crates, or dependencies.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Selector data comes from a new public SDK `list_microvms() -> Result<Vec<MicroVmSummary>, SdkError>`
   (name + state, ordered by name); CLI-side SQLite access was rejected as a boundary violation and
   name-probing is impossible, so a small additive SDK read is the only compliant source.
2. Elevation mirrors the `new` command exactly: interactive parents re-exec as
   `start <name> --non-interactive` with `TAUMARU_HOME` + `TAUMARU_ESCALATED=1` via the unchanged
   `privilege.rs` flow; no trusted-values envelope is needed because SDK start takes only the name.
   Non-interactive mode calls `require_privileged` with an empty command so missing root fails fast.
3. Name validation reuses `commands::new::resolve_name`/`validate_name` (`pub(crate)`) instead of a
   second naming rule; abort-no-reprompt and mismatch semantics match `new`.
4. In-flight feedback is a spinner reusing the `CatalogSpinner` indicatif pattern with a
   start-specific message (spinners never carry progress values, satisfying FR-012's no-invention rule).
5. Success output order is `microvm ssh` → direct ssh (with elevation prefix when the parent
   escalated) → LAN key-copy paragraph (LAN mode only, remote end uses `network.lan_address`) →
   `microvm stop` last; key paths only, never contents.
6. Errors reuse the `\u{1f}`-joined triplet constructors with start-specific what/why/next text
   (not-found points to `microvm new`, no creation shortcut); cancellation needs a
   start-specific message since `CliError::cancelled`/`Prompt("cancelled")` render creation/download
   text today.
7. The `new` summary gains one trailing `Start: microvm start {name}` line; all existing lines stay.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): CLI entities, the additive SDK listing type, validation rules,
  and state/flow ownership.
- [contracts/cli-start.md](./contracts/cli-start.md): command surface, input modes, escalation
  argv, SDK call order, output layouts, error table, exit codes, and out-of-scope bounds.
- [quickstart.md](./quickstart.md): runnable validation scenarios and quality gates.

### Public SDK boundary (additive only)

Extend the `MicroVmSdk` facade with:

- `list_microvms(&self) -> Result<Vec<MicroVmSummary>, SdkError>` returning every persisted VM
  ordered by name, each with `name: String` and `state: MicroVmState`.

Re-export `MicroVmSummary` deliberately from `crates/sdk/src/lib.rs` with Rustdoc (read-only
inventory snapshot for selectors and future listing surfaces; live running state requires
`start_microvm`/status checks, never this snapshot alone). Internally: one
`MicroVmRepository::list_microvm_names` (or equivalent) method plus the SQLite
`SELECT name, state FROM microvms ORDER BY name` mapping through the existing `MicroVmState::parse`
(return `Migration`/invalid-metadata typed error on an unparsable stored state, never panic). No
schema migration, no lock needed (single read statement through `run_repository`), no other SDK
surface changes.

### Start command flow (`commands/start.rs`)

Implement in this order, mirroring `commands/new.rs` structure:

1. Validate `--non-interactive` prerequisites first: name present (positional or `--name`,
   via the shared `resolve_name`) else missing-value error with usage; root required up front via
   `require_privileged(..., non_interactive=true, home, &[], Vec::new(), retry_hint)` so missing
   root fails before any mutation with no prompt.
2. Interactive path: resolve the name from flags when present (skip selector), else fetch
   `sdk.list_microvms()` and present an `inquire::Select` with the shared `prompt_render_config`;
   empty inventory exits with a nothing-to-start error pointing at `microvm new`; cancel maps to
   the start-specific cancellation message (exit `130`).
3. Validate the resolved name with the shared rule (abort, no re-prompt), then escalate when not
   root via `require_privileged(..., non_interactive=false, home, &[], child_argv, retry_hint)`
   where `child_argv` is `["start", name, "--non-interactive"]`; return the child's exit code when
   it runs. The child (root, `TAUMARU_ESCALATED=1`) skips escalation and executes directly.
4. Execute: show the start spinner, call `sdk.start_microvm(&name).await` once, clear the spinner,
   map errors to start triplets (table in contract), and on success render
   `write_start_result(&result, elevated_prefix)` to stdout with exit `0`. Already-running results
   render identically (same hints, no second process — guaranteed by the SDK).

No registry access, no artifact work, no confirmation prompt, no second SDK call in this command.

### Output and error design (`output/human.rs`, `error.rs`)

- `format_start_result`/`write_start_result`: `✓ MicroVM {name} running` title, then labeled rows
  for connection (`microvm ssh {name}`), direct ssh
  (`[prefix]ssh -i {key} -p {port} {user}@{ssh.address}`), conditional LAN paragraph
  (copy `{private_key_path}` to the other machine, then
  `ssh -i {key} -p {port} {user}@{lan_address}`), and `microvm stop {name}` last — text markers in
  addition to color, same paint/divider helpers as `format_new_result`.
- `StartSpinner`: same indicatif pattern as `CatalogSpinner` with message
  `Starting MicroVM {name}`; non-interactive prints the `·`-prefixed line to stderr.
- `format_new_result` gains a trailing `Start: microvm start {name}` row; existing rows unchanged.
- `error.rs`: additive start-specific constructors reusing the triplet format
  (`start_not_found` with the `microvm new` pointer, start failure mapping, start cancellation);
  existing creation/download messages are not reworded.

### Test implementation

- CLI unit tests: name agreement/mismatch via the shared resolver; `format_start_result` pins hint
  order, LAN-conditional paragraph, elevation prefix, and key-path-only rendering (host-only shows
  no LAN paragraph); non-color rendering keeps markers distinguishable.
- CLI surface tests: `start --help` exposes `[NAME]`, `--name`, `--non-interactive` and no
  lifecycle flags; `start --non-interactive` without a name fails with the missing-name message and
  no prompt; non-interactive without root reports elevated-rights (skipped when euid is 0, same as
  the existing privilege test).
- SDK tests: `list_microvms` on an empty inventory returns empty; created VMs appear ordered by
  name with their persisted states; no stdout/stderr/panic on the listing path.
- Manual (quickstart): interactive selector with 3+ VMs, elevation round-trip, LAN vs host-only
  reports, already-running repeat, and the `new` summary's start line — run as root where required.
