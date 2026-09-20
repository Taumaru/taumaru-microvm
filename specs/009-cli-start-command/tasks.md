# Tasks: CLI `start` Command for Launching a MicroVM

**Input**: Design documents from `/specs/009-cli-start-command/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli-start.md, quickstart.md

**Tests**: Included — the constitution mandates CLI tests for output/exit behavior and SDK tests
for new operations; quickstart.md §Verification commands requires deterministic unit/surface tests
(no KVM, root, live launch, or TTY in automated tests).

**Organization**: Tasks grouped by user story; each story phase is an independently testable increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US4)
- Include exact file paths in descriptions

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Baseline and pattern survey before touching code.

- [ ] T001 Verify clean baseline from repository root (`git status --short`, `cargo check -p taumaru-microvm -p taumaru-microvm-cli --all-targets`)
- [ ] T002 [P] Survey reusable patterns read-only in `crates/cli/src/commands/new.rs`, `crates/cli/src/privilege.rs`, `crates/cli/src/output/human.rs`, `crates/cli/src/commands/download.rs` (escalation flow, `Select` + `prompt_render_config`, spinner, triplet errors)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Additive SDK listing all stories depend on, plus CLI scaffolding (args, routing, error
constructors, spinner) that all story phases build on.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T003 Add public `MicroVmSummary` type with Rustdoc in `crates/sdk/src/domain/microvm.rs` — fields verbatim from data-model.md: "`name: String` Stable VM identifier, ordered by name" and "`state: MicroVmState` Last persisted lifecycle state (`Configured`, `Running`, `Creating`). Never live-verified; running truth requires start/status checks"
- [ ] T004 Re-export `MicroVmSummary` deliberately in `crates/sdk/src/lib.rs` (depends on T003)
- [ ] T005 Add `list_microvm_names` read method to the `MicroVmRepository` trait in `crates/sdk/src/ports/repository.rs` (depends on T003)
- [ ] T006 Implement the listing query in `crates/sdk/src/adapters/persistence/sqlite.rs` as `SELECT name, state FROM microvms ORDER BY name` mapped through `MicroVmState::parse` with a typed error (never panic) on unparsable state (depends on T005)
- [ ] T007 Implement `MicroVmSdk::list_microvms() -> Result<Vec<MicroVmSummary>, SdkError>` in `crates/sdk/src/manager.rs` via `run_repository`, with Rustdoc noting the snapshot is selector-grade, not live running truth (depends on T006)
- [ ] T008 [P] Add SDK listing tests in `crates/sdk/tests/microvm_listing.rs` using a temporary home: empty inventory returns empty, seeded VMs return ordered by name with persisted states, listing performs no stdout/stderr output (depends on T007)
- [ ] T009 [P] Widen `resolve_name`/`validate_name` to `pub(crate)` in `crates/cli/src/commands/new.rs` for reuse by `start` (no rule duplication; rule verbatim: "1–64 ASCII characters, starts alphanumeric, remaining alphanumeric/`-`/`_`")
- [ ] T010 [P] Add `StartArgs` (`name: Option<String>`, `--name`, `--non-interactive`) and `Command::Start` in `crates/cli/src/cli.rs` with parser tests (no lifecycle flags)
- [ ] T011 Route `Command::Start` to `start::run` in `crates/cli/src/commands/mod.rs` (depends on T010; module body lands in US1)
- [ ] T012 [P] Add start-specific triplet constructors in `crates/cli/src/error.rs` reusing the `\u{1f}`-joined format: not-found (points to `microvm new`, no creation shortcut), start failure mapping, start cancellation (existing `cancelled`/download wordings untouched)
- [ ] T013 [P] Add `StartSpinner` in `crates/cli/src/output/human.rs` reusing the `CatalogSpinner` indicatif pattern with message `Starting MicroVM {name}` and the non-interactive `·`-prefixed stderr line (carries no progress values per FR-012)

**Checkpoint**: Foundation ready — `list_microvms` returns ordered snapshots, `microvm start --help`
parses, and user story implementation can begin.

---

## Phase 3: User Story 1 — Interactive Start With Machine Selector (Priority: P1) 🎯 MVP

**Goal**: `microvm start` / `microvm start {name}` resolves one machine, escalates via the unchanged
privilege flow, calls `start_microvm` once, and prints the running report.

**Independent Test**: Per spec US1 — run `microvm start` with no arguments in an interactive
terminal against a host with several created VMs, select one with the keyboard, approve elevation,
and verify the machine reaches running state and the success output names the started VM.

- [ ] T014 [US1] Implement `crates/cli/src/commands/start.rs`: name resolution via shared `resolve_name` (selector skipped when a name is present, mismatch aborts before mutation), `Select` machine picker over `sdk.list_microvms()` with shared `prompt_render_config` (empty inventory → nothing-to-start error pointing at `microvm new`; cancel → start cancellation, exit `130`), elevation via unchanged `privilege::require_privileged` re-execing as `start <name> --non-interactive` with `TAUMARU_HOME` + `TAUMARU_ESCALATED=1` (no trusted-values envelope), immediate `sdk.start_microvm(&name)` call with `StartSpinner` held across the await (no confirmation prompt, no registry/artifact/SQLite/process work in the CLI)
- [ ] T015 [US1] Render the basic running report in `crates/cli/src/output/human.rs` (`format_start_result`/`write_start_result`: `✓ MicroVM {name} running` title plus `microvm ssh`, direct ssh, `microvm stop` rows using existing paint/divider helpers, text markers beyond color) (depends on T014 for the result shape)
- [ ] T016 [P] [US1] Add unit tests in `crates/cli/src/commands/start.rs` (`#[cfg(test)]`): positional/`--name` agreement and mismatch abort, invalid name aborts with the rule restated and no re-prompt, escalated child argv is exactly `start <name> --non-interactive`
- [ ] T017 [P] [US1] Add surface test in `crates/cli/tests/command_surface.rs`: `start --help` exposes `[NAME]`, `--name`, `--non-interactive` and no `--image`/`--disk-gb`/`--memory`/`--vcpus`/`--expose-lan` flags

