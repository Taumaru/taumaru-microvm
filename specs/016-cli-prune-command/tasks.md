# Tasks: CLI `artifacts prune` Command

**Input**: Design documents from `/specs/016-cli-prune-command/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli-prune.md, quickstart.md

**Tests**: Included — the constitution mandates CLI tests for output/exit behavior and destructive-action safeguards; quickstart.md §Verification commands requires deterministic unit/surface tests (no root, live deletion, or TTY in automated tests).

**Organization**: Tasks grouped by user story; each story phase is an independently testable increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US3)
- Include exact file paths in descriptions

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Baseline and pattern survey before touching code.

- [ ] T001 Verify clean baseline from repository root (`git status --short`, `cargo check -p taumaru-microvm -p taumaru-microvm-cli --all-targets`)
- [ ] T002 [P] Survey reusable patterns read-only in `crates/cli/src/commands/ls.rs`, `crates/cli/src/commands/stop.rs`, `crates/cli/src/commands/download.rs`, `crates/cli/src/privilege.rs`, `crates/cli/src/output/human.rs`, `crates/cli/src/error.rs` (single-gate escalation flow, escalated child argv pattern, `Confirm` + `prompt_render_config` review-then-confirm, `format_bytes` binary-unit helper, triplet errors, child exit-code convention)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: CLI scaffolding (args, routing, error constructors, report formatters) that all story phases build on. No SDK changes in any phase.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T003 [P] Add `PruneArgs` (`non_interactive: bool` via `--non-interactive`, no positionals, no selection flags) and `ArtifactsCommand::Prune(PruneArgs)` in `crates/cli/src/cli.rs` with parser tests (mirrors the `LsArgs` flag-only shape; `artifacts prune --image x` and `artifacts prune <name>` are rejected)
- [ ] T004 Route `ArtifactsCommand::Prune` to `prune::run` in `crates/cli/src/commands/mod.rs` (depends on T003; module body lands in US1)
- [ ] T005 [P] Add prune-specific triplet constructors in `crates/cli/src/error.rs` reusing the `\u{1f}`-joined format: `prune_failed` (mirrors `stop_failed` with a prune retry next step), `prune_cancelled` (nothing deleted, rerun-when-ready next step), plus the `MicroVM prune cancelled` → `130` exit-code arm (existing creation/download/start/ssh/stop/ls wordings untouched)
- [ ] T006 [P] Add prune report formatters in `crates/cli/src/output/human.rs` reusing `paint`/`divider` and the shared `format_bytes` binary-unit helper (`B`/`KiB`/`MiB`/`GiB`, `0 B` for stale entries): `format_prune_preview` (every kernel and image identity plus estimated freed space), `format_prune_result` (removed kernels/images with counts per kind, freed space per kind and in total, skipped group when non-empty), `format_prune_empty` (calm nothing-to-prune report, no removal list), `format_prune_partial` (removed so far with freed space so far, skipped group, each failed key with cause) plus thin `write_*` wrappers; groups text-distinguishable without color, no key/socket/digest/process material printed

**Checkpoint**: Foundation ready — `microvm artifacts prune --help` parses, prune triplets and formatters exist, and user story implementation can begin.

---

## Phase 3: User Story 1 — Reclaim Disk With One Command (Priority: P1) 🎯 MVP

**Goal**: `microvm artifacts prune` (plus `--non-interactive` scripted form) derives the read-only preview, confirms once interactively, calls `prune_unused_artifacts` exactly once, and prints the grouped result with freed space.

**Independent Test**: Per spec US1 — with a seeded mixed inventory (referenced plus unreferenced kernels/images, stopped/never-started owners), run `microvm artifacts prune`, confirm the preview, verify every unreferenced artifact is gone, every referenced artifact is intact, and the output reports removed identities with freed space per kind and in total.

- [ ] T007 [US1] Implement `crates/cli/src/commands/prune.rs` core flow: `escalated_child_command()` returning exactly `["artifacts", "prune", "--non-interactive"]`, non-interactive path (root gate up front via `require_privileged(..., non_interactive=true, home, &[], Vec::new(), retry_hint)` with no prompts, straight to the single SDK call), interactive path (one escalation gate with the child argv before any preview work), read-only preview derivation (downloaded identities minus `list_microvms()` references with the constraint "existence of the MicroVM row alone protects an artifact; lifecycle state, process liveness, socket responsiveness, and creation completeness never influence membership", estimated freed space from the preview queries), empty preview → nothing-to-prune report with exit `0` and no confirmation, otherwise preview plus one `Confirm` (default `false`, sibling render config and help copy; declined/interrupted → `prune_cancelled` with no deletions), exactly one `sdk.prune_unused_artifacts()` call, `Ok(summary)` with empty removed lists → nothing-to-prune report with exit `0` else result report with exit `0` (depends on T004, T005, T006)
- [ ] T008 [P] [US1] Add unit tests in `crates/cli/src/commands/prune.rs` (`#[cfg(test)]`) (depends on T007): escalated child argv is exactly `artifacts prune --non-interactive`, declined/interrupted confirmation maps to `prune_cancelled` with no SDK call, empty preview and empty summary both render nothing-to-prune with no confirmation
- [ ] T009 [P] [US1] Add parser tests in `crates/cli/src/cli.rs` (depends on T003): `artifacts prune` accepts only `--non-interactive`; positionals (`artifacts prune web-01`) and selection flags (`--image`, `--disk-gb`, `--memory`, `--vcpus`, `--expose-lan`, `--name`) are rejected; no home flag exists
- [ ] T010 [P] [US1] Add surface tests in `crates/cli/tests/command_surface.rs` (depends on T007): `artifacts prune --help` exposes the flag-only surface with no `--image`/lifecycle flags; `artifacts prune` with a positional name fails; non-interactive without rights reports elevated-rights (skip when euid is 0, mirroring the existing privilege tests)

