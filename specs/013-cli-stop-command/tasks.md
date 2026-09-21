# Tasks: CLI `stop` Command for Stopping a MicroVM

**Input**: Design documents from `/specs/013-cli-stop-command/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli-stop.md, quickstart.md

**Tests**: Included — the constitution mandates CLI tests for output/exit behavior and destructive-action safeguards; quickstart.md §Verification commands requires deterministic unit/surface tests (no KVM, root, live shutdown, or TTY in automated tests).

**Organization**: Tasks grouped by user story; each story phase is an independently testable increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US3)
- Include exact file paths in descriptions

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Baseline and pattern survey before touching code.

- [ ] T001 Verify clean baseline from repository root (`git status --short`, `cargo check -p taumaru-microvm -p taumaru-microvm-cli --all-targets`)
- [ ] T002 [P] Survey reusable patterns read-only in `crates/cli/src/commands/start.rs`, `crates/cli/src/commands/ssh.rs`, `crates/cli/src/commands/new.rs`, `crates/cli/src/privilege.rs`, `crates/cli/src/output/human.rs`, `crates/cli/src/error.rs` (double-gate escalation flow, `Select` + `prompt_render_config` running filter, `resolve_name`/`validate_name`, spinner, triplet errors, child exit-code convention)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: CLI scaffolding (args, routing, error constructors, spinner) that all story phases build on. No SDK changes in any phase.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T003 [P] Add `StopArgs` (`name: Option<String>`, `--name`, `--non-interactive`) and `Command::Stop` in `crates/cli/src/cli.rs` with parser tests (mirrors `StartArgs` exactly; no trailing command words, no lifecycle flags)
- [ ] T004 Route `Command::Stop` to `stop::run` in `crates/cli/src/commands/mod.rs` (depends on T003; module body lands in US1)
- [ ] T005 [P] Add stop-specific triplet constructors in `crates/cli/src/error.rs` reusing the `\u{1f}`-joined format: `stop_not_found` (points to `microvm new`, no creation shortcut), `stop_empty` (nothing running, points to `microvm start`), `stop_cancelled`, `stop_failed` mirroring `start_failed`, plus the `MicroVM stop cancelled` → `130` exit-code arm (existing creation/download/start/ssh wordings untouched)
- [ ] T006 [P] Add `StopSpinner` in `crates/cli/src/output/human.rs` reusing the `StartSpinner` indicatif pattern with message `Stopping MicroVM {name}` and the non-interactive `·`-prefixed stderr line (carries no progress values per FR-009)

**Checkpoint**: Foundation ready — `microvm stop --help` parses, stop triplets and spinner exist, and user story implementation can begin.

---

## Phase 3: User Story 1 — Stop a Named Running Machine (Priority: P1) 🎯 MVP

**Goal**: `microvm stop {name}` (plus `--non-interactive` scripted form) validates the name, passes the privilege gate, calls `stop_microvm` once, and prints the stopped report with the forcing outcome.

**Independent Test**: Per spec US1 — with a machine running, run `microvm stop web-01`, approve elevation when asked, verify the machine reaches stopped state and the success output names the stopped VM with the forcing outcome; repeat scripted with an explicit name and no terminal, verify no prompts occur and the exit status reflects the stop result.

- [ ] T007 [US1] Implement `crates/cli/src/commands/stop.rs` named + non-interactive flow: name resolution via shared `resolve_name` (rule verbatim "1–64 ASCII characters, starts alphanumeric, remaining alphanumeric/`-`/`_`" — abort with the rule restated, no re-prompt; positional/`--name` mismatch aborts before anything stops; selector skipped when a name is present), non-interactive guards first (missing name → missing-value error with `--name <NAME>` and full example; missing rights → `require_privileged(..., non_interactive=true, home, &[], Vec::new(), retry_hint)` with no prompt, both fail before any stop attempt), mandatory pre-call gate with child argv `["stop", name, "--non-interactive"]` returning the child status when it runs, single `sdk.stop_microvm(&name)` call with `StopSpinner` held across the await, `NotFound` → `stop_not_found`, any other `SdkError` → `stop_failed`, success → `write_stop_result` with exit `0` (already-stopped succeeds idempotently; no second lookup, no confirmation, no socket/signal/state logic in the CLI)
- [ ] T008 [US1] Render the stopped report in `crates/cli/src/output/human.rs` (`format_stop_result`/`write_stop_result` mirroring `format_start_result` structure: `\n✓ MicroVM {name} stopped\n` + divider + text-first `Shutdown:` line — graceful vs forced in words, never color or symbol alone — + `Start:` row with `microvm start {name}`; key paths, socket paths, and process identities never printed) (depends on T007 for the result shape)
- [ ] T009 [P] [US1] Add unit tests in `crates/cli/src/commands/stop.rs` (`#[cfg(test)]`) (depends on T007): positional/`--name` agreement and mismatch abort, invalid name aborts with the rule restated and no re-prompt, escalated child argv is exactly `stop <name> --non-interactive` (bare form exactly `stop`), `NotFound` maps to the creation-pointing triplet, other SDK errors map to `stop_failed`
- [ ] T010 [P] [US1] Add surface tests in `crates/cli/tests/command_surface.rs` (depends on T007): `stop --help` exposes `[NAME]`, `--name`, `--non-interactive` and no `--image`/`--disk-gb`/`--memory`/`--vcpus`/`--expose-lan`/remote-command flags; `stop --non-interactive` without a name fails with the missing-name message and no prompt; non-interactive without rights reports elevated-rights (skip when euid is 0, mirroring the existing privilege tests)

