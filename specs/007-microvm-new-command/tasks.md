# Tasks: CLI `new` Command for Guided MicroVM Creation

**Input**: Design documents from `/specs/007-microvm-new-command/`

**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: Included per the constitution's quality gates (unit tests for domain rules and typed errors, integration/contract tests for the SDK-to-CLI boundary, failure-path tests proving no panic or unsolicited output, CLI tests for output and exit behavior) and the spec's Independent Test criteria per story. This is not optional-test padding: every test task below maps to a spec acceptance scenario or a constitution-mandated gate.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **SDK library**: `crates/sdk/src/`, SDK tests in `crates/sdk/tests/`
- **CLI binary**: `crates/cli/src/`, CLI executable tests in `crates/cli/tests/`
- Feature docs: `specs/007-microvm-new-command/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Baseline verification and fixture confirmation before any code changes.

- [x] T001 Verify clean baseline gates for the `Cargo.toml` workspace from repository root (`cargo fmt --all -- --check`, `cargo check --all-targets --all-features`)
- [x] T002 [P] Confirm fixture coverage in `crates/sdk/tests/fixtures/manifest.json` (test kernels, binary packages with `firecracker`/`firectl` components, one distribution with servable image payloads) with payload routes in `crates/sdk/tests/support/mod.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Additive read-only SDK readiness query plus reuse wiring for the download catalog that MUST be complete before ANY user story work begins. Existing download, persistence, and creation behavior stays untouched.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [x] T003 Implement `is_distribution_image_ready(&self, distribution_id: &str, image_id: &str) -> Result<bool, SdkError>` in `crates/sdk/src/manager.rs`, reusing the private `resolve_distribution_image` plus file-integrity recheck path (unknown distribution yields `NotFound { kind: "distribution" }`; image absent from that distribution yields `NotFound { kind: "distribution image" }`; `Ok(true)` only for a present inventory relationship with matching size and digest, `Ok(false)` for missing/incomplete/stale; no download, mutation, repair, print, log, or global state; Rustdoc documents the same invariants)
- [x] T004 [P] Add readiness SDK tests in `crates/sdk/tests/download_flow.rs` (ready image returns `Ok(true)`; missing image returns `Ok(false)`; stale file with wrong size or digest returns `Ok(false)`; unknown distribution vs wrong-distribution image distinguished as typed `NotFound`; blank IDs rejected; no file is created or modified by the query)
- [x] T005 [P] Widen visibility of reusable download-catalog items to `pub(crate)` in `crates/cli/src/commands/download.rs` with zero behavior change (`RegistryCatalog`, `load_catalog`, `parse_image_selections`, `prompt_render_config`, `prompt_error`, `call_with_signal`, `ProgressForwarder`, `DownloadProgressView`, `availability_for_files`, runtime-selection and checked-arithmetic helpers)
- [x] T006 Run focused SDK gates for `crates/sdk/` from repository root (`cargo test -p taumaru-microvm --all-targets --all-features`)

**Checkpoint**: Foundation ready — readiness query verified against fixtures and catalog helpers importable from the new command; user story implementation can now begin.

---

## Phase 3: User Story 1 - Guided Interactive Creation (Priority: P1) 🎯 MVP

**Goal**: `microvm new [NAME]` collects name, single image, disk, memory, vCPUs, and network mode through guided prompts (prompting only for missing values), validates with abort-on-invalid, and ends with a creation summary plus explicit confirmation before any transfer.

**Independent Test**: Run against a deterministic registry fixture using keyboard-only input (type or accept a name, pick exactly one image, enter disk/memory/vCPU values, answer the network question, confirm) and verify the confirmation summary matches the entered values before any transfer begins.

