# Implementation Plan: CLI `ls` Command for Listing MicroVMs

**Branch**: `014-cli-ls-command` | **Date**: 2026-09-21 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/014-cli-ls-command/spec.md`

## Summary

Add a thin read-only CLI `microvm ls` command (with `microvm list` as a
`visible_alias` on the same variant) over the existing SDK
`list_microvms()` operation, with the smallest additive SDK extension the
spec allows: extra already-persisted capacity and network fields on the
existing `MicroVmSummary` struct, populated from rows
`list_stored_microvms()` already loads — no new SDK operation, no new
query, no schema migration, no registry or filesystem probing. The command
secures root through the unchanged `privilege.rs` flow with a single
pre-listing gate (bare `["ls"]` child, no second gate since there is no
machine name to carry), then renders one compact table (`NAME STATE VCPUS
MEMORY DISK IMAGE NETWORK`) with configured-only sizes, stored network
details, no SSH material, and dashes for degraded rows, plus a calm
stdout empty report pointing to `microvm new`.

## Technical Context

**Language/Version**: Rust 2024 edition, repository stable toolchain
(workspace `edition = "2024"`, `rustc 1.98.1`).

**Primary Dependencies**: Existing `clap` 4.6 (derive, `visible_alias`),
`tokio`; SDK dependency is the path crate `taumaru-microvm`
(`list_microvms`, `MicroVmState`, `MicroVmSummary`, `NetworkMode`,
`SdkError`). No new crates. `inquire` and `indicatif` are not used by
this command (no selector, no spinner).

**Storage**: Existing SQLite inventory under the explicit SDK home; no
schema migration. Every displayed field already lives in the `microvms`
row (`vcpu_count`, `memory_requested_bytes`, `disk_size_bytes`,
`distribution_id`, `image_id`) or the `vm_networks` row
(`load_network_record`), both loaded by the existing
`list_stored_microvms()`. The CLI never opens the database.

**Testing**: `cargo test` suites: SDK listing tests
(`crates/sdk/tests/microvm_listing.rs`, `public_api.rs`), CLI unit tests
(`commands::ls` child argv, table rendering, empty report, error-triplet
mapping), CLI surface tests (`crates/cli/tests/command_surface.rs`),
output format tests for the new table. Live socket verification stays in
deterministic fixtures (bound Unix socket answering machine-config, stale
file resolving to stopped); no test requires KVM, root, or a terminal.

**Target Platform**: Linux hosts; listing requires root access (existing
privilege flow). No interactive terminal is required beyond the gate —
piped runs (`microvm ls | head`) render the same table to stdout, since
the command takes no input and shows no selector.

**Project Type**: CLI presentation feature in `crates/cli` over one
existing SDK operation in `crates/sdk`, plus a strictly additive field
extension on the existing listing result. No lifecycle engine in the CLI.

**Performance Goals**: Exactly one SDK listing call per invocation; state
verification reuses the existing bounded-parallel probes (8–32 permits,
12s probe timeout, failure resolves to `Stopped`). Listing completes in
under 30 seconds for a typical inventory (tens of machines) on a capable
host. No registry access, no file hashing, no per-VM follow-up queries.

**Constraints**: SDK stays silent, typed, and panic-free; CLI stays a
thin consumer (no lifecycle behavior, no socket/process logic, no direct
SQLite access); listing is strictly read-only; `--non-interactive`
performs zero prompts; state never communicated by color alone; SSH user,
port, and key paths never printed; no new filter/sort/format flags;
English-only operator text.

**Scale/Scope**: Multiple independent VMs per host are first-class; rows
are name-ordered, one per machine. Out of scope: per-machine drill-down,
filtering, sorting, output-format flags, measured disk usage, guest-OS
introspection, and any change to existing SDK behavior, lifecycle
semantics, error contracts, or persistence layout.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle/gate | Pre-research | Post-design |
|---|---|---|
| SDK owns lifecycle; CLI stays thin | PASS: existing list op reused; SDK diff limited to additive persisted fields | PASS: `ls.rs` resolves home, gates privilege, calls `list_microvms` once, renders; no socket/signal/state logic in CLI |
| Public SDK silent, typed, panic-free | PASS: extension populates `Option` fields from loaded rows, no new failure mode | PASS: no new `SdkError` variant; incomplete records map to `None` fields, probe timeouts still resolve to `Stopped` |
| Domain independent from infrastructure | PASS: new struct fields are plain data (`u32`/`u64`/`String`/`Option<IpAddr>`/`Option<NetworkMode>`) | PASS: `microvm.rs` gains data only; SQLite/manager mapping stays behind ports/adapters |
| SQLite is local source of truth | PASS: CLI reads via SDK only, no migration | PASS: zero schema change; no CLI database access; population reuses `list_stored_microvms` rows |
| Firecracker/firectl stay behind runtime port | PASS: untouched | PASS: no runtime changes; verification path (`verify_all`) unchanged |
| Multiple MicroVMs supported | PASS: all rows listed, name-ordered | PASS: no shared CLI state; one row per machine, degraded rows kept with dashes |
| Public contracts documented | PASS: contract/data-model/quickstart planned; additive SDK fields get Rustdoc | PASS: artifacts below; struct docs corrected (live-verified state) and new fields documented |
| Calm, accessible CLI; English only | PASS: reuse table/error patterns; text-first state; empty report as success | PASS: `ls`-specific empty report and `ls_failed` triplet; `130` behavior inherited unchanged; non-color-safe output |

No constitution violation or complexity exception is required.

## Project Structure

### Documentation (this feature)

```text
specs/014-cli-ls-command/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
│   └── cli-ls.md
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
crates/
├── sdk/
│   ├── src/
│   │   ├── domain/
│   │   │   └── microvm.rs          # MicroVmSummary += capacity/network fields (additive)
│   │   ├── manager.rs              # list_microvms populates the new fields
│   │   └── lib.rs                  # Re-exports unchanged (same struct name)
│   └── tests/
│       ├── microvm_listing.rs      # Field assertions on seeded rows
│       └── public_api.rs           # Export surface (unchanged names)
└── cli/
    ├── src/
    │   ├── cli.rs                  # LsArgs + Command::Ls (visible_alias = "list")
    │   ├── commands/
    │   │   ├── mod.rs              # Route Command::Ls
    │   │   └── ls.rs               # Privilege gate, list call, render dispatch
    │   ├── error.rs                # Additive ls_failed constructor
    │   └── output/
    │       └── human.rs            # format_ls_table/write_ls_table + empty report
    └── tests/
        └── command_surface.rs      # ls/list help, alias parity, non-interactive guards
