# Phase 0 Research: CLI `delete` Command for Deleting a MicroVM

## Research goals

Resolve how the `delete` command delivers its spec: which SDK source feeds the
all-states selector without duplicating the liveness decision, how the named
path reaches the SDK without pre-resolution, how elevation reuses the existing
privilege flow while guaranteeing the delete call always runs privileged,
where the explicit confirmation sits in the flow, and how progress and the
deleted outcome are presented — all without a second lifecycle engine, new
dependencies, schema changes, or new SDK operations.

All findings below were verified against the current sources (`crates/cli/src/
cli.rs`, `commands/mod.rs`, `commands/stop.rs`, `commands/start.rs`,
`commands/ssh.rs`, `commands/prune.rs`, `commands/new.rs` (`resolve_name`/
`validate_name`), `output/human.rs` (`StopSpinner`, `format_stop_result`),
`error.rs` (stop triplets plus the `130` arms), `privilege.rs`, plus the SDK
`delete_microvm`/`MicroVmDeleteResult` surface from `017-delete-microvm`).

## Findings

### Selector source: `list_microvms()` with no state filter, labels carry state

- Decision: the interactive selector calls the existing
  `MicroVmSdk::list_microvms()` and keeps every stored row in any state,
  presenting each as `name [state]` in an `inquire::Select` with the shared
  `prompt_render_config` — the same call and prompt shape the `ssh`/`stop`
  selectors use, minus their running-only filter.
- Rationale: deletion applies to stopped, never-started, and incomplete
  records alike, so a running-only filter would hide exactly the leftovers
  operators most need to clean up. State labels keep the menu honest (a
  running pick is visibly running before it is chosen), and enforcement stays
  in the SDK: selecting a running machine surfaces the stop-first refusal
  with its exact stop command, so the CLI never duplicates the
  socket-governed liveness decision.
- Alternatives considered: `list_running_microvms()` (rejected — carries
  unneeded SSH material and would hide non-running rows); running-only
  filter like `stop` (rejected — contradicts the clarified Q1 all-states
  answer); a new SDK listing (rejected — no new SDK surface is needed for a
  CLI-only feature).

### Named path: validate locally, call `delete_microvm` directly, no pre-resolution

- Decision: an explicitly named machine is validated with the shared
  `resolve_name`/`validate_name` rule and passed straight to the existing
  `sdk.delete_microvm(name)`. There is no listing lookup first.
  `SdkError::NotFound` maps to the `delete_not_found` triplet (points to
  `microvm new`); a running `LifecycleConflict` maps to the stop-first
  triplet with `microvm stop {name}`; any other SDK error maps through
  `delete_failed`.
- Rationale: the SDK already distinguishes unknown (typed `NotFound`) from
  running (typed `LifecycleConflict`) from retryable failure, so a
  pre-resolution listing would be a second call that adds a race without
  adding information — the same argument the stop plan records for its named
  path.
- Alternatives considered: resolve-against-listing-then-label (rejected —
  doubles SDK calls on the success path and risks resolving against a stale
  snapshot).

### Elevation: mirror `stop`, child carries the resolved name

- Decision: reuse `privilege.rs` unchanged with the `start`/`stop`
  parent-validates/child-executes pattern. Non-interactive mode calls
  `require_privileged(..., non_interactive=true, ..., Vec::new(), retry_hint)`
  so missing rights fail fast with no prompt. The bare interactive path
  escalates first with child argv `["delete"]` (so listing and selection happen
  privileged), and after resolution every path passes the gate again with
  `escalated_child_command(name)` = `["delete", name, "--non-interactive"]`
  before the confirmation and the SDK call. The escalated child
  (`TAUMARU_ESCALATED=1`) skips escalation and runs the delete directly.
- Rationale: the double gate (pre-selector plus pre-call) is what guarantees
  the SDK call itself always runs privileged while keeping the single
  existing escalation mechanism. No trusted-values envelope crosses the
  boundary: the child re-resolves the validated name through the SDK, exactly
  like `stop` (which likewise passes only the name, not machine material).
