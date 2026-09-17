# Taumaru MicroVM Agent Guide

This file contains the working conventions for contributors and coding agents in the
`taumaru-microvm` repository. The project constitution at `.specify/memory/constitution.md` is
the governing source of truth. These instructions make that constitution operational during
repository work.

## Language Policy

English is the only language for project artifacts, regardless of the language used in a prompt
or request. Write all source code, identifiers, comments, Rustdoc, CLI commands and output,
error messages, documentation, tests, fixtures, examples, configuration, generated files,
release notes, commit-facing project text, and other repository content in English. Prompt
language MUST NOT propagate into implementation artifacts or user-facing product text.

## Project Mission

`taumaru-microvm` is an independently usable, open-source Rust project for managing Firecracker
MicroVMs on a host. It integrates with the Taumaru Artifacts Registry for kernels, distribution
images, and supporting artifacts, and stores the local inventory and runtime metadata required
to manage multiple MicroVMs.

The project is a host-local infrastructure primitive. It is not responsible for Taumaru
authentication, Control Plane coordination, cloud scheduling, failover decisions, routing,
billing, or other distributed product concerns.

## Repository Shape and Dependency Direction

The workspace has two distinct crates:

- The SDK crate contains the reusable core, is intended for publication on crates.io, and is
  consumed by Rust applications such as the Taumaru Agent.
- The CLI crate consumes the SDK, is not published on crates.io, and is distributed as an
  installable binary.

The SDK is the single implementation of MicroVM lifecycle behavior. The CLI is a presentation
and interaction layer over the SDK; it MUST NOT contain a separate lifecycle engine or duplicate
Firecracker orchestration.

Dependency direction MUST flow toward the SDK. The SDK MUST NOT depend on the CLI, the Taumaru
Agent, or Taumaru's distributed control services. Callers use typed SDK operations rather than
assembling `firectl` or Firecracker commands themselves.

## Repository Organization

Use the following ownership-oriented layout as the default organization for SDK and CLI
production code:

```text
crates/
├── sdk/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs              # Public facade and re-exports
│   │   ├── error.rs            # Typed SDK errors
│   │   ├── manager.rs          # Lifecycle use cases
│   │   ├── domain/
│   │   │   ├── mod.rs
│   │   │   ├── microvm.rs      # Identity and VM metadata
│   │   │   ├── config.rs
│   │   │   ├── lifecycle.rs    # Explicit VM states
│   │   │   └── artifact.rs
│   │   ├── ports/
│   │   │   ├── mod.rs
│   │   │   ├── repository.rs   # Local state abstraction
│   │   │   ├── artifacts.rs    # Artifact source abstraction
│   │   │   └── runtime.rs      # Firecracker/process abstraction
│   │   └── adapters/
│   │       ├── persistence/
│   │       │   └── sqlite.rs
│   │       ├── registry/
│   │       │   └── taumaru.rs
│   │       └── runtime/
│   │           └── firecracker.rs
│   └── tests/
│       ├── public_api.rs
│       ├── lifecycle.rs
│       └── failure_paths.rs
└── cli/
    ├── Cargo.toml
    ├── src/
    │   ├── main.rs            # Startup, parse handling, and exit codes
    │   ├── cli.rs             # Clap definitions
    │   ├── context.rs         # SDK and adapter wiring
    │   ├── error.rs           # User-facing error formatting
    │   ├── commands/
    │   │   ├── mod.rs
    │   │   ├── create.rs
    │   │   ├── configure.rs
    │   │   ├── start.rs
    │   │   ├── stop.rs
    │   │   ├── reboot.rs
    │   │   ├── delete.rs
    │   │   ├── list.rs
    │   │   ├── inspect.rs
    │   │   └── status.rs
    │   └── output/
    │       ├── mod.rs
    │       ├── human.rs
    │       └── json.rs
    └── tests/
        └── command_surface.rs
```

The SDK layout has these responsibilities:

