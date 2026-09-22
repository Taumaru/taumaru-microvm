# Tasks: CLI `delete` Command for Deleting a MicroVM

**Input**: Design documents from `/specs/018-cli-delete-command/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli-delete.md, quickstart.md

**Tests**: Included — the constitution mandates CLI tests for output/exit behavior and destructive-action safeguards; quickstart.md §Verification commands requires deterministic unit/surface tests (no KVM, root, live deletion, or TTY in automated tests).

**Organization**: Tasks grouped by user story; each story phase is an independently testable increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US3)
- Include exact file paths in descriptions

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Baseline and pattern survey before touching code.

- [ ] T001 Verify clean baseline from repository root (`git status --short`, `cargo check -p taumaru-microvm -p taumaru-microvm-cli --all-targets`)
- [ ] T002 [P] Survey reusable patterns read-only in `crates/cli/src/commands/stop.rs`, `crates/cli/src/commands/start.rs`, `crates/cli/src/commands/ssh.rs`, `crates/cli/src/commands/prune.rs`, `crates/cli/src/commands/new.rs`, `crates/cli/src/privilege.rs`, `crates/cli/src/output/human.rs`, `crates/cli/src/error.rs` (double-gate escalation flow, `Select` + `prompt_render_config` selector, `Confirm` default-decline confirmation, `resolve_name`/`validate_name`, spinner, triplet errors, child exit-code convention)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: CLI scaffolding (args, routing, error constructors, spinner) that all story phases build on. No SDK changes in any phase.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T003 [P] Add `DeleteArgs` (`name: Option<String>`, `--name`, `--non-interactive`) and `Command::Delete` in `crates/cli/src/cli.rs` with parser tests (mirrors `StopArgs` exactly; no trailing command words, no lifecycle flags)
- [ ] T004 Route `Command::Delete` to `delete::run` in `crates/cli/src/commands/mod.rs` (depends on T003; module body lands in US1)
- [ ] T005 [P] Add delete-specific triplet constructors in `crates/cli/src/error.rs` reusing the `\u{1f}`-joined format: `delete_not_found` (points to `microvm new`, no creation shortcut), `delete_empty` (nothing stored, points to `microvm new`), `delete_cancelled`, `delete_failed` mirroring `stop_failed`, running-refusal triplet naming the machine with the exact `microvm stop {name}` next step, plus the `MicroVM delete cancelled` → `130` exit-code arm (existing creation/download/start/ssh/stop/prune wordings untouched)
- [ ] T006 [P] Add `DeleteSpinner` in `crates/cli/src/output/human.rs` reusing the `StopSpinner` indicatif pattern with message `Deleting MicroVM {name}` and the non-interactive `·`-prefixed stderr line (carries no progress values per FR-009)

**Checkpoint**: Foundation ready — `microvm delete --help` parses, delete triplets and spinner exist, and user story implementation can begin.

---

## Phase 3: User Story 1 — Delete a Named Machine With Confirmation (Priority: P1) 🎯 MVP

**Goal**: `microvm delete {name}` (plus `--non-interactive` scripted form) validates the name, passes the privilege gate, confirms once in interactive mode, calls `delete_microvm` once, and prints the deleted report naming the machine.

**Independent Test**: Per spec US1 — with a stopped machine, run `microvm delete web-01`, approve elevation when asked, confirm the shown deletion prompt, verify the machine is gone (name no longer resolves) and the success output names the deleted machine; repeat scripted with an explicit name and no terminal, verify no prompts occur and the outcome is deterministic.

- [ ] T007 [US1] Implement `crates/cli/src/commands/delete.rs` named + non-interactive flow: name resolution via shared `resolve_name` (rule verbatim "1–64 ASCII characters, starts alphanumeric, remaining alphanumeric/`-`/`_`" — abort with the rule restated, no re-prompt; positional/`--name` mismatch aborts before anything is deleted; selector skipped when a name is present), non-interactive guards first (missing name → missing-value error with `--name <NAME>` and full example; missing rights → `require_privileged(..., non_interactive=true, home, &[], Vec::new(), retry_hint)` with no prompt, both fail before any deletion attempt), mandatory pre-call gate with child argv `["delete", name, "--non-interactive"]` returning the child status when it runs, interactive `inquire::Confirm` naming the resolved machine with a permanent-loss warning defaulting to decline (decline/interrupt → `delete_cancelled`, exit `130`, nothing deleted; skipped entirely non-interactive), single `sdk.delete_microvm(&name)` under a `DeleteSpinner`, `NotFound` → `delete_not_found(name)`, running `LifecycleConflict` → stop-first triplet with `microvm stop {name}`, any other `SdkError` → `delete_failed("MicroVM delete failed", &error)`
- [ ] T008 [US1] Render the deleted report in `crates/cli/src/output/human.rs` (`format_delete_result`/`write_delete_result` mirroring `format_stop_result` structure: `\n✓ MicroVM {name} deleted\n` + divider + text-first `Removed:` line (record, volume, owned network attachment) + `Preserved:` line (shared kernels and images) + `Create:` row with `microvm new`; key contents, socket paths, and process identities never printed) (depends on T007 for the result shape)
- [ ] T009 [P] [US1] Add unit tests in `crates/cli/src/commands/delete.rs` (`#[cfg(test)]`) (depends on T007): positional/`--name` agreement and mismatch abort, invalid name aborts with the rule restated and no re-prompt, escalated child argv is exactly `delete <name> --non-interactive` (bare form exactly `delete`), `NotFound` maps to the creation-pointing triplet, running conflict maps to the stop-first triplet with the exact stop command, other SDK errors map to `delete_failed`
- [ ] T010 [P] [US1] Add surface tests in `crates/cli/tests/command_surface.rs` (depends on T007): `delete --help` exposes `[NAME]`, `--name`, `--non-interactive` and no `--image`/`--disk-gb`/`--memory`/`--vcpus`/`--expose-lan`/remote-command flags; `delete --non-interactive` without a name fails with the missing-name message and no prompt; non-interactive without rights reports elevated-rights (skip when euid is 0, mirroring the existing privilege tests)