- Alternatives considered: a single pre-call gate only (rejected — the bare
  path would list and prompt unprivileged, diverging from the sibling
  behavior the spec mandates); the prune single gate (rejected — prune has
  no selector and no name to carry, while delete has both, so the stop shape
  fits and the prune shape does not).

### Confirmation: one default-decline `Confirm` after elevation, before the SDK call

- Decision: after the pre-call elevation gate and before the spinner/SDK
  call, show one `inquire::Confirm` naming the resolved machine with a
  permanent-loss warning (`Delete {name}? This permanently removes its disk,
  credentials, and network attachment`), default `false`, shared render
  config. `false` or interrupt maps to `delete_cancelled` (exit `130`) with
  no deletion. Non-interactive mode skips the prompt entirely (clarified Q2:
  the explicit name plus the root gate carry the intent).
- Rationale: delete is destructive over persistent data, so per the project
  confirmation rule the elevation gate alone is not enough — this is the one
  deliberate difference from `stop`, which has no confirmation. The prune
  flow already proves the accepted `Confirm`-with-default-decline pattern;
  placing it after elevation (not before) means the operator confirms once,
  with rights already secured, and the escalated child re-confirms nothing —
  it runs non-interactively by construction.
- Alternatives considered: confirmation before elevation (rejected — the
  operator would confirm, then possibly decline elevation, wasting the
  decision; confirming with rights secured matches review-then-act);
  always-confirm even non-interactive (rejected — contradicts clarified Q2
  and would make scripting impossible); no confirmation at all (rejected —
  contradicts the spec and the project destructive-action rule).

### Progress and outcome: `DeleteSpinner` plus a deleted report with a creation hint

- Decision: add a `DeleteSpinner` in `output/human.rs` mirroring `StopSpinner`
  (spinner message `Deleting MicroVM {name}`, non-interactive stderr line, no
  invented progress values). On success render `format_delete_result` with
  the deleted title, a `Removed:` line (record, volume, owned network
  attachment), a `Preserved:` line (shared kernels and images), and a
  `Create:` line (`microvm new`) — mirroring `format_stop_result`'s
  title/rule/detail structure, with the clarified Q3 creation hint as the
  next step.
- Rationale: the SDK deletion is bounded but not instant (network release,
  recursive removal, record commit), so a silent delete looks hung; the
  `stop` flow already proves the accepted spinner pattern. The
  removed-versus-preserved lines answer the operator's two questions after an
  irreversible action (what is gone, what is safe), and the text lines
  survive non-color terminals and pipes.
- Alternatives considered: reusing `StopSpinner` verbatim (rejected — the
  message names the wrong operation); reusing the prune grouped report
  (rejected — prune reports sets across kinds with byte counters, while a
  single-machine delete reports one name with owned/preserved lines).

### Errors: additive `delete_*` constructors, running refusal gets its own triplet

- Decision: add `delete_not_found` (names the machine, points to `microvm new`),
  `delete_empty` (nothing stored, points to `microvm new` — not `microvm start`,
  because deletion needs no running machine), `delete_cancelled`
  (selector/elevation/confirmation cancelled, exits `130` via a new
  `starts_with("MicroVM delete cancelled")` arm), `delete_failed(what,
  &SdkError)` mirroring `stop_failed` (retryable failures carry the
  fix-and-retry next step), plus a running-refusal triplet naming the machine
  with the exact `microvm stop {name}` next step. Missing-name and
  missing-rights non-interactive failures reuse the existing `missing_value`
  and `privileged` shapes; the non-interactive-no-TTY case reuses the generic
  `creation` triplet with the `microvm delete <NAME> --non-interactive` pointer.
- Rationale: the constitution requires what/why/next triplets with calm
  failure states; the additive-constructor pattern is exactly how `stop` and
  `prune` extended `error.rs`. The running case needs its own triplet (not
  the generic `delete_failed`) because the SDK `LifecycleConflict` message
  alone does not give the exact stop command, and the spec requires it.
- Alternatives considered: reusing `stop_*` constructors (rejected — they name
  the wrong operation and point at the wrong recovery commands); routing the
  running conflict through generic `delete_failed` (rejected — loses the exact
  stop-command next step the spec requires).

No unresolved technical questions remain for Phase 1 design.
