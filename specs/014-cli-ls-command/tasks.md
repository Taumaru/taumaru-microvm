# Tasks: CLI `ls` Command for Listing MicroVMs

**Input**: Design documents from `/specs/014-cli-ls-command/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli-ls.md, quickstart.md

**Tests**: Included — the constitution mandates CLI tests for output/exit behavior when the CLI changes; quickstart.md §Verification commands requires deterministic unit/surface tests (no KVM, root, live guest, or TTY in automated tests).

**Organization**: Tasks grouped by user story; each story phase is an independently testable increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US3)
- Include exact file paths in descriptions

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Baseline and pattern survey before touching code.

- [ ] T001 Verify clean baseline from repository root (`git status --short`, `cargo check -p taumaru-microvm -p taumaru-microvm-cli --all-targets`)
- [ ] T002 [P] Survey reusable patterns read-only in `crates/cli/src/commands/stop.rs`, `crates/cli/src/commands/start.rs`, `crates/cli/src/commands/new.rs`, `crates/cli/src/privilege.rs`, `crates/cli/src/output/human.rs`, `crates/cli/src/error.rs`, `crates/sdk/src/manager.rs`, `crates/sdk/src/domain/microvm.rs` (single pre-listing privilege gate with bare child argv, `format_gb`/`format_mb_gb` size helpers, format/write render pairs, triplet errors, `list_microvms` verify semantics, `MicroVmSummary` literal sites)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Additive SDK listing extension plus CLI surface scaffolding that all story phases build on.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T003 [P] Extend `MicroVmSummary` additively in `crates/sdk/src/domain/microvm.rs` with Rustdoc per field: `vcpu_count: u32`, `memory_bytes: u64`, `disk_size_bytes: u64`, `distribution_id: String`, `image_id: String` (all "already persisted at creation", "configured values chosen at creation, never live-measured usage"), `network_mode: Option<NetworkMode>`, `guest_address: Option<IpAddr>`, `lan_address: Option<IpAddr>` ("`None` for incomplete records"); correct the stale struct-level doc to describe call-time verified state (`Running` iff the volume-local control socket answers, else `Stopped`)
- [ ] T004 Populate the new fields in `crates/sdk/src/manager.rs` (`list_microvms` zips stored rows with verified states as today; scalars from `vm.record.*`, network triple from `vm.network.as_ref()` mapping `config.mode` / `config.guest_address` / `config.lan_address`, `None` when incomplete; ordering, probe, and error behavior unchanged) and update in-repo `MicroVmSummary` literals in `crates/cli/src/commands/start.rs`, `crates/cli/src/commands/stop.rs`, `crates/cli/src/commands/ssh.rs` with the new fields (depends on T003; mechanical fallout, same change)
- [ ] T005 [P] Add `LsArgs` (`non_interactive: bool` only, `--non-interactive`) and `Command::Ls(LsArgs)` with `#[command(visible_alias = "list")]` and doc `List all MicroVMs with state and configured capacities.` in `crates/cli/src/cli.rs` with parser tests (both spellings parse to one variant; positional name and lifecycle flags rejected as clap errors)
- [ ] T006 Route `Command::Ls` to `ls::run` in `crates/cli/src/commands/mod.rs` (depends on T005; module body lands in US1)
- [ ] T007 [P] Add `ls_failed` triplet constructor in `crates/cli/src/error.rs` mirroring `stop_failed` (`ls_failed(what, &SdkError)`; existing wordings untouched; no new `exit_code` arm — empty is success, cancellation inherited)

**Checkpoint**: Foundation ready — SDK builds with the extended listing shape, `microvm ls --help` and `microvm list --help` parse to one variant, `ls_failed` exists, and user story implementation can begin.

---

## Phase 3: User Story 1 — See All Machines at a Glance (Priority: P1) 🎯 MVP

**Goal**: `microvm ls` (and identical `microvm list`) secures privilege, calls `list_microvms` once, and prints the single compact table with state, capacities, image token, and network per machine.

**Independent Test**: Per spec US1 — with several machines in mixed states (running, stopped), run `microvm ls` (repeat with `microvm list`), approve elevation when asked, verify every machine appears exactly once with correct state and capacity details in one readable view.

