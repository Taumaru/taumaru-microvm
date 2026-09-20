# Tasks: CLI `ssh` Command for Connecting to a MicroVM

**Input**: Design documents from `/specs/010-cli-ssh-command/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli-ssh.md, quickstart.md

**Tests**: Included — the constitution mandates CLI tests for output/exit behavior and SDK tests
for new operations; quickstart.md §Verification commands requires deterministic unit/surface tests
(no KVM, root, live guest, or TTY in automated tests).

**Organization**: Tasks grouped by user story; each story phase is an independently testable increment.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (US1–US3)
- Include exact file paths in descriptions

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Baseline and pattern survey before touching code.

- [ ] T001 Verify clean baseline from repository root (`git status --short`, `cargo check -p taumaru-microvm -p taumaru-microvm-cli --all-targets`)
- [ ] T002 [P] Survey reusable patterns read-only in `crates/cli/src/commands/start.rs`, `crates/cli/src/commands/new.rs`, `crates/cli/src/privilege.rs`, `crates/cli/src/error.rs` (escalation flow, `Select` + `prompt_render_config`, `resolve_name`/`validate_name`, `backend_path` lookup, triplet errors, child exit-code convention)

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Additive SDK running-machines listing all stories depend on, plus CLI scaffolding (args, routing, error constructors) that all story phases build on.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T003 Add public `RunningMicroVm` type with Rustdoc in `crates/sdk/src/domain/microvm.rs` — fields verbatim from data-model.md: "`name: String` Stable VM identifier, ordered by name" and "`ssh: SshConnectionInfo` Stored SSH connection material (guest `user`, `port`, guest `address`, host `private_key_path`, `public_key_path`), verbatim from the inventory. Paths only, never key contents"
- [ ] T004 Re-export `RunningMicroVm` deliberately in `crates/sdk/src/lib.rs` (depends on T003)
- [ ] T005 Implement `MicroVmSdk::list_running_microvms() -> Result<Vec<RunningMicroVm>, SdkError>` in `crates/sdk/src/manager.rs` by composing existing reads only (iterate `list_microvm_names` in name order, skip rows whose persisted state is not `Running`, load candidates with existing `find_microvm`, keep rows passing the existing private `is_vm_live` — recorded process still references the VM **and** the volume-local control socket answers), with Rustdoc noting liveness is verified at call time and key paths are returned verbatim with no filesystem validation inside the listing; no repository trait change, no migration, no locks, no mutation (depends on T003)
- [ ] T006 [P] Add SDK listing tests using the existing `TestRuntime` liveness doubles in `crates/sdk/src/manager.rs` (`#[cfg(test)]`): empty inventory returns empty, live VMs appear ordered by name with stored SSH material, dead-process / silent-socket / stopped / creating rows are all absent, listing performs no stdout/stderr output and never panics (depends on T005)
- [ ] T007 [P] Add `SshArgs` (`name: Option<String>`, `--name`, `--non-interactive`, `command: Vec<String>` with `#[arg(trailing_var_arg = true, allow_hyphen_values = true)]`) and `Command::Ssh` in `crates/cli/src/cli.rs` with parser tests (no lifecycle flags; everything after `--` is always remote command)
- [ ] T008 Route `Command::Ssh` to `ssh::run` in `crates/cli/src/commands/mod.rs` (depends on T007; module body lands in US1)
- [ ] T009 [P] Add ssh-specific triplet constructors in `crates/cli/src/error.rs` reusing the `\u{1f}`-joined format: `ssh_not_found` (points to `microvm new`, no creation shortcut), `ssh_not_running` (names the machine and its persisted state, points to `microvm start {name}`), `ssh_empty` (nothing running, points to `microvm start`), `ssh_cancelled` (exit `130`), key-file and ssh-binary failures (existing creation/download/start wordings untouched)

**Checkpoint**: Foundation ready — `list_running_microvms` returns live-verified running snapshots, `microvm ssh --help` parses, and user story implementation can begin.

---

## Phase 3: User Story 1 — Connect to a Named Running Machine (Priority: P1) 🎯 MVP

**Goal**: `microvm ssh {name} [-- COMMAND...]` resolves one running machine, escalates via the unchanged privilege flow, pre-flights the chosen key, and hands the terminal to `ssh` with silent success and verbatim exit propagation.

**Independent Test**: Per spec US1 — with a machine running, run `microvm ssh web-01`, approve elevation when asked, verify a guest shell opens for that exact machine and closes cleanly; repeat scripted with an explicit name plus a remote command and no terminal, verify no prompts occur and the remote exit status is returned.

