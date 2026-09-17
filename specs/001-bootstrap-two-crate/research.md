# Research: Bootstrap Two-Crate Workspace

**Date**: 2026-09-17

## Decision 1: Use a virtual Cargo workspace

**Decision**: Convert the repository root manifest into a virtual workspace with two members:
`crates/sdk` and `crates/cli`. Set the workspace dependency resolver explicitly to `"3"` and
centralize shared package metadata where it reduces duplication.

**Rationale**: The project has two independently meaningful packages and no root application
that needs to be published or run. A virtual workspace keeps the root focused on coordination,
gives both packages one lockfile and target directory, and lets maintainers run workspace-wide
checks. Cargo documents virtual workspaces as the appropriate structure when there is no primary
root package and requires an explicit resolver in that form.

**Alternatives considered**:

- Keep the current root package and add a library target plus a separate CLI package. Rejected
  because it couples the current root application to the CLI and makes the SDK boundary less
  explicit.
- Keep one package with both library and binary targets. Rejected because the constitution
  requires distinct SDK and CLI crates.

**Source**: [Cargo Workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)

## Decision 2: Separate package identity from executable identity

**Decision**: Keep `taumaru-microvm` as the SDK package name and use `taumaru-microvm-cli` for
the CLI package. Configure the CLI's executable target name as exactly `microvm`. The SDK is
intended for crates.io publication; the CLI is configured with `publish = false` and remains a
binary distribution artifact.

**Rationale**: The existing package name already represents the reusable project. Cargo package
names and binary target names are separate concepts, so the CLI can have an explicit package
identity while presenting the short user-facing command required by the feature.

**Alternatives considered**:

- Use `taumaru-microvm` as the executable name. Rejected because users explicitly require
  invoking the tool as `microvm`.
- Publish both packages. Rejected because the CLI is intentionally distributed as a binary,
  not as a crates.io library package.

## Decision 3: Use Clap's derive API for the CLI foundation

**Decision**: Add Clap in the CLI crate with the `derive` feature and use typed parser and
command metadata definitions. Set the command name explicitly to `microvm`, enable version
metadata from the package, render concise help and exit successfully when no arguments are
provided, and leave invalid input to Clap's non-success parse errors.

**Rationale**: Clap's derive API generates a typed parser from Rust declarations, and its
documented command attributes support explicit names and package-derived versions. A small
explicit no-argument branch supplies the required successful help behavior, while Clap's default
features provide usage, contextual errors, suggestions, and terminal styling without a second
presentation framework.

**Alternatives considered**:

- Hand-written argument parsing. Rejected because it would create unnecessary parsing code and
  weaker help/error behavior.
- A different parser such as `argh` or `bpaf`. Rejected because the user explicitly requested
  Clap as the default direction and no current requirement justifies changing it.

**Sources**: [Clap derive reference](https://docs.rs/clap/latest/clap/_derive/), [Clap feature
flags](https://docs.rs/clap/latest/clap/_features/), and [Clap tutorial](https://docs.rs/clap/latest/clap/_tutorial/)

## Decision 4: Keep the initial dependency set minimal

**Decision**: The SDK starts with no runtime dependencies. The CLI starts with Clap as its only
required direct runtime dependency. Do not add `owo-colors`, `colored`, `indicatif`,
`comfy-table`, `serde_json`, `tokio`, or logging crates in this bootstrap unless a baseline
acceptance scenario actually uses the capability.

**Rationale**: The initial scope contains only an example SDK function and CLI help/version
behavior. Clap already supports styled help and error output, while unused direct dependencies
would increase compile time, maintenance surface, and future compatibility obligations. This
also protects the SDK's no-logging and no-output contract.

**Alternatives considered**:

- Preinstall a full terminal UI stack for anticipated VM commands. Rejected because it violates
  the repository's no-unused-dependencies rule and would make the bootstrap harder to review.
- Add a general-purpose error or logging framework to the SDK. Rejected because the SDK must
  return typed errors and remain silent; those policies belong to future layers and the CLI.

## Decision 5: Export a pure, deterministic SDK example function

**Decision**: Expose `pub fn example_message() -> &'static str` from the SDK as a temporary,
documented bootstrap demonstration API. It returns the constant message `taumaru-microvm SDK is
ready` and performs no I/O, logging, process control, environment lookup, network access, or
persistence.

**Rationale**: A pure function proves that a consumer can import and call the SDK independently
from the CLI while making the no-panic and no-side-effect requirements straightforward to test.
The function is intentionally marked as a bootstrap example so it does not become a substitute
for future MicroVM domain APIs.

**Alternatives considered**:

- Expose a `hello` function with no project context. Rejected because the example message should
  make the package boundary recognizable.
- Start with a MicroVM manager or configuration API. Rejected because that would pull runtime,
  persistence, and Firecracker decisions into a foundation feature whose scope excludes them.

## Decision 6: Test the package boundaries without adding test-only frameworks

**Decision**: Test the SDK example with package unit tests and test CLI parsing through Clap's
fallible parser from Rust tests. Validate the actual executable name and help/version behavior
through the quickstart commands. Do not add `assert_cmd` or similar test frameworks in the
bootstrap.

**Rationale**: The baseline has a small surface. Standard Rust tests can verify deterministic
SDK behavior and parser outcomes without increasing the dependency set. Command-level smoke
checks remain documented and runnable, while a future command-heavy feature can introduce a
dedicated CLI test harness if its value justifies it.

**Alternatives considered**:

- Add process assertion libraries immediately. Rejected because the initial parser surface is
  small and the added test dependencies would not be needed by the baseline unit tests.
- Test only compilation. Rejected because compilation does not prove the public SDK contract or
  the required `microvm` command behavior.

## Decision 7: Use conservative shared package metadata

**Decision**: Keep the existing `0.1.0` project version and Rust 2024 edition. Add English
package descriptions, repository metadata where known, and MIT license metadata for every
project component. Treat license-file completion and actual publication as release work outside
this feature.

**Rationale**: Shared metadata makes both packages consistent and gives the SDK a credible
publication starting point without coupling the CLI to crates.io. The MIT license is an explicit
project decision confirmed during clarification.

**Alternatives considered**:

- Invent a repository URL, author list, or publication date. Rejected because those values are
  not present in the repository context.
- Block the foundation on release metadata. Rejected because the requested feature is the
  package boundary and executable bootstrap, not a release process.