**Checkpoint**: US1 fully functional and testable independently — named interactive and scripted stops work end to end with a forcing-aware report.

---

## Phase 4: User Story 2 — Pick a Running Machine Interactively (Priority: P1)

**Goal**: Bare `microvm stop` escalates first, then offers only running machines in an ssh-identical keyboard selector and stops the chosen machine.

**Independent Test**: Per spec US2 — run bare `microvm stop` with several machines in mixed states (running, stopped, being created), verify only running machines are offered with the same presentation and behavior as the `ssh` selector, select one with the keyboard, approve elevation, and verify that exact machine reaches stopped state.

- [ ] T011 [US2] Add the running-selector path in `crates/cli/src/commands/stop.rs`: bare path escalates first with child argv `["stop"]` (listing and selection happen privileged), non-interactive-shaped invocation without a name fails with the `microvm stop <NAME> --non-interactive` usage triplet when the terminal is not interactive, selector source is `sdk.list_microvms()` filtered to `state == MicroVmState::Running` (same call and filter the `ssh` selector uses; `RunningMicroVm` SSH material not consulted), empty running set → `stop_empty` pointing at `microvm start`, `inquire::Select` with the shared `prompt_render_config` (`Choose a MicroVM to stop`), cancel → `stop_cancelled` with exit `130`, selected name flows through the shared validator and the mandatory pre-call gate into the single stop call (depends on Phase 3 T007)
- [ ] T012 [US2] Add selector-mapping unit tests in `crates/cli/src/commands/stop.rs` (depends on T011): empty running set maps to the nothing-running error with no stop attempt, prompt cancel/interrupt maps to `stop_cancelled`, a selected name produces the `stop <name> --non-interactive` pre-call child argv

**Checkpoint**: US1 AND US2 both work independently — named and selector paths share one escalation, stop-call, and report flow.

---

## Phase 5: User Story 3 — Privileged Stop With Honest Progress and Outcome (Priority: P2)

**Goal**: Every path provably reaches the SDK call privileged, the in-flight stop shows honest progress, and the report marks graceful vs forced in text that survives non-color terminals.

**Independent Test**: Per spec US3 — stop a cooperative machine and an unresponsive machine (one that ignores the graceful request), verify both runs show a live status indicator while stopping, then report stopped with forcing marked as not used (cooperative) and used (unresponsive) respectively.

- [ ] T013 [US3] Harden the privileged-execution and progress guarantee in `crates/cli/src/commands/stop.rs` (depends on T011): audit that every path (named, selector, escalated child) passes the pre-call `require_privileged` before the SDK call so the stop itself always runs privileged, the escalated child never re-escalates and re-resolves through the SDK, `StopSpinner` is started before the `stop_microvm` await and finished after on all outcomes, with no second pre-flight lookup and no CLI-side lifecycle invention
- [ ] T014 [P] [US3] Add format tests in `crates/cli/src/output/human.rs` (`#[cfg(test)]`) (depends on T008): graceful vs forced shutdown lines pinned and distinguishable without color, restart hint carries the exact stopped name, narrow/non-color rendering keeps title, shutdown line, and status markers distinguishable through text
- [ ] T015 [US3] Add cancellation and prompt-discipline tests in `crates/cli/src/commands/stop.rs` (depends on T013): selector/elevation cancel maps to `stop_cancelled` with exit `130` and nothing stopped, non-interactive mode performs zero prompts with name and rights failures ordered before any SDK call

**Checkpoint**: All three user stories independently functional — resolution, privileged execution, progress, and forcing-aware reporting form one coherent command.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Gates, contract compliance, and end-to-end validation.