- [ ] T010 [US1] Implement `crates/cli/src/commands/ssh.rs`: name resolution via shared `resolve_name` (selector skipped when a name is present; rule verbatim "1–64 ASCII characters, starts alphanumeric, remaining alphanumeric/`-`/`_`" — abort with the rule restated, no re-prompt; positional/`--name` mismatch aborts before anything opens), resolution only against `sdk.list_running_microvms()` with the miss-labeling fallback (`sdk.list_microvms()` once on the failure path: absent means unknown → `ssh_not_found`, present means known-but-not-running → `ssh_not_running`), non-interactive guards first (missing name → missing-value error with `--name <NAME>`; missing rights → `require_privileged(..., non_interactive=true, home, &[], Vec::new(), retry_hint)` with no prompt), elevation via unchanged `privilege::require_privileged` re-execing as `ssh <name> --non-interactive [-- <remote...>]` with `TAUMARU_HOME` + `TAUMARU_ESCALATED=1` (no trusted-values envelope — the child re-resolves through the SDK), key pre-flight on the chosen entry only (private-key path must be a readable regular file, else calm triplet, no session), `ssh` located with existing `backend_path("ssh")` (absent → guided error), spawn of `ssh -i {key} [-p {port} when != 22] {user}@{address} [-- remote...]` with inherited stdin/stdout/stderr, no host-key options added, no Ctrl-C interception around the wait, silent success (prints nothing), child exit status propagated verbatim with signal death mapping to `130`
- [ ] T011 [P] [US1] Add unit tests in `crates/cli/src/commands/ssh.rs` (`#[cfg(test)]`): positional/`--name` agreement and mismatch abort, invalid name aborts with the rule restated, escalated child argv round-trips name plus remote words after `--`, argv builder omits `-p` when port is 22 and includes `-p {port}` otherwise, argv builder appends remote words verbatim and never adds host-key options, missing key maps to a triplet with no session (depends on T010)
- [ ] T012 [P] [US1] Add surface tests in `crates/cli/tests/command_surface.rs`: `ssh --help` exposes `[NAME]`, `[COMMAND]...`, `--name`, `--non-interactive` and no `--image`/`--disk-gb`/`--memory`/`--vcpus`/`--expose-lan` flags; `ssh --non-interactive` without a name fails with the missing-name message and no prompt; non-interactive without rights reports elevated-rights (skip when euid is 0, mirroring the existing privilege test) (depends on T010)

**Checkpoint**: US1 fully functional and testable independently — named shell and named remote command work end to end with silent handoff.

---

## Phase 4: User Story 2 — Pick a Running Machine Interactively (Priority: P1)

**Goal**: Bare `microvm ssh` (plus bare `microvm ssh -- COMMAND...`) offers only running machines in a keyboard selector and runs the session or remote command on the chosen machine.

**Independent Test**: Per spec US2 — run bare `microvm ssh` with several machines in mixed states (running, stopped, being created), verify only running machines are offered, select one with the keyboard, approve elevation, and verify the guest session opens for the selected machine.

- [ ] T013 [US2] Add the running-selector path in `crates/cli/src/commands/ssh.rs`: when no name is present and the terminal is interactive, fetch `sdk.list_running_microvms()` and present an `inquire::Select` with the shared `prompt_render_config` (running names only, keyboard navigation, confirmation, cancellation); empty running set → `ssh_empty` pointing at `microvm start`, cancel → `ssh_cancelled` with exit `130`; trailing words supplied without a name (after `--`) run as the remote command on the interactively selected machine (depends on Phase 3 T010)
- [ ] T014 [US2] Add unit tests in `crates/cli/src/commands/ssh.rs` for selector-path parsing and mapping: bare `-- <remote>` words parse as remote command with no name, words-after-name parse as remote command without `--`, empty running set maps to the nothing-running error (no session opened), selector cancellation maps to exit `130` (depends on T013)

**Checkpoint**: US1 AND US2 both work independently — named and selector paths share one resolution, escalation, and spawn flow.

---

## Phase 5: User Story 3 — Native-Feeling Privileged Terminal Session (Priority: P2)

**Goal**: The established session is indistinguishable from the equivalent manual `ssh` invocation — typing, full-screen output, resize, interrupts, and remote exit status — with default host-key verification and byte-clean piped use.

**Independent Test**: Per spec US3 — open a session via the command and via the equivalent manual SSH invocation, exercise interactive input, full-screen output, interrupt keys, and a remote command with a known exit status, and verify both behave identically.

- [ ] T015 [US3] Harden the session-fidelity contract in `crates/cli/src/commands/ssh.rs`: stdio fully inherited with no piping through the parent, no Ctrl-C interception around `child.wait()` (interrupts inside the session belong to the remote side), exit code propagated verbatim (remote `N` → CLI `N`), no success-path output anywhere (selector/elevation prompts excepted) so piped remote-command use stays byte-clean, host-key verification left at OpenSSH defaults (first connect prompts, accepted keys remembered) with an explicit no-bypass rule (depends on T013)
- [ ] T016 [US3] Add fidelity-contract tests in `crates/cli/src/commands/ssh.rs` (`#[cfg(test)]`): spawn configuration inherits all three stdio handles, argv contains no `StrictHostKeyChecking`/`UserKnownHostsFile` options under any input, success path produces no stdout text (byte-clean pipes), exit-code mapping passes remote status through verbatim (depends on T015)

**Checkpoint**: All three user stories independently functional — resolution, selection, and native-fidelity session form one coherent command.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Gates, contract compliance, and end-to-end validation.