- `src/lib.rs` is the public facade. It exposes stable types and operations through deliberate
  re-exports and does not become a dumping ground for implementation logic.
- `error.rs` owns the public typed error surface, while `manager.rs` coordinates lifecycle use
  cases without exposing infrastructure details.
- `domain/` contains MicroVM identities, configuration, artifacts, and lifecycle states without
  direct filesystem, database, network, terminal, or process dependencies.
- `ports/` contains replaceable abstractions for local persistence, artifact sources, and runtime
  process control.
- `adapters/` contains infrastructure implementations such as SQLite, the Taumaru registry, and
  Firecracker. Firecracker and `firectl` mechanics remain in the runtime adapter.

The CLI layout has these responsibilities:

- `main.rs` owns startup, top-level parse handling, and exit-code mapping and remains thin.
- `cli.rs` owns Clap definitions, `context.rs` owns dependency wiring, and `error.rs` owns
  user-facing error presentation.
- Each `commands/` module may call the SDK for one cohesive command, but does not implement
  lifecycle behavior or invoke Firecracker directly.
- `output/` owns human and machine-readable presentation and does not change SDK domain
  semantics.

Keep unit tests close to the module they exercise. Put SDK public-contract and failure-path tests
in `crates/sdk/tests/`, and executable or command-surface tests in `crates/cli/tests/`. Add new
directories and modules when their first real capability requires them; do not add speculative
empty layers. The current bootstrap may remain minimal until a feature needs this expansion.

## SDK Rules

The SDK is an embeddable library and MUST be safe for applications with their own output,
logging, and error policies.

- Expected invalid input, missing artifacts, registry and network failures, database failures,
  process failures, and lifecycle conflicts MUST be returned as typed `Result` errors.
- Public SDK operations MUST NOT intentionally panic, terminate the host process, write to
  stdout or stderr, emit logs or tracing events, install a logging subscriber, or hide behavior
  in global side effects.
- User-controlled and host-controlled failure paths MUST NOT rely on `unwrap`, `expect`,
  `panic!`, or assertion macros.
- Public SDK types, functions, error behavior, invariants, and compatibility-sensitive
  contracts MUST have clear Rustdoc.
- The SDK MUST expose explicit lifecycle states and predictable operations for creation,
  configuration, startup, shutdown, reboot, deletion, listing, inspection, and status.
- Operations MUST be idempotent whenever their semantics allow repetition and MUST surface
  conflicts rather than silently destroying state.

The CLI owns diagnostics, formatting, prompts, progress, exit codes, and user-facing messages.
The SDK owns domain behavior, persistence, artifact resolution, process coordination, and local
MicroVM lifecycle state.

## Local State and Artifact Boundaries

The local database is the source of truth for host-local MicroVM inventory and runtime metadata.
The SDK MUST persist metadata such as identity, configuration, artifact references, runtime
paths, process information, and lifecycle state as required to manage existing processes safely. This local
state is distinct from Taumaru's authoritative distributed state.

The canonical registry integration is:

`https://artifacts.taumaru.com/v1/`

Registry access MUST live behind a replaceable adapter or boundary. The design MUST distinguish
local artifacts, cached artifacts, unavailable artifacts, and incompatible artifacts, and MUST
return acquisition or cache failures as typed SDK errors.

Firecracker and `firectl` are implementation details of the MicroVM layer. Host-process
mechanics MUST remain behind an internal abstraction so a future Firecracker integration can
evolve without unnecessary public API changes.

Multiple independent MicroVMs per host are a first-class capability. Code MUST NOT introduce a
single-VM assumption into managers, persistence, process tracking, networking, or CLI commands.

## Extensibility and Code Organization

Code MUST apply the principle "closed for modification, open for extension":

- Add artifact sources, storage implementations, Firecracker backends, output formats, and
  lifecycle capabilities through stable domain boundaries, adapters, and composable policies.