- [ ] T008 [US1] Render the table in `crates/cli/src/output/human.rs` (`format_ls_table`/`write_ls_table` format/write pair: header `NAME STATE VCPUS MEMORY DISK IMAGE NETWORK`, one row per machine in SDK name order, widths from content maxima, no truncation; memory via `format_mb_gb`, disk via `format_gb`; image as `{distribution_id}={image_id}`; network as `host-only {guest}` / `lan {lan} (guest {guest})` / `lan (guest {guest})` / `-`; state as `MicroVmState` display text with semantic color — running green, stopped dim, text carries meaning; missing capacity/network value renders `-`; SSH user, port, key paths never printed; no key/socket/process material)
- [ ] T009 [US1] Implement `crates/cli/src/commands/ls.rs` core flow: single pre-listing `require_privileged` gate (non-interactive fails fast with empty command; interactive re-executes as bare `["ls"]`; euid 0 proceeds), one `sdk.list_microvms()` call, empty result → `write_ls_empty` exit `0`, otherwise `write_ls_table` exit `0`; listing errors map to `ls_failed("MicroVM listing failed", &error)`; no selector, no spinner, no mutation (depends on T008 for the render pair)
- [ ] T010 [P] [US1] Add SDK listing assertions in `crates/sdk/tests/microvm_listing.rs` (depends on T004): seeded live/stale/incomplete rows assert `vcpu_count`, `memory_bytes`, `disk_size_bytes`, `distribution_id`, `image_id` always present, network triple present on complete rows and `None` on incomplete rows, state still live-verified, name order preserved
- [ ] T011 [P] [US1] Add surface tests in `crates/cli/tests/command_surface.rs` (depends on T009): `ls --help` and `list --help` expose only `--non-interactive`; `list` output byte-identical to `ls` for the same fixture; stray positional/lifecycle operands rejected; non-interactive without rights reports elevated-rights (skip when euid is 0, mirroring existing privilege tests)
- [ ] T012 [P] [US1] Add unit tests in `crates/cli/src/commands/ls.rs` (depends on T009): escalated child argv is exactly `["ls"]`, non-interactive path uses the empty command; table dispatch on non-empty vs empty-report dispatch on empty; `ls_failed` mapping for listing errors

**Checkpoint**: US1 fully functional and testable independently — both spellings render the same privileged table end to end.

---

## Phase 4: User Story 2 — Privileged Listing Through the Existing Elevation Flow (Priority: P1)

**Goal**: Every privilege case behaves exactly like `stop`: interactive prompt-driven escalation before any listing, hard rerun-guidance error without rights in non-interactive/non-TTY contexts, clean cancellation with no listing and no host changes.

**Independent Test**: Per spec US2 — run `microvm ls` unprivileged in an interactive terminal, verify the elevation prompt and the listing after approval; run unprivileged without a terminal (or with `--non-interactive`), verify fast failure with rerun guidance before any listing and zero prompts.

- [ ] T013 [US2] Harden the privilege gate in `crates/cli/src/commands/ls.rs` (depends on T009): audit that the single gate runs before any listing attempt on all paths, the escalated child (`TAUMARU_ESCALATED=1`) never re-escalates and re-lists through the SDK, decline/interrupt propagates the privilege-flow status with nothing listed, no second gate and no trusted-values envelope cross the boundary
- [ ] T014 [US2] Add privilege-behavior tests in `crates/cli/src/commands/ls.rs` (depends on T013): non-interactive without rights fails before any SDK call with the rerun-as-root hint and no prompt; interactive decline/interrupt yields cancellation with exit `130` and no listing; already-root proceeds with no re-exec

**Checkpoint**: US1 AND US2 both work independently — the table renders and every privilege case matches the established flow.

---

## Phase 5: User Story 3 — Honest Empty and Failure States (Priority: P2)

**Goal**: Empty inventory prints the calm creation-pointing report with success status; degraded rows stay in place with dashes; broken-inventory failures surface as calm triplets with nonzero status.

**Independent Test**: Per spec US3 — run `microvm ls` with an empty inventory, verify the no-machines report pointing at creation; corrupt or lock the inventory, verify a calm typed-error report with a concrete next step and nonzero exit.

- [ ] T015 [US3] Add the empty report in `crates/cli/src/output/human.rs` (`format_ls_empty`/`write_ls_empty`: calm no-machines report pointing to `microvm new` on stdout, not a table and not a `CliError`) and wire its dispatch in `crates/cli/src/commands/ls.rs` (depends on T009)
- [ ] T016 [P] [US3] Add empty/degraded/failure tests: empty-report content with creation pointer and success exit in `crates/cli/src/commands/ls.rs`; dash-filled degraded row and color-off text distinction for state in `crates/cli/src/output/human.rs` (`#[cfg(test)]`); broken-inventory `ls_failed` triplet with nonzero status (depends on T015 for the empty path, T008 for the table path)

**Checkpoint**: All three user stories independently functional — overview, privilege, and honest empty/failure states form one coherent read-only command.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Gates, contract compliance, and end-to-end validation.