- [ ] T017 Run `cargo fmt --all -- --check` from repository root and apply formatting to touched files (`crates/sdk/src/domain/microvm.rs`, `crates/sdk/src/lib.rs`, `crates/sdk/src/manager.rs`, `crates/cli/src/cli.rs`, `crates/cli/src/commands/mod.rs`, `crates/cli/src/commands/ssh.rs`, `crates/cli/src/error.rs`, test files)
- [ ] T018 Run workspace gates from repository root: `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features` (depends on T017)
- [ ] T019 [P] Validate quickstart scenarios 1–7 in `specs/010-cli-ssh-command/quickstart.md` (selector with mixed states, explicit name, elevation round-trip carrying the remote command, remote-command exit propagation, stale-state absence, all failure shapes, manual fidelity comparison against `ssh -i {key} {user}@{address}` plus host-key first-connect prompt) — live-session scenarios with rights on a KVM host; record any deviation (depends on all story phases)
- [ ] T020 Verify contract compliance against `specs/010-cli-ssh-command/contracts/cli-ssh.md`: exactly one additive SDK operation, no repository trait change, no schema migration, no new dependency, no CLI-side SQLite, no key contents in output, no host-key bypass options, `--non-interactive` performs zero prompts (depends on T018)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — starts immediately.
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories.
- **User Stories (Phases 3–5)**: All depend on Foundational completion.
  - US1 → US2 → US3 in priority/dependency order (US2–US3 extend the same `ssh.rs` flow US1 creates; run sequentially, not parallel across stories).
  - Each story is independently testable per its checkpoint once its phase completes.
- **Polish (Phase 6)**: Depends on all desired user stories being complete.

### User Story Dependencies

- **US1 (P1)**: After Foundational — no other story dependency. Delivers the MVP slice (named shell + named remote command).
- **US2 (P1)**: After Foundational + US1 (adds the selector branch in the file US1 creates).
- **US3 (P2)**: After Foundational + US1 + US2 (hardens the spawn path both prior stories share).

### Within Each User Story

- Core implementation before tests in the same file chain (e.g., T010 before T011; T013 before T014; T015 before T016).
- Tests in different files (unit in `ssh.rs` vs surface in `command_surface.rs`) run in parallel once the implementation they cover lands.
- Story checkpoint validated before moving to the next priority.

### Parallel Opportunities

- Phase 1: T002 parallel with T001.
- Phase 2: T006 (SDK tests), T007 (`cli.rs` args), T009 (`error.rs`) parallel with each other and with the T003→T004 / T003→T005 chains (different files, no overlap).
- US1: T011 (unit tests in `ssh.rs`) parallel with T012 (surface test file).
- Polish: T019 (manual validation) parallel with T018 gate runs on separate checkouts.

---

## Parallel Example: Foundational Phase

```bash
# SDK chain (sequential, same layering):
Task T003: "Add RunningMicroVm in crates/sdk/src/domain/microvm.rs"
  → Task T004: "Re-export in crates/sdk/src/lib.rs"
  → Task T005: "Implement list_running_microvms in crates/sdk/src/manager.rs"
  → Task T006: "SDK listing tests in crates/sdk/src/manager.rs"

# In parallel with the chain (different files, no dependencies):
Task T007: "Add SshArgs in crates/cli/src/cli.rs"
Task T009: "Add constructors in crates/cli/src/error.rs"
```

## Parallel Example: User Story 1

```bash
# After T010 lands:
Task T011: "Unit tests in crates/cli/src/commands/ssh.rs"
Task T012: "Surface tests in crates/cli/tests/command_surface.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational (CRITICAL — running listing + CLI scaffolding).
3. Complete Phase 3: US1 (named resolution + escalation + pre-flight + silent spawn).
4. **STOP and VALIDATE**: named shell and named remote command against live running VMs; `cargo test -p taumaru-microvm-cli`, `cargo test -p taumaru-microvm`.
5. Deploy/demo if ready — selector (US2) and fidelity hardening (US3) follow.

### Incremental Delivery

1. Setup + Foundational → running data and CLI surface ready.
2. US1 → named connect works (MVP).
3. US2 → bare `ssh` selects from running machines only.
4. US3 → session fidelity pinned and byte-clean pipes verified.
5. Each increment preserves prior behavior (no rewording of existing messages, no SDK lifecycle change, SDK change stays one additive read).

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together (one owner for the T003→T006 SDK chain, others on T007–T009 scaffolding).
2. Once Foundational is done:
   - Developer A: US1 → US2 → US3 (sequential, same file).
   - Developer B: test support and quickstart validation prep.
3. Merge in story order; run full workspace gates before Polish sign-off.

---

## Notes

- [P] tasks = different files, no dependencies; same-file story extensions (T010→T013→T015, T011→T014→T016) are intentionally sequential.
- [Story] label maps each story-phase task to its spec user story for traceability.
- `microvm stop` / `list` / `status` / `inspect` remain documentation pointers — any task implementing them is out of scope and must be rejected.
- Commit after each task or logical group; stop at any checkpoint to validate the story independently.
- Avoid: CLI-side SQLite access, CLI-side liveness probing, second naming rule, trusted-values envelope for ssh, piped stdio, Ctrl-C interception around the session, host-key bypass options, success-path output, key contents in output.
