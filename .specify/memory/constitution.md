<!--
Sync Impact Report
- Version change: 1.2.0 -> 1.3.0
- Modified principles:
  - IV. Closed for Modification, Open for Extension -> expanded with mandatory SDK and CLI
    repository organization and host data-directory ownership rules
- Added sections:
  - Repository Organization under Architecture & Product Boundaries
  - Host Data Directory under Repository Organization
- Removed sections: None
- Follow-up TODOs: None
-->

# Taumaru MicroVM Constitution

## Core Principles

### I. SDK-First Shared Core

All host-local MicroVM lifecycle behavior MUST be implemented behind the Rust SDK and its
shared core. The CLI MUST consume that SDK for creation, configuration, startup, shutdown,
reboot, deletion, listing, inspection, and status; it MUST NOT implement a second lifecycle
engine. The SDK MUST remain independently usable and independent from Taumaru product
orchestration, so it can serve the Taumaru Agent, standalone Rust applications, and the
published crates.io package. This boundary keeps behavior consistent across every consumer.

### II. Panic-Free, Silent SDK Boundary

The public SDK contract MUST be panic-free for expected invalid input, missing artifacts,
registry and network failures, database failures, process failures, and lifecycle conflicts.
These conditions MUST be returned as typed `Result` errors with enough context for the caller
to decide what to do. SDK operations MUST NOT terminate the host process, write to stdout or
stderr, emit logs or tracing events, install a logging subscriber, or hide behavior in global
side effects. User-controlled and host-controlled failure paths MUST NOT rely on `unwrap`,
`expect`, `panic!`, or assertion macros. The CLI owns diagnostics and presentation; this
separation makes the SDK safe to embed in applications with their own policies.

### III. Explicit Local State and Lifecycle

The SDK MUST maintain durable local metadata for every managed MicroVM, including its
identity, configuration, artifact references, runtime paths, process information, and current
lifecycle state as applicable. A local database MUST be the source of truth for this host-local
inventory, while remaining distinct from Taumaru's distributed product state. Lifecycle
operations MUST use explicit states, reconcile persisted metadata with actual Firecracker
process state, and be idempotent whenever the operation's semantics allow repetition. Artifact
resolution MUST support the Taumaru Artifacts Registry as the project registry for kernels,
distribution images, and supporting files, with acquisition and cache failures surfaced as
typed errors. Multiple independent MicroVMs per host are a first-class requirement.

### IV. Closed for Modification, Open for Extension

The architecture MUST be closed to unrelated modification and open to new capabilities through
stable domain boundaries, adapters, and composable policies. New artifact sources, storage
implementations, Firecracker backends, output formats, and lifecycle capabilities MUST be
addable without duplicating the engine or coupling unrelated consumers to implementation
details. `firectl` invocation and other host-process mechanics MUST remain behind an internal
abstraction so the implementation can evolve without forcing unnecessary public API changes.
Modules and functions MUST be cohesive and appropriately sized: code MUST avoid both large
multi-purpose procedures and artificial one-line wrappers that add no meaningful boundary.
Additive changes MUST preserve existing SDK behavior; breaking changes require explicit
versioning and migration guidance.

### V. Calm, Accessible CLI and Intentional Documentation

The CLI MUST be a modern, fast, scriptable operational surface over the SDK, with presentation
logic kept separate from lifecycle logic. Its interaction and visual language MUST follow the
Taumaru Design System: restrained Zinc/Vega foundations, semantic colors used only for
operational meaning, clear status labels, compact information density, progressive disclosure,
immediate feedback, and calm failure states. State MUST NOT be communicated by color alone,
and errors MUST explain what happened, why it matters, and what the user can do next. Public
SDK items and behaviorally important contracts MUST be documented with Rustdoc. CLI code MUST
avoid comments when names and structure can explain the behavior; comments are reserved for
constraints or intent that cannot be expressed naturally in the code.