- [ ] T016 Run `cargo fmt --all -- --check` from repository root and apply formatting to touched files (`crates/cli/src/cli.rs`, `crates/cli/src/commands/mod.rs`, `crates/cli/src/commands/stop.rs`, `crates/cli/src/error.rs`, `crates/cli/src/output/human.rs`, `crates/cli/tests/command_surface.rs`)
- [ ] T017 Run workspace gates from repository root: `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features` (depends on T016)
- [ ] T018 [P] Validate quickstart scenarios 1–8 in `specs/013-cli-stop-command/quickstart.md` (selector with mixed states, explicit name, elevation round-trip on bare and named paths, already-stopped success, forced-path report distinction, all failure shapes, cancellation with `130`, non-color/narrow readability) — live-stop scenarios as root on a KVM host; record any deviation (depends on all story phases)
- [ ] T019 Verify contract compliance against `specs/013-cli-stop-command/contracts/cli-stop.md`: zero SDK changes, no schema migration, no new dependency, no CLI-side SQLite, no key/socket/process material in output, `--non-interactive` performs zero prompts, the SDK call always runs privileged, exactly one machine resolved and stopped per invocation (depends on T017)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — starts immediately.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories.
- **User Stories (Phases 3–5)**: All depend on Foundational completion.
  - US1 → US2 → US3 in priority/dependency order (US2–US3 extend the same `stop.rs` flow US1 creates; run sequentially, not parallel across stories).
  - Each story is independently testable per its checkpoint once its phase completes.
- **Polish (Phase 6)**: Depends on all desired user stories being complete.

### User Story Dependencies

- **US1 (P1)**: After Foundational — no other story dependency. Delivers the MVP slice (named + scripted stop with forcing-aware report).
- **US2 (P1)**: After Foundational + US1 (adds the selector branch in the file US1 creates).
- **US3 (P2)**: After Foundational + US1 + US2 (hardens the privileged-execution, progress, and report path both prior stories share).

### Within Each User Story

- Core implementation before tests in the same file chain (e.g., T007 before T009; T011 before T012; T013 before T015).
- Tests in different files (unit in `stop.rs` vs surface in `command_surface.rs` vs format in `human.rs`) run in parallel once the implementation they cover lands.
- Story checkpoint validated before moving to the next priority.

### Parallel Opportunities

- Phase 1: T002 parallel with T001.
- Phase 2: T003 (`cli.rs` args), T005 (`error.rs`), T006 (`human.rs` spinner) parallel with each other (different files, no overlap); T004 follows T003.
- US1: T009 (unit tests in `stop.rs`) parallel with T010 (surface test file) once T007–T008 land.
- US3: T014 (format tests in `human.rs`) parallel with T015 (cancellation tests in `stop.rs`).
- Polish: T018 (manual validation) parallel with T017 gate runs on separate checkouts.

---

## Parallel Example: Foundational Phase

```bash
# CLI scaffolding (different files, no overlap):
Task T003: "Add StopArgs in crates/cli/src/cli.rs"
Task T005: "Add constructors in crates/cli/src/error.rs"
Task T006: "Add StopSpinner in crates/cli/src/output/human.rs"

# Sequential after args land (same routing chain):
Task T003 → Task T004: "Route Command::Stop in crates/cli/src/commands/mod.rs"
```

## Parallel Example: User Story 1

```bash
# After T007+T008 land:
Task T009: "Unit tests in crates/cli/src/commands/stop.rs"
Task T010: "Surface tests in crates/cli/tests/command_surface.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational (CRITICAL — args, routing, triplets, spinner).
3. Complete Phase 3: US1 (named + non-interactive flow with single SDK call and forcing-aware report).
4. **STOP and VALIDATE**: named stop against a live running VM (as root); `cargo test -p taumaru-microvm-cli`, `cargo test -p taumaru-microvm`.
5. Deploy/demo if ready — selector (US2) and hardening (US3) follow.

### Incremental Delivery

1. Setup + Foundational → CLI surface and scaffolding ready.
2. US1 → named stop works (MVP).
3. US2 → bare `stop` selects from running machines only.
4. US3 → privileged-execution guarantee, honest progress, and text-first forced distinction pinned.
5. Each increment preserves prior behavior (no rewording of existing messages, no SDK change, `ssh`/`start` flows untouched).

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together (one owner for T003→T004 routing chain, others on T005–T006 scaffolding).
2. Once Foundational is done:
   - Developer A: US1 → US2 → US3 (sequential, same file).
   - Developer B: test support and quickstart validation prep.
3. Merge in story order; run full workspace gates before Polish sign-off.

---

## Notes

- [P] tasks = different files, no dependencies; same-file story extensions (T007→T011→T013, T009→T012→T015) are intentionally sequential.
- [Story] label maps each story-phase task to its spec user story for traceability.
- `microvm ssh` remains a documentation pointer — any task implementing session behavior is out of scope and must be rejected.
- Zero SDK tasks: `stop_microvm`, `list_microvms`, `MicroVmStopResult`, and `SdkError` are reused unchanged; any task modifying `crates/sdk/` is out of scope and must be rejected.
- Commit after each task or logical group; stop at any checkpoint to validate the story independently.
- Avoid: CLI-side SQLite access, running-listing pre-resolution of named stops, second naming rule, trusted-values envelope over the privilege boundary, invented progress values, key/socket/process material in output.