**Checkpoint**: US1 fully functional and testable independently — named interactive and scripted deletes work end to end with confirmation and a deleted report.

---

## Phase 4: User Story 2 — Pick a Machine Interactively (Priority: P1)

**Goal**: Bare `microvm delete` escalates first, then offers every stored machine in any state with state labels in an ssh-identical keyboard selector, confirms, and deletes the chosen machine.

**Independent Test**: Per spec US2 — run bare `microvm delete` with several machines in mixed states (running, stopped, never-started), verify all stored machines are offered with state labels and the same presentation and behavior as the `ssh` selector, select one with the keyboard, approve elevation, confirm the deletion prompt, and verify that exact machine is deleted.

- [ ] T011 [US2] Add the all-states selector path in `crates/cli/src/commands/delete.rs`: bare path escalates first with child argv `["delete"]` (listing and selection happen privileged), non-interactive-shaped invocation without a name fails with the `microvm delete <NAME> --non-interactive` usage triplet when the terminal is not interactive, selector source is `sdk.list_microvms()` unfiltered (every stored row in any state, each option labeled `name [state]`; no running-only filter; `RunningMicroVm` SSH material not consulted), empty stored set → `delete_empty` pointing at `microvm new`, `inquire::Select` with the shared `prompt_render_config` (`Choose a MicroVM to delete`), cancel → `delete_cancelled` with exit `130`, selected name flows through the shared validator, the pre-call gate, the confirmation, and the single SDK call
- [ ] T012 [US2] Add selector-mapping unit tests in `crates/cli/src/commands/delete.rs` (depends on T011): empty stored set maps to the nothing-to-delete error with no deletion attempt, prompt cancel/interrupt maps to `delete_cancelled`, a selected name produces the `delete <name> --non-interactive` pre-call child argv and reaches the confirmation

**Checkpoint**: US1 AND US2 both work independently — named and selector paths share one escalation, confirmation, delete-call, and report flow.

---

## Phase 5: User Story 3 — Privileged Delete With Honest Progress and Outcome (Priority: P2)

**Goal**: Every path provably reaches the SDK call privileged, the in-flight delete shows honest progress, the confirmation defaults to decline, and the report plus failures render in text that survives non-color terminals.

**Independent Test**: Per spec US3 — delete a stopped machine and verify the run shows a live status indicator while deleting, then a deleted report naming the machine; attempt to delete a running machine, an unknown machine, and with an unavailable inventory, and verify each failure names the cause with a concrete next step and a nonzero status.

- [ ] T013 [US3] Harden the privileged-execution, confirmation, and progress guarantee in `crates/cli/src/commands/delete.rs` (depends on T011): audit that every path (named, selector, escalated child) passes the pre-call `require_privileged` before the confirmation and the SDK call so the delete itself always runs privileged, the escalated child never re-escalates and re-resolves through the SDK, the confirmation defaults to decline with the permanent-loss warning on all interactive paths, `DeleteSpinner` is started before the `delete_microvm` await and finished after on all outcomes, with no second pre-flight lookup and no CLI-side lifecycle invention
- [ ] T014 [P] [US3] Add format tests in `crates/cli/src/output/human.rs` (`#[cfg(test)]`) (depends on T008): removed/preserved lines pinned and distinguishable without color, creation hint carries `microvm new`, narrow/non-color rendering keeps title, removed line, preserved line, and status markers distinguishable through text
- [ ] T015 [US3] Add cancellation, confirmation, and prompt-discipline tests in `crates/cli/src/commands/delete.rs` (depends on T013): selector/elevation/confirmation cancel and confirmation decline map to `delete_cancelled` with exit `130` and nothing deleted, running-machine selection surfaces the stop-first triplet with nothing deleted, non-interactive mode performs zero prompts (no selector, no confirmation) with name and rights failures ordered before any SDK call

**Checkpoint**: All three user stories independently functional — resolution, privileged execution, confirmation, progress, and deleted reporting form one coherent command.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Gates, contract compliance, and end-to-end validation.