```

**Structure Decision**: Presentation lives in `crates/cli` following the
constitution-mandated layout (`cli.rs` definitions, one `commands/` module
per command, `error.rs` triplets, `output/human.rs` rendering through the
shared format/write pair). The SDK diff is confined to the existing
listing result type plus its single population site. No new top-level
modules, crates, or dependencies. Existing `MicroVmSummary` literal sites
(`commands::start`/`stop`/`ssh` tests) are updated with the new fields in
the same change — mechanical fallout of the additive extension, not a
second feature.

## Complexity Tracking

No constitution violations or additional project layers require justification.

## Phase 0 Research Summary

Research is recorded in [research.md](./research.md). The resolved decisions are:

1. Data source is the existing `list_microvms()` with `MicroVmSummary`
   extended by already-persisted fields (`vcpu_count`, `memory_bytes`,
   `disk_size_bytes`, `distribution_id`, `image_id` always present;
   `network_mode`/`guest_address`/`lan_address` as `Option`, `None` for
   incomplete records) — no new operation, query, migration, or probing.
2. Elevation is a single pre-listing gate through unchanged
   `privilege.rs`: non-interactive fails fast with an empty command; the
   interactive path re-executes as bare `["ls"]`; the privileged child
   proceeds directly. No second gate (no name to carry).
3. Rendering is one compact table (`NAME STATE VCPUS MEMORY DISK IMAGE
   NETWORK`) reusing `format_gb`/`format_mb_gb`, image as
   `{distribution}={image}`, text-first state labels, SSH material
   excluded, dashes for missing values. No spinner, no pager, no selector.
4. Empty inventory renders a stdout report pointing to `microvm new` with
   exit `0` — not a `CliError`, so no exit-code arm is needed.
5. Errors are one additive `ls_failed(what, &SdkError)` mirroring
   `stop_failed`; non-interactive rights reuse `privileged`, home failures
   reuse `Home`, cancellation behavior is inherited unchanged.
6. Surface is `Command::Ls(LsArgs)` with `visible_alias = "list"` and a
   flag-only `LsArgs { non_interactive }` — one variant, one code path for
   both spellings by construction.

## Phase 1 Design

Detailed outputs:

- [data-model.md](./data-model.md): CLI entities, the extended SDK
  listing shape, network rendering rules, table and empty-report shapes,
  and state/flow ownership.
- [contracts/cli-ls.md](./contracts/cli-ls.md): command surface, input
  modes, single-gate escalation, execution, table contract, error table,
  exit codes, and out-of-scope bounds.
- [quickstart.md](./quickstart.md): runnable validation scenarios and
  quality gates.

### SDK extension (`domain/microvm.rs`, `manager.rs`)

Extend the struct additively (field order: existing fields first, then
capacities, then network — new code reads naturally, old literals update
mechanically):

```rust
pub struct MicroVmSummary {
    pub name: String,
    pub state: MicroVmState,
    pub vcpu_count: u32,
    pub memory_bytes: u64,
    pub disk_size_bytes: u64,
    pub distribution_id: String,
    pub image_id: String,
    pub network_mode: Option<NetworkMode>,
    pub guest_address: Option<IpAddr>,
    pub lan_address: Option<IpAddr>,
}
```

Rustdoc on each new field states its source (creation-persisted row, or
stored network row with `None` for incomplete records) and repeats the
configured-not-measured rule for sizes. The stale struct-level comment
(claiming state is never live-verified) is corrected to describe the
call-time verified state, since the edit touches the doc anyway.

In `manager.rs::list_microvms`, zip stored rows with verified states as
today and populate: scalars from `vm.record.*` (`memory_bytes` is the
requested-bytes column, matching what `format_mb_gb` shows on the
creation review path); network triple from `vm.network.as_ref()` mapping
`config.mode` / `config.guest_address` / `config.lan_address`, `None`
when the record is incomplete. Ordering, probe, and error behavior are
byte-for-byte unchanged. `lib.rs` and `domain/mod.rs` re-exports are
unchanged (same struct name).

In-repo fallout (same change, mechanical): struct literals constructing
`MicroVmSummary` in `commands::start`/`stop`/`ssh` tests gain the new
fields; `microvm_listing.rs` seeds assert the new fields on live, stale,
and incomplete rows.

### Command surface (`cli.rs`, `commands/mod.rs`)

Add flag-only args mirroring the other commands' non-interactive flag and
nothing else:

```rust
pub(crate) struct LsArgs {
    /// Disable prompts; root is required up front.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,
}
```

Add the variant with an advertised alias so help shows both spellings:

```rust
/// List all MicroVMs with state and configured capacities.
#[command(visible_alias = "list")]
Ls(LsArgs),
```

Route it in `commands/mod.rs` alongside the existing arms. A positional
name or any per-machine flag surfaces as a clap error (covered by a
parser test) — the command takes no operands by design.

### Ls command flow (`commands/ls.rs`)

Implement in this order, mirroring the `stop` bare path minus its second
gate:

1. Non-interactive mode first: call `require_privileged(...,
   non_interactive=true, home, &[], Vec::new(), retry_hint)` so missing
   rights fail before any listing attempt with no prompt, then list and
   render directly with deterministic output.
2. Interactive path: single gate with `require_privileged(...,
   non_interactive=false, home, &[], vec![OsString::from("ls")],
   retry_hint)` — unprivileged terminals re-execute elevated and exit
   with the child's status; already-root invocations (euid 0, including
   the escalated child with `TAUMARU_ESCALATED=1`) proceed with no
   change. No TTY check beyond the gate: piped runs render the same table
   once privileged.
3. Execute: await the single `context.sdk.list_microvms()`; empty
   result → `write_ls_empty(terminal)` and exit `0`; otherwise
   `write_ls_table(&listed, terminal)` and exit `0`. Map listing errors
   to `ls_failed("MicroVM listing failed", &error)`; home-resolution
   failures propagate through the existing `Home` path.

No selector, no spinner, no confirmation, no second SDK lookup, no
mutation of any kind.

### Output and error design (`output/human.rs`, `error.rs`)

- `format_ls_table(&[MicroVmSummary], TerminalCapabilities) -> String`:
  header `NAME  STATE  VCPUS  MEMORY  DISK  IMAGE  NETWORK` (dim when
  color is on), one row per machine in SDK order, column widths from
  content maxima, no truncation. Cell rules: state via `MicroVmState`
  display text with semantic color (running green, stopped dim);
  `vcpus` as integer; memory via `format_mb_gb`, disk via `format_gb`;
  image as `{distribution_id}={image_id}`; network as `host-only
  {guest}` / `lan {lan} (guest {guest})` / `lan (guest {guest})` /
  `-`. Missing capacity or network value renders `-` (degraded row
  stays in place). Leading `\n`, no trailing pager.
- `write_ls_table` / `format_ls_empty` / `write_ls_empty`: the shared
  format/write pair convention; the empty report names the zero-machine
  state and points to `microvm new` on stdout.
- `error.rs`: additive `ls_failed(what, &SdkError)` mirroring
  `stop_failed`; no new `exit_code` arm (empty is success, cancellation
  is inherited). Existing messages are not reworded.

### Test implementation

- SDK tests (`microvm_listing.rs`): seeded live/stale/incomplete rows
  assert the new fields (capacities and identity always present, network
  triple present on complete rows and `None` on incomplete rows, state
  still live-verified, name order preserved).
- CLI unit tests (`commands::ls`): escalated child argv round-trip
  (`["ls"]`, non-interactive empty command); table rendering (column
  presence, name order, configured sizes, image token, network strings,
  dash-filled degraded row, state text-distinguishable with color off);
  empty-report content; `ls_failed` mapping.
- CLI surface tests: `ls --help` and `list --help` expose only
  `--non-interactive`; `list` output is byte-identical to `ls` for the
  same fixture; positional/flag operands rejected; non-interactive
  without rights reports elevated-rights (skipped when euid is 0, same
  as the existing privilege tests).
- Manual (quickstart): mixed-state inventory table, elevation
  round-trip, empty inventory, broken inventory, piped output, and
  non-color/narrow-terminal readability — run with rights where
  required.