**Checkpoint**: US1 fully functional and testable independently — interactive preview-confirm-delete and scripted delete work end to end with the grouped report.

---

## Phase 4: User Story 2 — Privileged Prune Through the Existing Elevation Flow (Priority: P1)

**Goal**: Every path provably reaches the SDK call privileged through the unchanged flow, with cancellation before any deletion on every decline path.

**Independent Test**: Per spec US2 — run `microvm artifacts prune` unprivileged in a terminal, verify the elevation prompt precedes any preview work and the prune completes after approval; decline elevation and verify cancellation with no deletions and no host changes; run `--non-interactive` unprivileged and verify the hard error with rerun guidance before any deletion attempt.

- [ ] T011 [US2] Harden the privileged-execution guarantee in `crates/cli/src/commands/prune.rs` (depends on T007): audit that every path (interactive, non-interactive, escalated child) passes `require_privileged` before any preview derivation or SDK call so deletion always runs privileged, the escalated child never re-escalates and reruns the whole flow non-interactively from current inventory state, preview derivation performs zero writes (no SQLite opens, no file hashing, no registry contact)
- [ ] T012 [P] [US2] Add privilege unit tests in `crates/cli/src/commands/prune.rs` (depends on T011): interactive gate plans `artifacts prune --non-interactive` escalation (mirrors the `ls` `interactive_gate_plans_bare_ls_escalation` pattern), escalated child never re-escalates, non-interactive mode performs zero prompts with rights failure ordered before any SDK call

**Checkpoint**: US1 AND US2 both work independently — reclamation and the privilege guarantee share one gate, one SDK call, and one report flow.

---

## Phase 5: User Story 3 — Honest Empty and Failure States (Priority: P2)

**Goal**: Nothing-to-prune stays calm and successful, partial failures keep the removed set with named causes, and skipped transfers read as a separate group — all text-distinguishable without color.

**Independent Test**: Per spec US3 — run with a fully-referenced and an empty inventory and verify the calm report with success status and no deletions; seed one undeletable unreferenced artifact and verify everything else is reclaimed, the output shows removed-so-far with freed space plus the named failure with a repair step, and the exit is nonzero.

- [ ] T013 [US3] Wire the partial-failure branch in `crates/cli/src/commands/prune.rs` (depends on T007): match `SdkError::PruneIncomplete { summary, failures }` to the partial report (removed set with freed space so far, skipped group, each failed artifact key with its cause, repair-and-retry next step) with exit `1`; all other SDK errors map through `prune_failed` with exit `1` (depends on T005 for the triplet)
- [ ] T014 [P] [US3] Add format tests in `crates/cli/src/output/human.rs` (`#[cfg(test)]`) (depends on T006, T013): removed/skipped/failed groups pinned and distinguishable without color, counts match identity list lengths, byte counters render through the shared helper, nothing-to-prune contains no removal list, partial keeps the removed set with causes
- [ ] T015 [P] [US3] Add error-mapping unit tests in `crates/cli/src/commands/prune.rs` (depends on T013): `PruneIncomplete` renders removed-so-far plus named failures with exit `1`, generic SDK errors render `prune_failed` with exit `1`, declined preview renders `prune_cancelled` with exit `130`

**Checkpoint**: All three user stories independently functional — reclamation, privileged execution, and honest empty/failure reporting form one coherent command.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Gates, contract compliance, and end-to-end validation.