English is the sole language of project artifacts. All source code, identifiers, comments,
Rustdoc, CLI commands and output, error messages, documentation, tests, fixtures, examples,
configuration, generated files, release notes, and other repository text MUST be written in
English, regardless of the language used in a prompt or request. Prompt language MUST NOT
propagate into implementation artifacts or user-facing product text. Localization or
translation is a separate feature and requires explicit approval.

## Architecture & Product Boundaries

`taumaru-microvm` is an independently usable open-source Rust project for managing
Firecracker MicroVMs on one host. The workspace MUST expose two distinct crates:

- The SDK crate contains the reusable core, is intended for publication on crates.io, and is
  the only lifecycle implementation consumed by other Rust code.
- The CLI crate consumes the SDK, is not published on crates.io, and is distributed as an
  installable binary.

The exact crate and binary names may evolve, but this dependency direction MUST remain intact:
consumers depend on the SDK, and the SDK MUST NOT depend on the CLI, the Taumaru Agent, or
Taumaru's distributed control services. The project MUST NOT take ownership of authentication,
Control Plane coordination, cloud scheduling, failover decisions, routing, billing, or other
Taumaru product concerns that are outside host-local MicroVM management.

The Taumaru Artifacts Registry at `https://artifacts.taumaru.com/v1/` is the canonical registry
integration for kernels, distribution images, and other artifacts required to create a
MicroVM. Registry access MUST be isolated behind a replaceable boundary and MUST support clear
handling of local, cached, unavailable, and incompatible artifacts.

The local database MUST store the inventory and runtime metadata needed to manage existing
MicroVM processes on the host. It is not a substitute for Taumaru's authoritative distributed
state. Firecracker and `firectl` details MUST stay inside the MicroVM layer, and callers MUST
use typed SDK operations rather than assembling host commands themselves.

### Repository Organization

The workspace MUST use the following ownership-oriented layout as the default organization for
SDK and CLI production code:

```text
crates/
├── sdk/
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── error.rs
│   │   ├── manager.rs
│   │   ├── domain/
│   │   │   ├── mod.rs
│   │   │   ├── microvm.rs
│   │   │   ├── config.rs
│   │   │   ├── lifecycle.rs
│   │   │   └── artifact.rs
│   │   ├── ports/
│   │   │   ├── mod.rs
│   │   │   ├── repository.rs
│   │   │   ├── artifacts.rs
│   │   │   └── runtime.rs
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
    │   ├── main.rs
    │   ├── cli.rs
    │   ├── context.rs
    │   ├── error.rs
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

#### Host Data Directory

The CLI MUST resolve the host-local base directory and pass the resolved path explicitly to the
SDK during dependency wiring. When `TAUMARU_HOME` is unset, the CLI MUST use the user's home
directory default `~/.taumaru-microvm`. A custom base directory MUST be selected only through
the `TAUMARU_HOME` environment variable; the CLI MUST NOT expose this setting through a command-
line option or positional argument.

The SDK MUST NOT discover or override the base directory by reading `HOME`, `TAUMARU_HOME`, or
another environment variable. It MUST receive the base directory from its caller and own the
layout and behavior of everything below that directory, including local state, downloaded
artifacts, tool binaries, runtime files, locks, temporary files, cache entries, and cleanup.

The SDK-owned directory MAY evolve as capabilities are added, but it MUST preserve clear
separation between these categories:

```text
<taumaru-home>/
├── state/
│   └── inventory.db
├── artifacts/
│   ├── kernels/
│   ├── rootfs/
│   └── supporting/
├── tools/
│   ├── firecracker/
│   └── firectl/
├── runtime/
│   └── <vm-id>/
├── cache/
└── tmp/
```

The SDK MUST create required directories on demand, apply appropriate local permissions, use
safe and atomic file operations where applicable, and return typed errors for path, permission,
download, integrity, locking, and storage failures. The CLI owns only path resolution and
user-facing diagnostics; it MUST NOT implement or duplicate this directory management.

The SDK layout MUST follow these ownership rules:

- `src/lib.rs` is the public facade and MUST expose stable public types and operations through
  deliberate re-exports rather than serving as a dumping ground for implementation logic.
- `error.rs` owns the public typed error surface, while `manager.rs` coordinates lifecycle use
  cases without exposing infrastructure details.
- `domain/` contains MicroVM identities, configuration, artifacts, and lifecycle states without
  direct filesystem, database, network, terminal, or process dependencies.
- `ports/` contains replaceable abstractions for local persistence, artifact sources, and runtime
  process control.
- `adapters/` contains infrastructure implementations such as SQLite, the Taumaru registry,
  and Firecracker. Firecracker and `firectl` mechanics MUST remain in the runtime adapter.

The CLI layout MUST follow these ownership rules:

- `main.rs` owns startup, top-level parse handling, and exit-code mapping; it MUST remain thin.
- `cli.rs` owns Clap definitions, `context.rs` owns dependency wiring, and `error.rs` owns
  user-facing error presentation.
- Each `commands/` module MAY call the SDK for one cohesive command, but MUST NOT implement
  lifecycle behavior or invoke Firecracker directly.
- `output/` owns human and machine-readable presentation. Formatting MUST NOT change SDK domain
  semantics.

Unit tests SHOULD remain close to the module they exercise. SDK public-contract and failure-path
tests belong in `crates/sdk/tests/`, while executable and command-surface tests belong in
`crates/cli/tests/`. New directories and modules MUST be introduced when their first real
capability requires them; the repository MUST NOT add speculative empty layers.

## Development Workflow & Quality Gates

Every feature or behavior change MUST define its observable success, failure, and lifecycle
state transitions before implementation. The change MUST preserve the two-crate boundary and
identify whether it affects the SDK contract, the CLI presentation contract, local persistence,
artifact integration, or the Firecracker process boundary.

Before merge, the project MUST provide tests appropriate to the change, including:

- unit tests for domain rules, validation, state transitions, and typed errors;
- integration or contract tests for the registry, local database, process adapter, and
  SDK-to-CLI boundary when those interfaces change;
- failure-path tests demonstrating that public SDK operations return errors without panic or
  unsolicited output; and
- CLI tests for meaningful output, exit behavior, status semantics, and destructive-action
  safeguards when the CLI changes.

The standard Rust quality gates MUST pass: formatting, compilation, linting with warnings
treated as errors, and the relevant test suite. Reviewers MUST verify that lifecycle behavior
was not duplicated, SDK side effects were not introduced, public SDK changes have Rustdoc and
compatibility notes, and CLI changes preserve the Design System's calm, semantic, and
keyboard-friendly interaction model. Reviewers MUST also verify that all new project artifacts
and user-facing text are written in English. Any exception MUST be documented with its reason
and scope in the change proposal.

## Governance

This constitution governs architecture, public interfaces, runtime side effects, quality
expectations, and CLI interaction for `taumaru-microvm`. It is authoritative when repository
practice conflicts with these principles. A feature specification, implementation plan, task
set, or code review MUST identify the principles and boundaries it affects.

Amendments MUST state the motivation, affected principles, compatibility impact, migration or
rollout needs, and validation plan. The amended constitution MUST include an updated Sync
Impact Report during review, and the temporary report MUST be removed before the amendment is
committed. Changes to application behavior that follow from an amendment belong in their own
feature specification and MUST NOT be smuggled into the constitution update.

Versioning follows Semantic Versioning for governance:

- MAJOR increments remove or redefine a principle in a backward-incompatible way.
- MINOR increments add a principle or materially expand a governed section.
- PATCH increments clarify wording, fix errors, or make non-semantic refinements.

Compliance MUST be reviewed before implementation and again during code review. Non-compliant
changes MUST be corrected before merge or carry a documented exception with a responsible owner,
scope, and expiry or follow-up plan. The constitution itself MUST be amended when a temporary
exception becomes a permanent architectural or product rule.

**Version**: 1.3.0 | **Ratified**: 2026-09-17 | **Last Amended**: 2026-09-17