- [ ] T017 Run `cargo fmt --all -- --check` from repository root and apply formatting to touched files (`crates/sdk/src/domain/microvm.rs`, `crates/sdk/src/manager.rs`, `crates/cli/src/cli.rs`, `crates/cli/src/commands/mod.rs`, `crates/cli/src/commands/ls.rs`, `crates/cli/src/error.rs`, `crates/cli/src/output/human.rs`, `crates/sdk/tests/microvm_listing.rs`, `crates/cli/tests/command_surface.rs`)
- [ ] T018 Run workspace gates from repository root: `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features` (depends on T017)
- [ ] T019 [P] Validate quickstart scenarios 1–8 in `specs/014-cli-ls-command/quickstart.md` (mixed-state table, alias parity, elevation round-trip, empty inventory, degraded row, all failure shapes, cancellation with `130`, non-color/narrow/piped readability) — live listing scenarios as root on a Linux host; record any deviation (depends on all story phases)
- [ ] T020 Verify contract compliance against `specs/014-cli-ls-command/contracts/cli-ls.md`: SDK diff strictly additive (no new operation, query, migration, or probing; existing behavior/error contracts unchanged), no CLI-side SQLite, no SSH/key/socket/process material in output, `--non-interactive` performs zero prompts, listing always runs privileged, strictly read-only with exactly one SDK call per invocation (depends on T018)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — starts immediately.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories.
- **User Stories (Phases 3–5)**: All depend on Foundational completion.
  - US1 → US2 → US3 in priority/dependency order (US2–US3 extend the `ls.rs` flow US1 creates; run sequentially, not parallel across stories).
  - Each story is independently testable per its checkpoint once its phase completes.
- **Polish (Phase 6)**: Depends on all desired user stories being complete.

### User Story Dependencies

- **US1 (P1)**: After Foundational — no other story dependency. Delivers the MVP slice (privileged table with capacities, image token, network; identical `list` alias).
- **US2 (P1)**: After Foundational + US1 (hardens the gate in the file US1 creates).
- **US3 (P2)**: After Foundational + US1 (empty report dispatch extends US1; degraded/failure tests pin US1 rendering).

### Within Each User Story

- Core implementation before tests in the same file chain (e.g., T008 before T009; T009 before T012; T013 before T014; T015 before T016).
- Tests in different files (SDK listing vs surface vs unit vs format) run in parallel once the implementation they cover lands.
- Story checkpoint validated before moving to the next priority.

### Parallel Opportunities

- Phase 1: T002 parallel with T001.
- Phase 2: T003 (SDK struct), T005 (CLI args), T007 (`error.rs`) parallel with each other (different files, no overlap); T004 follows T003; T006 follows T005.
- US1: T010 (SDK tests), T011 (surface tests), T012 (unit tests) parallel with each other once T004/T009 land.
- US3: T016 parallel with story work in other files once T008/T015 land.
- Polish: T019 (manual validation) parallel with T018 gate runs on separate checkouts.

---

## Parallel Example: Foundational Phase

```bash
# Additive scaffolding (different files, no overlap):
Task T003: "Extend MicroVmSummary in crates/sdk/src/domain/microvm.rs"
Task T005: "Add LsArgs in crates/cli/src/cli.rs"
Task T007: "Add constructor in crates/cli/src/error.rs"

# Sequential after the struct lands (population + literal fallout):
Task T003 → Task T004: "Populate fields in crates/sdk/src/manager.rs"
```

## Parallel Example: User Story 1

```bash
# After T004+T009 land:
Task T010: "SDK listing assertions in crates/sdk/tests/microvm_listing.rs"
Task T011: "Surface tests in crates/cli/tests/command_surface.rs"
Task T012: "Unit tests in crates/cli/src/commands/ls.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational (CRITICAL — additive SDK fields, args, routing, triplet).
3. Complete Phase 3: US1 (privileged table with capacities, image token, network; identical alias).
4. **STOP and VALIDATE**: privileged `ls`/`list` against a mixed-state inventory (as root); `cargo test -p taumaru-microvm`, `cargo test -p taumaru-microvm-cli`.
5. Deploy/demo if ready — privilege hardening (US2) and empty/failure honesty (US3) follow.

### Incremental Delivery

1. Setup + Foundational → extended listing shape and CLI surface ready.
2. US1 → privileged overview table works (MVP).
3. US2 → every privilege case matches the established flow.
4. US3 → empty report, dash-filled degraded rows, and calm failure triplets pinned.
5. Each increment preserves prior behavior (no rewording of existing messages, no SDK behavior change, `start`/`stop`/`ssh` flows untouched).

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together (one owner for the T003→T004 SDK chain, one for the T005→T006 routing chain, one for T007–T008 scaffolding).
2. Once Foundational is done:
   - Developer A: US1 → US2 → US3 (sequential, same file).
   - Developer B: test support and quickstart validation prep.
3. Merge in story order; run full workspace gates before Polish sign-off.

---

## Notes

- [P] tasks = different files, no dependencies; same-file story extensions (T009→T013→T015, T008→T015) are intentionally sequential.
- [Story] label maps each story-phase task to its spec user story for traceability.
- `microvm ssh` remains a documentation pointer — any task implementing session behavior is out of scope and must be rejected.
- Zero new SDK operations, queries, migrations, or probes: `list_microvms` is reused with additive already-persisted fields; any task adding an SDK operation, query, migration, registry access, or filesystem probing is out of scope and must be rejected.
- Commit after each task or logical group; stop at any checkpoint to validate the story independently.
- Avoid: CLI-side SQLite access, second listing path, measured disk usage, SSH material in output or struct, filter/sort/format flags, invented progress values, key/socket/process material in output.
