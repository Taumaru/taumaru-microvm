# Implementation Plan: Bootstrap Two-Crate Workspace

**Branch**: `001-bootstrap-two-crate` | **Date**: 2026-09-17 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/001-bootstrap-two-crate/spec.md`

**Note**: This plan ends at Phase 1 design. Implementation tasks belong to `$speckit-tasks`.

## Summary

Create a virtual Cargo workspace with two members: the publishable `taumaru-microvm` SDK and
the binary-only `taumaru-microvm-cli`. The SDK will expose one pure, documented example
function. The CLI will depend on the SDK, expose the executable as `microvm`, and use Clap's
derive API for baseline help, version, and argument parsing. The foundation will stay free of
Firecracker, registry, database, and production lifecycle behavior.

## Technical Context

**Language/Version**: Rust 2024 edition; local toolchain is Rust 1.98.1. No separate MSRV is
declared by this feature.

**Primary Dependencies**: SDK uses the Rust standard library only. CLI uses `clap` 4.6 with
the `derive` feature and default help, usage, error-context, suggestion, and color features.
No additional runtime dependency is required by the baseline.

**Storage**: N/A for this feature. Local VM persistence is explicitly deferred.

**Testing**: `cargo test --workspace`, SDK integration tests, and CLI process smoke tests using
the Rust standard library. Parser behavior can also be exercised through Clap's fallible parser.

**Target Platform**: Linux hosts. Baseline help and version paths do not require KVM,
Firecracker, a registry, a database, or an existing VM.

**Project Type**: Virtual Cargo workspace containing one publishable Rust library package and
one binary-oriented CLI package.

**Performance Goals**: Help and version commands only parse arguments and render output; they
must not initialize runtime resources or contact external services. No numeric latency target is
needed for this foundation.

**Constraints**: Preserve the SDK/CLI dependency direction, keep the SDK free of panic and
output side effects, use English for every project artifact, name the executable `microvm`,
exclude the CLI from crates.io publication, and avoid unused direct dependencies.

**Scale/Scope**: Two packages, one public SDK example function, baseline CLI help/version/
empty-input/invalid-input paths, and their tests. No production MicroVM commands.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Gate | Status | Evidence |
|------|--------|----------|
| I. SDK-First Shared Core | PASS | CLI depends on the SDK; no second lifecycle engine is introduced. |
| II. Panic-Free, Silent SDK Boundary | PASS | The example function is pure and the SDK has no output, logging, or process control. |
| III. Explicit Local State and Lifecycle | PASS | Persistence and lifecycle are preserved as future boundaries and remain out of scope. |
| IV. Closed for Modification, Open for Extension | PASS | Workspace members and CLI parser leave clear extension points without speculative layers. |
| V. Calm, Accessible CLI and Intentional Documentation | PASS | `microvm` help/version use English text and Clap's terminal-aware styling. |
| Architecture & Product Boundaries | PASS | SDK publication and CLI binary distribution are separate; external product services are excluded. |
| Development Workflow & Quality Gates | PASS | Unit, integration, process smoke tests, formatting, checking, linting, and test gates are planned. |

**Gate result**: PASS. No constitution violation or complexity waiver is required.

## Project Structure

### Documentation (this feature)

```text
specs/001-bootstrap-two-crate/
├── plan.md              # This file ($speckit-plan command output)
├── research.md          # Phase 0 output ($speckit-plan command)
├── data-model.md        # Phase 1 output ($speckit-plan command)
├── quickstart.md        # Phase 1 output ($speckit-plan command)
├── contracts/           # Phase 1 output ($speckit-plan command)
└── tasks.md             # Phase 2 output ($speckit-tasks command - NOT created by $speckit-plan)
```

### Source Code (repository root)

```text
Cargo.toml                    # Virtual workspace manifest
Cargo.lock                    # Shared lockfile after dependency resolution
crates/
├── sdk/
│   ├── Cargo.toml             # Publishable SDK package: taumaru-microvm
│   ├── src/
│   │   └── lib.rs             # Public example API and Rustdoc
│   └── tests/
│       └── public_api.rs      # External-consumer SDK contract tests
└── cli/
    ├── Cargo.toml             # Binary-only package: taumaru-microvm-cli
    ├── src/
    │   └── main.rs            # `microvm` parser and entry point
    └── tests/
        └── command_surface.rs # Executable help/version/error smoke tests
```

**Structure Decision**: Use a virtual workspace because the repository has no root application
after the split. The SDK package owns the reusable public library target. The CLI package owns
the binary target and depends on the SDK through a local path dependency. The CLI's `main.rs`
stays thin; future command modules can be introduced inside the CLI package without moving
domain behavior out of the SDK.

## Design Details

### Workspace and package manifests

1. Replace the current root package manifest with a virtual workspace declaring `crates/sdk` and
   `crates/cli` as members and setting `resolver = "3"`.
2. Define shared edition, version, description, repository, and license metadata at the
   workspace level where appropriate. Keep the existing project version at `0.1.0`.
3. Name the SDK package `taumaru-microvm` and configure it as a library target intended for
   crates.io publication.
4. Name the CLI package `taumaru-microvm-cli`, set `publish = false`, and declare a binary
   target whose name is exactly `microvm`.
5. Add the SDK as a path dependency of the CLI. Do not add a reverse dependency or a dependency
   on Taumaru distributed services.

### SDK example boundary

The SDK exports `pub fn example_message() -> &'static str`, documented in English. It returns
the stable message `taumaru-microvm SDK is ready` and performs no I/O, logging, environment
lookup, network access, persistence, process control, or global initialization.

### CLI foundation

Use Clap's derive API for a typed top-level parser with explicit name `microvm`, package-derived
version, English description, standard help/version flags, and help guidance when no command is
selected. Keep the initial parser free of production VM subcommands; the structure must leave a
clear location for future subcommands.

Rely on Clap's default terminal-aware styling for baseline help and errors. Do not add a color,
table, progress, JSON, async, or logging dependency until a later feature uses that capability.
Any later dependency must have a current purpose, a documented reason, and tests that exercise
it.

### Test strategy

- The SDK integration test imports the public function as an external consumer and asserts its
  exact deterministic result.
- The CLI integration test locates the `microvm` binary through Cargo's test environment and
  verifies successful help/version/no-argument behavior plus a non-success invalid-input path.
- Tests assert that baseline CLI commands do not require runtime resources and that SDK tests
  observe no unsolicited output or process termination.
- Workspace quality commands cover formatting, compilation, linting with warnings denied, and
  all relevant tests.

## Implementation Sequence

1. Establish the virtual workspace and shared package metadata.
2. Create the SDK package, export the example function, add Rustdoc, and add the external API
   contract test.
3. Create the CLI package, connect the SDK path dependency, add Clap derive configuration, set
   the binary target to `microvm`, and implement baseline parser behavior.
4. Add executable smoke tests for help, version, empty input, and invalid input.
5. Run the quickstart and quality gates, then review package publication boundaries and English
   text compliance.

## Post-Design Constitution Check

The design preserves every pre-design gate. The virtual workspace makes the SDK/CLI boundary
explicit, the pure SDK example has no prohibited side effects, and the CLI owns all presentation
and process-exit behavior. The plan adds no speculative terminal stack, no runtime persistence,
and no Firecracker implementation. All public SDK behavior has a Rustdoc and test location, and
all planned repository text is English.