- [ ] T016 Run `cargo fmt --all -- --check` from repository root and apply formatting to touched files (`crates/cli/src/cli.rs`, `crates/cli/src/commands/mod.rs`, `crates/cli/src/commands/delete.rs`, `crates/cli/src/error.rs`, `crates/cli/src/output/human.rs`, `crates/cli/tests/command_surface.rs`)
- [ ] T017 Run workspace gates from repository root: `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features` (depends on T016)
- [ ] T018 [P] Validate quickstart scenarios 1–10 in `specs/018-cli-delete-command/quickstart.md` (selector with mixed states, explicit name, elevation round-trip on bare and named paths, confirmation decline/interrupt with `130`, running refusal with exact stop command, repeat-delete not-found, retryable-failure report, all failure shapes, cancellation with `130`, non-color/narrow readability) — live-delete scenarios as root on a KVM host; record any deviation (depends on all story phases)
- [ ] T019 Verify contract compliance against `specs/018-cli-delete-command/contracts/cli-delete.md`: zero SDK changes, no schema migration, no new dependency, no CLI-side SQLite, no key/socket/process material in output, `--non-interactive` performs zero prompts (no selector, no confirmation), the SDK call always runs privileged, exactly one machine resolved, confirmed, and deleted per invocation (depends on T017)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — starts immediately.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories.
- **User Stories (Phases 3–5)**: All depend on Foundational completion.
  - US1 → US2 → US3 in priority/dependency order (US2–US3 extend the same `delete.rs` flow US1 creates; run sequentially, not parallel across stories).
  - Each story is independently testable per its checkpoint once its phase completes.
- **Polish (Phase 6)**: Depends on all desired user stories being complete.

### User Story Dependencies

- **US1 (P1)**: After Foundational — no other story dependency. Delivers the MVP slice (named + scripted delete with confirmation and deleted report).
- **US2 (P1)**: After Foundational + US1 (adds the selector branch in the file US1 creates).
- **US3 (P2)**: After Foundational + US1 + US2 (hardens the privileged-execution, confirmation, progress, and report path both prior stories share).

### Within Each User Story

- Core implementation before tests in the same file chain (e.g., T007 before T009; T011 before T012; T013 before T015).
- Tests in different files (unit in `delete.rs` vs surface in `command_surface.rs` vs format in `human.rs`) run in parallel once the implementation they cover lands.
- Story checkpoint validated before moving to the next priority.

### Parallel Opportunities

- Phase 1: T002 parallel with T001.
- Phase 2: T003 (`cli.rs` args), T005 (`error.rs`), T006 (`human.rs` spinner) parallel with each other (different files, no overlap); T004 follows T003.
- US1: T009 (unit tests in `delete.rs`) parallel with T010 (surface test file) once T007–T008 land.
- US3: T014 (format tests in `human.rs`) parallel with T015 (cancellation tests in `delete.rs`).
- Polish: T018 (manual validation) parallel with T017 gate runs on separate checkouts.

---

## Parallel Example: Foundational Phase

```bash
# CLI scaffolding (different files, no overlap):
Task T003: "Add DeleteArgs in crates/cli/src/cli.rs"
Task T005: "Add constructors in crates/cli/src/error.rs"
Task T006: "Add DeleteSpinner in crates/cli/src/output/human.rs"

# Sequential after args land (same routing chain):
Task T003 → Task T004: "Route Command::Delete in crates/cli/src/commands/mod.rs"
```

## Parallel Example: User Story 1

```bash
# After T007+T008 land:
Task T009: "Unit tests in crates/cli/src/commands/delete.rs"
Task T010: "Surface tests in crates/cli/tests/command_surface.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational (CRITICAL — args, routing, triplets, spinner).
3. Complete Phase 3: US1 (named + non-interactive flow with confirmation, single SDK call, and deleted report).
4. **STOP and VALIDATE**: named delete against a live stored VM (as root); `cargo test -p taumaru-microvm-cli`, `cargo test -p taumaru-microvm`.
5. Deploy/demo if ready — selector (US2) and hardening (US3) follow.

### Incremental Delivery

1. Setup + Foundational → CLI surface and scaffolding ready.
2. US1 → named delete works (MVP).
3. US2 → bare `delete` selects from all stored machines with state labels.
4. US3 → privileged-execution guarantee, default-decline confirmation, honest progress, and text-first deleted distinction pinned.
5. Each increment preserves prior behavior (no rewording of existing messages, no SDK change, `ssh`/`start`/`stop` flows untouched).

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
- `microvm ssh`/`microvm start` remain documentation pointers — any task implementing session or start behavior is out of scope and must be rejected.
- Zero SDK tasks: `delete_microvm`, `list_microvms`, `MicroVmDeleteResult`, and `SdkError` are reused unchanged; any task modifying `crates/sdk/` is out of scope and must be rejected.
- Commit after each task or logical group; stop at any checkpoint to validate the story independently.
- Avoid: CLI-side SQLite access, running-only selector filter, second naming rule, trusted-values envelope over the privilege boundary, always-confirm in non-interactive mode, invented progress values, key/socket/process material in output.