**Checkpoint**: US1 fully functional and testable independently — interactive start works end to end
with a basic running report.

---

## Phase 4: User Story 2 — Non-Interactive Start for Scripts (Priority: P1)

**Goal**: `microvm start {name} --non-interactive` performs zero prompts and fails fast on missing
name or missing root before any mutation.

**Independent Test**: Per spec US2 — invoke without a terminal as root with an explicit name (starts
with no prompts, deterministic output), then repeat without a name and without root (each errors
before mutation explaining what is missing).

- [ ] T018 [US2] Implement non-interactive guards at the top of `run` in `crates/cli/src/commands/start.rs`: missing name → missing-value error with `--name <NAME>` and a full example; missing root → `require_privileged(..., non_interactive=true, home, &[], Vec::new(), retry_hint)` so elevation is never prompted; both fail before selector, escalation, or SDK calls (depends on Phase 3 T014)
- [ ] T019 [P] [US2] Add surface tests in `crates/cli/tests/command_surface.rs`: `start --non-interactive` without a name fails with the missing-name message and no prompt; non-interactive without root reports elevated-rights (skip when euid is 0, mirroring the existing privilege test) (depends on T018)
- [ ] T020 [P] [US2] Add unit tests in `crates/cli/src/commands/start.rs` for guard ordering: name validated before privilege, privilege failure performs no SDK call, explicit name skips the selector even in interactive terminals (depends on T018)

**Checkpoint**: US1 AND US2 both work independently — guided and scripted starts share one execution
path with mode-appropriate gating.

---

## Phase 5: User Story 3 — Next-Step Connection Guidance (Priority: P2)

**Goal**: Success output shows ordered copyable hints — `microvm ssh`, direct ssh (with elevation
prefix), LAN key-copy paragraph only for LAN-exposed VMs, `microvm stop` last — identical for fresh
and already-running results.