- [x] T007 [US1] Add `Command::New(NewArgs)` with optional positional `name` plus `--name`, exactly-once `--image DISTRIBUTION=IMAGE`, `--disk-gb`, `--memory`, `--vcpus`, `--expose-lan`, `--non-interactive` flags and help text in `crates/cli/src/cli.rs`
- [x] T008 [US1] Route the new `Command::New` dispatch to the creation flow in `crates/cli/src/commands/mod.rs`
- [x] T009 [US1] Create `crates/cli/src/commands/new.rs` with `NewVmRequest { name, distribution_id, image_id, disk_size_bytes, memory_bytes, vcpu_count, expose_on_lan }` plus pure parsing functions (name rule 1–64 ASCII starting alphanumeric; disk trimmed decimal `f64` to ceiling GB × 1024³ with overflow check; memory case-insensitive decimal `<amount><MB|GB>` with optional space to ceiling bytes with overflow check; vCPUs trimmed `u32`; image `DISTRIBUTION=IMAGE` with non-blank sides; positional/`--name` agreement) and request assembly against the shared catalog (constraint, quoted from data-model: "The pair `(distribution_id, image_id)` is the image key; display names never identify a row."; "`lan_address` is always `None` and `volume_path` is always `Non…
- [x] T010 [US1] Implement the fixed-order interactive collection in `crates/cli/src/commands/new.rs` using `inquire` (`Text` name/disk/memory/vCPU, single `Select` image with `downloaded`/`needs download` text marker from `is_distribution_image_ready`, `Confirm` default-false LAN exposure, `Confirm` default-false creation question) with abort-on-invalid and no re-prompt loop, prompting only for values not already supplied
- [x] T011 [P] [US1] Render the creation summary (name, image with parent distribution, disk, memory, vCPU count, network mode, prerequisites still to fetch) with shared `paint`/`divider`/`format_bytes` styling in `crates/cli/src/output/human.rs`
- [x] T012 [US1] Cover name/disk/memory/vCPU parsing, GB/MB minimum display, `1.5GB` acceptance with round-up, `--image` form validation, positional/`--name` agreement, and sub-minimum abort with the minimum shown in the tests module of `crates/cli/src/commands/new.rs`

- [x] T013 [P] [US1] Assert `microvm new --help` exposes `[NAME]`, `--image <DISTRIBUTION=IMAGE>`, `--disk-gb`, `--memory`, `--vcpus`, `--expose-lan`, `--non-interactive` and no `--kernel`/`--volume-path`/`--lan-address` flags in `crates/cli/tests/command_surface.rs`
**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently (collection through confirmation with zero transfers started before confirmation).

---

## Phase 4: User Story 2 - Automatic Provisioning With Live Per-Step Progress (Priority: P1)

**Goal**: Confirmed request provisions the runtime bundle first, then the distribution default kernel and the selected image, then calls `create_microvm` once — with one live row per step, truthful counters, reuse labeled distinctly, and no download plan shown.

**Independent Test**: Confirm a creation whose runtime bundle and image are missing, then confirm the same creation with a warm cache; verify dependency-order transfers with truthful byte progress first, reuse without fresh transfers second, and a configured stopped VM in both cases.

- [x] T014 [US2] Execute post-confirmation provisioning in dependency order in `crates/cli/src/commands/new.rs` through the cancellation-aware SDK variants with one shared `DownloadCancellation` (sorted runtime packages, then the single resolved default kernel, then the single image; runtime failure stops all dependent work; only all-verified prerequisites reach creation) and invoke `create_microvm` with `Some(observer)` mapping `CreationProgress` (stage, `N/6` counters, `overall_percent`, phase, byte counters, terminal outcome) into the renderer (constraint, quoted from data-model: "Progress derives only from SDK counters plus member metadata — never from elapsed time or assumed rates.")
- [x] T015 [P] [US2] Render sequential provisioning lines (`runtime/{package}`, `kernel/{id}`, `image/{distribution}/{image}` with stage, member and aggregate bytes, disposition) plus creation lines (`creation/{stage}` with `N/6`, `overall_percent`, phase, outcome) and the final creation summary (identity, network mode with address, volume location, resource sizes, SSH account/port/private-key path, never key contents) in `crates/cli/src/output/human.rs`
- [x] T016 [US2] Cover runtime-first ordering, single kernel/image calls, verified-reuse labeling without second transfer, `overall_percent`-driven creation bar, and no-plan-table invariant with a recording SDK-shaped fake in the tests module of `crates/cli/src/commands/new.rs`

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently (collect, provision in order, create a configured stopped VM with live progress).

---

## Phase 5: User Story 3 - Non-Interactive Automation (Priority: P2)

**Goal**: A complete explicit flag set provisions and creates with no prompts and deterministic output; any missing or invalid value fails fast with usage guidance before any transfer or mutation.

**Independent Test**: Invoke without a terminal with the complete explicit set, then repeat with one value omitted; verify no prompts and deterministic status first, then a missing-value error with zero transfers and no VM second.

- [x] T017 [US3] Gate explicit versus interactive modes in `crates/cli/src/commands/new.rs` (any explicit creation flag or `--non-interactive` disables fallback prompts for supplied values; `--non-interactive` requires name, exactly one `--image`, `--disk-gb`, `--memory`, `--vcpus`, skips confirmation, performs zero prompts, and reports the first missing or invalid value with usage guidance before any transfer or mutation; non-TTY without a complete set errors with the required flag form and an example)
- [x] T018 [US3] Cover complete-explicit deterministic runs, omitted-value rejection with no prompts, invalid-value rejection with the applicable minimum or expected format, and non-TTY guidance in the tests module of `crates/cli/src/commands/new.rs`
- [x] T019 [P] [US3] Assert `microvm new --non-interactive` without values fails without prompting and a complete explicit fixture-backed run stays deterministic in `crates/cli/tests/command_surface.rs`

**Checkpoint**: All collection modes independently functional over both interactive and scripted input.

---

## Phase 6: User Story 4 - Calm Failures and Safe Retry (Priority: P2)

**Goal**: Identical repeats report already-configured without duplicates; name conflicts leave the existing VM unchanged; provisioning failures preserve verified work; creation-phase interrupts settle before reporting; every failure explains what happened, why no VM was created, and what to do next.

**Independent Test**: Fail the runtime bundle, fail the kernel, reuse an identical name, and collide with a different configuration; verify exit statuses, separated group summaries, preserved prerequisites, no partial or duplicate VM, and `130` with no published partial on provisioning interrupt.

- [x] T020 [US4] Handle creation outcomes in `crates/cli/src/commands/new.rs` (`Completed` success summary; `AlreadyConfigured` identical-repeat report with no duplicate work; `ConfigurationConflict`/`LifecycleConflict` name-conflict report leaving the existing VM unchanged; kernel/image failure creating no VM; provisioning interrupt via the shared token with partial-file removal, verified-work preservation, affected-group report, exit `130`; creation-phase interrupt flag that awaits settlement and reports the settled outcome plus the interruption note, never a fabricated cancellation)
- [x] T021 [P] [US4] Add new-command error messaging in `crates/cli/src/error.rs` without changing any existing download string ("MicroVM creation input is invalid" with rule/minimum plus flag-or-prompt next step; "MicroVM creation cancelled" with `microvm new` retry guidance; SDK-error reasons reuse typed `SdkError` text with a verified-prerequisites-reused next step; three-part what/why/next shape; exit `130` for prompt/confirmation/provisioning cancellation)
- [x] T022 [US4] Cover identical-repeat already-configured, name-conflict unchanged, kernel-failure no-VM, provisioning-cancel `130` with preserved groups, and creation-interrupt settle-then-report with a recording fake in the tests module of `crates/cli/src/commands/new.rs`

**Checkpoint**: Failure, conflict, and cancellation behavior independently verified.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: String audit, boundary conformance, and full-gate validation.

- [x] T023 Remove or forbid out-of-scope paths (`--kernel`, `--volume-path`, `--lan-address`, multi-image, lifecycle operations) at argument parsing with guidance in `crates/cli/src/cli.rs` and `crates/cli/src/commands/new.rs`
- [x] T024 [P] Validate English-only user-facing strings, text-plus-symbol status (never color-only), and non-color/narrow-terminal rendering across `crates/cli/src/commands/new.rs`, `crates/cli/src/error.rs`, and `crates/cli/src/output/human.rs`
- [x] T025 [P] Confirm the package boundary still holds in `crates/cli/tests/package_boundary.rs` (CLI depends on the SDK path only; no direct `reqwest`/`rusqlite`/`sha2`/Firecracker dependency)
- [x] T026 Run the validation guide in `specs/007-microvm-new-command/quickstart.md` (guided flow, explicit flow, failure checks) followed by full workspace gates from repository root (`cargo fmt --all -- --check`, `cargo check --all-targets --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`)

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately.
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories (CLI work needs the SDK query compiled and fixture-tested plus catalog helpers importable first).
- **User Stories (Phases 3–6)**: All depend on Foundational completion; run sequentially in priority order (US1 → US2 → US3 → US4) because Phases 3–6 share `crates/cli/src/commands/new.rs` and same-file edits must not run in parallel.
- **Polish (Phase 7)**: Depends on all user story phases being complete.

### User Story Dependencies

- **User Story 1 (P1)**: Starts after Foundational — no dependencies on other stories.
- **User Story 2 (P1)**: Depends on US1 request/selection types (`NewVmRequest`, single-image resolution) but is independently testable via the recording fake.
- **User Story 3 (P2)**: Depends on US1 collection validation and US2 request shape; independently testable through explicit-mode tests.
- **User Story 4 (P2)**: Depends on US2 execution loop; independently testable through failure-injection tests.

### Within Each User Story

- Tests live in the same module/file as the behavior they cover (repo convention: unit tests close to the module); write the test first, watch it FAIL, then implement.
- Request/parsing types before prompts; prompts before review; review before execution.
- Core implementation before failure-continuation wiring.
- Story checkpoint validated before moving to the next priority.

### Parallel Opportunities

- T002 can run alongside any Setup/Foundational work (read-only fixture check).
- T004 and T005 can run in parallel (different files: SDK tests vs `download.rs` visibility).
- T011 can run alongside T009/T010 (different file: `human.rs` vs `new.rs`).
- T013 can run alongside US1 implementation (different file: `command_surface.rs`).
- T015 can run alongside T014/T016 (different file: `human.rs` vs `new.rs`).
- T019 can run alongside T017/T018 (different file: `command_surface.rs` vs `new.rs`).
- T021 can run alongside T020/T022 (different file: `error.rs` vs `new.rs`).
- T024 and T025 can run alongside T023 (different files).
- NEVER parallelize two tasks editing `crates/cli/src/commands/new.rs` simultaneously (T009/T010, T012, T014, T016, T017/T018, T020, T022, T023).
- NEVER parallelize two tasks editing `crates/sdk/src/manager.rs` with another SDK edit simultaneously (only T003 touches it — no conflict by construction).

---

## Parallel Example: User Story 1

```bash
# T009+T010 sequential (same file), T011 in parallel with them (different file):
Task: "Create NewVmRequest plus parsing functions in crates/cli/src/commands/new.rs"   # T009, then T010 same file
Task: "Render the creation summary in crates/cli/src/output/human.rs"                  # T011 [P] alongside

# Tests across files in parallel:
Task: "Cover parsing and minimums in crates/cli/src/commands/new.rs tests module"      # T012
Task: "Assert new-command help surface in crates/cli/tests/command_surface.rs"         # T013 [P] alongside T012
```

## Parallel Example: Foundational SDK

```bash
# Sequential (method before its gates):
Task: "Implement is_distribution_image_ready in crates/sdk/src/manager.rs"              # T003
Task: "Run focused SDK gates from repository root"                                      # T006, after T003-T005

# Then in parallel (different files, same contract):
Task: "Add readiness SDK tests in crates/sdk/tests/download_flow.rs"                   # T004 [P]
Task: "Widen download-catalog visibility in crates/cli/src/commands/download.rs"       # T005 [P]
```

---

## Implementation Strategy

### MVP First (User Story 1 + Foundational)

1. Complete Phase 1: Setup (baseline + fixture confirmation).
2. Complete Phase 2: Foundational (SDK readiness query fixture-tested, catalog helpers importable).
3. Complete Phase 3: User Story 1 (collect + confirm under the new entry, no transfers yet wired beyond the marker query).
4. **STOP and VALIDATE**: parsing, minimums, single-image select with markers, confirmation summary, and help surface independently.
5. Deploy/demo if ready (collection-only confidence before touching provisioning).

### Incremental Delivery

1. Setup + Foundational → readiness query ready, catalog reusable.
2. Add US1 → collect and confirm a valid request (MVP!).
3. Add US2 → provision in order and create with live progress (operational payoff).
4. Add US3 → scripted explicit automation with deterministic output.
5. Add US4 → calm conflicts, preserved-work failures, and `130`/settle-then-report interrupts.
6. Polish → out-of-scope guards, string audit, boundary check, full gates.
7. Each increment adds value without breaking previous stories.

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together (T003 chain, then T004/T005 split across two developers).
2. Single developer takes Phases 3–6 sequentially (shared `new.rs` forbids splitting by story across developers without merge conflicts).
3. A second developer can own `human.rs` tasks (T011, T015), `error.rs` (T021), and `command_surface.rs` tasks (T013, T019, T025) plus the string audit (T024) in parallel with the `new.rs` owner, integrating at each story checkpoint.

---

## Notes

- [P] tasks = different files, no same-file conflicts.
- [Story] label maps each task to its spec user story for traceability.
- Each user story is independently completable and testable via its Independent Test.
- Verify tests fail before implementing (TDD per repo convention).
- Commit after each task or logical group.
- Stop at any checkpoint to validate the story independently.
- Avoid: vague tasks, same-file parallel edits, cross-story dependencies that break independence.
- Data-model constraints quoted verbatim in T009 and T014 — implementation must not reinterpret them.
- Constitution trace: T003–T006 (Principles I–IV: SDK-first, silent/typed, explicit state, additive-only); T007–T026 (Principle V: calm/accessible CLI, English-only strings in T024); T026 (quality gates: fmt, check, clippy `-D warnings`, full test suite).