- [ ] T016 Run `cargo fmt --all -- --check` from repository root and apply formatting to touched files (`crates/cli/src/cli.rs`, `crates/cli/src/commands/mod.rs`, `crates/cli/src/commands/prune.rs`, `crates/cli/src/error.rs`, `crates/cli/src/output/human.rs`, `crates/cli/tests/command_surface.rs`)
- [ ] T017 Run workspace gates from repository root: `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features` (depends on T016)
- [ ] T018 [P] Validate quickstart scenarios 1–6 in `specs/016-cli-prune-command/quickstart.md` (mixed-inventory prune with preview confirm, nothing-to-prune, scripted non-interactive, partial failure plus retry, all failure shapes, cancellation with `130`, non-color/narrow readability) — live-deletion scenarios as root on a seeded host; record any deviation (depends on all story phases)
- [ ] T019 Verify contract compliance against `specs/016-cli-prune-command/contracts/cli-prune.md`: zero SDK changes, no schema migration, no new dependency, no CLI-side SQLite, no key/socket/digest/process material in output, `--non-interactive` performs zero prompts, the SDK call always runs privileged, exactly one SDK prune call per invocation (depends on T017)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — starts immediately.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories.
- **User Stories (Phases 3–5)**: All depend on Foundational completion.
  - US1 → US2 → US3 in priority/dependency order (US2–US3 extend the same `prune.rs` flow US1 creates; run sequentially, not parallel across stories).
  - Each story is independently testable per its checkpoint once its phase completes.
- **Polish (Phase 6)**: Depends on all desired user stories being complete.

### User Story Dependencies

- **US1 (P1)**: After Foundational — no other story dependency. Delivers the MVP slice (preview-confirm-delete plus scripted delete with grouped report).
- **US2 (P1)**: After Foundational + US1 (hardens the gate in the file US1 creates).
- **US3 (P2)**: After Foundational + US1 (wires the partial branch in the file US1 creates; needs T005/T006 scaffolding only).

### Within Each User Story

- Core implementation before tests in the same file chain (e.g., T007 before T008; T011 before T012; T013 before T015).
- Tests in different files (unit in `prune.rs` vs parser in `cli.rs` vs surface in `command_surface.rs` vs format in `human.rs`) run in parallel once the implementation they cover lands.
- Story checkpoint validated before moving to the next priority.

### Parallel Opportunities

- Phase 1: T002 parallel with T001.
- Phase 2: T003 (`cli.rs` args), T005 (`error.rs`), T006 (`human.rs` formatters) parallel with each other (different files, no overlap); T004 follows T003.
- US1: T008 (unit tests in `prune.rs`) parallel with T009 (parser tests in `cli.rs`) parallel with T010 (surface test file) once T007 lands.
- US3: T014 (format tests in `human.rs`) parallel with T015 (error-mapping tests in `prune.rs`).
- Polish: T018 (manual validation) parallel with T017 gate runs on separate checkouts.

---

## Parallel Example: Foundational Phase

```bash
# CLI scaffolding (different files, no overlap):
Task T003: "Add PruneArgs in crates/cli/src/cli.rs"
Task T005: "Add constructors in crates/cli/src/error.rs"
Task T006: "Add prune formatters in crates/cli/src/output/human.rs"

# Sequential after args land (same routing chain):
Task T003 → Task T004: "Route ArtifactsCommand::Prune in crates/cli/src/commands/mod.rs"
```

## Parallel Example: User Story 1

```bash
# After T007 lands:
Task T008: "Unit tests in crates/cli/src/commands/prune.rs"
Task T009: "Parser tests in crates/cli/src/cli.rs"
Task T010: "Surface tests in crates/cli/tests/command_surface.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational (CRITICAL — args, routing, triplets, formatters).
3. Complete Phase 3: US1 (preview-confirm-delete plus scripted delete with grouped report).
4. **STOP and VALIDATE**: mixed-inventory prune as root (interactive confirm plus `--non-interactive`); `cargo test -p taumaru-microvm-cli`, `cargo test -p taumaru-microvm`.
5. Deploy/demo if ready — privilege hardening (US2) and failure honesty (US3) follow.

### Incremental Delivery

1. Setup + Foundational → CLI surface and scaffolding ready.
2. US1 → preview-confirm-delete works (MVP).
3. US2 → every path provably privileged with cancellation before deletion.
4. US3 → partial failures keep the removed set with named causes; groups pinned text-first.
5. Each increment preserves prior behavior (no rewording of existing messages, no SDK change, `download`/`new`/`start`/`stop`/`ssh`/`ls` flows untouched).

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together (one owner for T003→T004 routing chain, others on T005–T006 scaffolding).
2. Once Foundational is done:
   - Developer A: US1 → US2 → US3 (sequential, same file).
   - Developer B: test support and quickstart validation prep.
3. Merge in story order; run full workspace gates before Polish sign-off.

---

## Notes

- [P] tasks = different files, no dependencies; same-file story extensions (T007→T011→T013, T008→T012→T015) are intentionally sequential.
- [Story] label maps each story-phase task to its spec user story for traceability.
- `microvm artifacts download` remains a group sibling — any task changing its flow is out of scope and must be rejected.
- Zero SDK tasks: `prune_unused_artifacts`, `list_microvms`, `PruneSummary`, `PruneFailure`, and `SdkError` are reused unchanged; any task modifying `crates/sdk/` is out of scope and must be rejected.
- Commit after each task or logical group; stop at any checkpoint to validate the story independently.
- Avoid: CLI-side SQLite access, file hashing, registry contact, independent size measurement, second naming rule, trusted-values envelope over the privilege boundary, invented progress values, key/socket/digest/process material in output.