**Independent Test**: Per spec US3 — start a host-only VM and a LAN-exposed VM; each report shows
the documented hints appropriate to its network mode plus the stop hint, with no attempt to
implement or invoke the hinted commands.

- [ ] T021 [US3] Extend `format_start_result` in `crates/cli/src/output/human.rs` to the contract order: `microvm ssh {name}` → direct `[prefix]ssh -i {private_key_path} -p {port} {user}@{ssh.address}` → LAN paragraph only when `network.mode` is LAN-exposed (copy `{private_key_path}` to the other machine, then remote `ssh -i {key} -p {port} {user}@{network.lan_address}`) → `microvm stop {name}` last; key paths only, never contents (depends on T015)
- [ ] T022 [P] [US3] Add format tests in `crates/cli/src/output/human.rs` (`#[cfg(test)]`): hint order pinned, LAN paragraph present with `lan_address` (not guest address) for LAN fixtures and absent for host-only, elevation prefix (`sudo `/empty) applied, non-color rendering keeps markers distinguishable (depends on T021)
- [ ] T023 [US3] Thread the elevation prefix and identical already-running rendering through `crates/cli/src/commands/start.rs` (prefix mirrors the backend actually used; already-running results render the same rows with no second process — SDK-guaranteed) (depends on T021)

**Checkpoint**: All success reports carry complete, ordered, mode-correct connection guidance.

---

## Phase 6: User Story 4 — `new` Success Output Points to `start` (Priority: P2)

**Goal**: `microvm new` success summary keeps every existing row and appends the exact start command
for the created machine.

**Independent Test**: Per spec US4 — create a machine through `microvm new` and verify the success
summary keeps all existing information plus the command to start that machine.

- [ ] T024 [US4] Append `Start: microvm start {name}` row to `format_new_result` in `crates/cli/src/output/human.rs`, existing rows byte-identical
- [ ] T025 [P] [US4] Add regression test in `crates/cli/src/output/human.rs` (`#[cfg(test)]`, extend `new_tests`): created-machine fixture renders the start line with the exact name alongside all prior rows (depends on T024)

**Checkpoint**: All four user stories independently functional — creation flows into start with no
behavior change to provisioning or creation.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Gates, contract compliance, and end-to-end validation.

- [ ] T026 Run `cargo fmt --all -- --check` from repository root and apply formatting to touched files (`crates/sdk/src/domain/microvm.rs`, `crates/sdk/src/lib.rs`, `crates/sdk/src/ports/repository.rs`, `crates/sdk/src/adapters/persistence/sqlite.rs`, `crates/sdk/src/manager.rs`, `crates/cli/src/cli.rs`, `crates/cli/src/commands/mod.rs`, `crates/cli/src/commands/start.rs`, `crates/cli/src/output/human.rs`, `crates/cli/src/error.rs`, test files)
- [ ] T027 Run workspace gates from repository root: `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features` (depends on T026)
- [ ] T028 [P] Validate quickstart scenarios 1–7 in `specs/009-cli-start-command/quickstart.md` (selector with 3+ VMs, explicit name, elevation round-trip, LAN vs host-only, repeat-start single process, all failure shapes, `new` summary line) — live-start scenarios as root on a KVM host; record any deviation (depends on all story phases)
- [ ] T029 Verify contract compliance against `specs/009-cli-start-command/contracts/cli-start.md`: no `ssh`/`stop` implementation, no new dependency, no schema migration, no CLI-side SQLite, no key contents in output, `--non-interactive` performs zero prompts (depends on T027)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — starts immediately.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories.
- **User Stories (Phases 3–6)**: All depend on Foundational completion.
  - US1 → US2 → US3 → US4 in priority/dependency order (US2–US3 extend the same `start.rs`/`human.rs` code US1 creates; run sequentially, not parallel across stories).
  - Each story is independently testable per its checkpoint once its phase completes.
- **Polish (Phase 7)**: Depends on all desired user stories being complete.

### User Story Dependencies