- Domain logic MUST remain independent from registry clients, databases, process runners,
  terminal rendering, and other replaceable infrastructure.
- Preserve existing SDK behavior when adding capabilities. Treat breaking public changes as
  explicit versioned migrations with compatibility and migration guidance.
- Keep modules cohesive and functions appropriately sized. Avoid both large multi-purpose
  procedures and artificial one-line wrappers that add no meaningful boundary.
- Prefer names, types, and module boundaries that explain intent before adding comments.

Taumaru Agent orchestration MUST NOT move into this repository, and the SDK MUST NOT depend on
distributed Taumaru services merely to manage a local MicroVM.

## CLI Experience

The CLI MUST make common MicroVM operations simple, fast, clear, and scriptable while remaining
a thin consumer of the SDK.

The CLI MUST follow the Taumaru Design System:

- Use restrained Zinc/Vega foundations and compact information density.
- Use semantic colors only for operational meaning; never communicate state through color alone.
- Prefer explicit status labels, small indicators, progressive disclosure, and immediate
  feedback for every user-triggered action.
- Keep failure states calm and systematic. Do not use dramatic outage language or visual chaos.
- Explain errors using three parts: what happened, why it matters, and what can be done next.
- Use precise action verbs and consistent infrastructure terminology.
- Preserve keyboard-friendly interaction and deterministic non-interactive behavior for scripts.
- Require strong confirmation for destructive actions involving persistent data; do not add
  unnecessary confirmation to routine restart, retry, or reconnect operations.

CLI code MUST avoid comments when names and structure can explain the behavior. Add a comment
only when a constraint, invariant, or external reason cannot be expressed clearly through names
and structure. The CLI may format SDK errors, but it MUST NOT change the SDK's domain semantics
or reimplement its lifecycle.

## Documentation and Comments

All repository text is English. Keep documentation, examples, fixtures, test descriptions, and
user-facing strings aligned with the same terminology.

Use Rustdoc for public SDK APIs and behaviorally important contracts. Document the rationale for
non-obvious invariants, safety boundaries, compatibility constraints, and integration behavior.
In the CLI, prefer self-explanatory code and reserve comments for genuinely irreducible context.

## Development Workflow

Before changing code:

1. Read the relevant source, tests, constitution, and existing abstractions.
2. Check the working tree and preserve unrelated user changes.
3. Define observable success, failure, and lifecycle state transitions.
4. Identify whether the change affects the SDK contract, CLI presentation, local persistence,
   registry integration, or the Firecracker process boundary.
5. For feature work, use the Spec Kit workflow as appropriate: `$speckit-specify`,
   `$speckit-plan`, `$speckit-tasks`, and `$speckit-implement`. Use `$speckit-clarify` when
   requirements are materially underspecified.

During implementation:

- Keep the SDK and CLI boundary explicit.
- Add or update tests for domain rules, validation, state transitions, typed errors, and
  affected integration boundaries.
- Test public SDK failure paths for returned errors, absence of panic, and absence of
  unsolicited output.
- Test CLI output, exit behavior, status semantics, and destructive-action safeguards when the
  CLI changes.
- Keep public API changes documented in Rustdoc and accompanied by compatibility notes.

For constitution-only work, modify only `.specify/memory/constitution.md`; do not modify its
template layers or implement deferred feature work in the same change.

## Quality Gates

Before considering a code change complete, run the relevant checks:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Reviewers and agents MUST confirm:

- lifecycle behavior is implemented once and reused by every interface;
- the SDK has no intentional panic, logging, output, process termination, or hidden global
  side effects;
- local state and registry boundaries are explicit and testable;
- the two-crate dependency direction is preserved;
- new project artifacts and user-facing text are in English;
- the CLI follows the calm, semantic, accessible Design System; and
- any exception is documented with its reason and scope.

The relevant subset of these checks is sufficient when a change does not affect every area, but
the reason for omitting a check MUST be clear from the change scope.