- **US1 (P1)**: After Foundational — no other story dependency. Delivers the MVP slice.
- **US2 (P1)**: After Foundational + US1 (extends `run` in the file US1 creates).
- **US3 (P2)**: After Foundational + US1 (extends `format_start_result` US1 creates).
- **US4 (P2)**: After Foundational only (touches `format_new_result`, independent file region) — may proceed alongside US2/US3 once US1 is done.

### Within Each User Story

- Tests (unit/surface) written against the story's acceptance criteria; implementation before new tests is acceptable here since fixtures are deterministic and pre-existing suites must keep passing.
- Core implementation before wiring (e.g., `start.rs` flow before prefix threading).
- Story checkpoint validated before moving to the next priority.

### Parallel Opportunities

- Phase 1: T002 parallel with T001.
- Phase 2: T008 (new SDK test file), T009 (`new.rs` visibility), T010 (`cli.rs` args), T012 (`error.rs`), T013 (`human.rs` spinner) parallel with each other and with the T003→T007 SDK chain (different files, no overlap).
- US1: T016 (unit tests in `start.rs`) parallel with T017 (surface test file).
- US2: T019 parallel with T020 (different files).
- US3: T022 parallel with T023 (test module vs command file).
- US4: T025 parallel with nothing pending (single dependent test).
- Polish: T028 (manual validation) parallel with T027 gate runs on separate checkouts.

---

## Parallel Example: Foundational Phase

```bash
# SDK chain (sequential, same layering):
Task T003: "Add MicroVmSummary in crates/sdk/src/domain/microvm.rs"
  → Task T005: "Add trait method in crates/sdk/src/ports/repository.rs"
  → Task T006: "Implement query in crates/sdk/src/adapters/persistence/sqlite.rs"
  → Task T007: "Implement coordinator in crates/sdk/src/manager.rs"

# In parallel with the chain (different files, no dependencies):
Task T009: "Widen validators in crates/cli/src/commands/new.rs"
Task T010: "Add StartArgs in crates/cli/src/cli.rs"
Task T012: "Add constructors in crates/cli/src/error.rs"
Task T013: "Add StartSpinner in crates/cli/src/output/human.rs"
```

## Parallel Example: User Story 1

```bash
# After T014+T015 land:
Task T016: "Unit tests in crates/cli/src/commands/start.rs"
Task T017: "Surface test in crates/cli/tests/command_surface.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational (CRITICAL — SDK listing + CLI scaffolding).
3. Complete Phase 3: US1 (interactive selector + escalation + single SDK call + basic report).
4. **STOP and VALIDATE**: interactive start against seeded VMs; `cargo test -p taumaru-microvm-cli`, `cargo test -p taumaru-microvm`.
5. Deploy/demo if ready — scripts (US2) and hint polish (US3) follow.

### Incremental Delivery

1. Setup + Foundational → selector data and CLI surface ready.
2. US1 → guided start works (MVP).
3. US2 → automation works with fail-fast guards.
4. US3 → reports carry full ordered LAN-aware guidance.
5. US4 → creation flows into start.
6. Each increment preserves prior behavior (no rewording of existing messages, no SDK lifecycle change).

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together (one owner for the T003→T007 SDK chain, others on T009–T013 scaffolding).
2. Once Foundational is done:
   - Developer A: US1 → US2 → US3 (sequential, same files).
   - Developer B: US4 (independent region) plus test support.
3. Merge in story order; run full workspace gates before Polish sign-off.

---

## Notes

- [P] tasks = different files, no dependencies; same-file story extensions (T014→T018→T023, T015→T021) are intentionally sequential.
- [Story] label maps each story-phase task to its spec user story for traceability.
- `microvm ssh` / `microvm stop` remain documentation hints — any task implementing them is out of scope and must be rejected.
- Commit after each task or logical group; stop at any checkpoint to validate the story independently.
- Avoid: CLI-side SQLite access, second naming rule, trusted-values envelope for start, invented progress values, key contents in output.
