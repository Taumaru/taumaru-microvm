# Feature Specification: Bootstrap Two-Crate Workspace

**Feature Branch**: `001-bootstrap-two-crate`

**Created**: 2026-09-17

**Status**: Draft

**Input**: User description: "Set up the basic project foundation with separate SDK and CLI crates. Export a simple SDK example function, add the CLI dependencies needed for a polished foundation including Clap, and expose the CLI executable as `microvm`."

## Clarifications

### Session 2026-09-17

- Q: Which package names should the two crates use? → A: Use `taumaru-microvm` for the SDK and
  `taumaru-microvm-cli` for the CLI; keep the crate directories named `sdk` and `cli`, and keep
  the executable name `microvm`.
- Q: Which dependency policy should the initial CLI foundation follow? → A: Start with Clap only
  and add each additional dependency alongside the first feature that actually uses it.
- Q: What should `microvm` do when invoked without arguments? → A: Display concise help with
  the next available actions, exit successfully with code `0`, and avoid initializing the SDK
  runtime or accessing local or external resources.
- Q: Should the example SDK function be treated as temporary bootstrap API or as a stable public
  contract? → A: Treat it as a documented temporary demonstration API that may be replaced before
  the first stable release.
- Q: Which license should the SDK package declare for its eventual crates.io publication? → A:
  Use MIT for every project component, including the SDK and CLI.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Build the Project Foundation (Priority: P1)

As a project maintainer, I want a workspace with clearly separated SDK and CLI packages so
that the project has a stable foundation for future MicroVM features.

**Why this priority**: Every later capability depends on a reliable package boundary and a
single shared implementation path.

**Independent Test**: Start from a clean checkout, build the project, and verify that both
packages are available independently while the CLI consumes the SDK.

**Acceptance Scenarios**:

1. **Given** a clean checkout, **When** the project is built, **Then** the SDK and CLI packages
   both compile successfully as distinct packages.
2. **Given** the package metadata, **When** a maintainer reviews the package boundaries,
   **Then** the SDK is prepared for crates.io publication and the CLI is configured for binary
   distribution without being published as a crates.io package.
3. **Given** the CLI package, **When** its dependency direction is reviewed, **Then** it
   consumes the SDK and contains no duplicate MicroVM lifecycle implementation.

---

### User Story 2 - Consume the Example SDK API (Priority: P1)

As a Rust developer, I want one small public SDK function that I can call from a consumer
program so that the public library boundary is demonstrated before the real MicroVM features
are added.

**Why this priority**: A working public SDK example proves that the reusable library boundary is
usable independently from the CLI.

**Independent Test**: Create a minimal consumer of the SDK, call the example function, and
verify its deterministic result and library behavior.

**Acceptance Scenarios**:

1. **Given** a consumer that imports the SDK, **When** it calls the exported example function,
   **Then** the function returns a deterministic documented value.
2. **Given** the SDK is embedded in another program, **When** the example function is called,
   **Then** it does not write to stdout or stderr, emit logs, terminate the process, or panic.
3. **Given** the example function is called repeatedly, **When** each call completes, **Then**
   every result remains consistent and independent of CLI state.

---

### User Story 3 - Use the `microvm` CLI Foundation (Priority: P1)

As a Linux user, I want to invoke the command as `microvm` with polished baseline help and
version behavior so that the CLI has a clear entry point for future VM management commands.

**Why this priority**: A stable command name and usable baseline interface are required before
adding operational commands.

**Independent Test**: Install or run the built CLI and exercise its help, version, empty-input,
and invalid-input paths without requiring Firecracker, KVM, a registry connection, or a local
VM.

**Acceptance Scenarios**:

1. **Given** the CLI is available, **When** the user runs `microvm --help`, **Then** the
   command exits successfully and presents usage information under the name `microvm`.
2. **Given** the CLI is available, **When** the user runs `microvm --version`, **Then** the
   command exits successfully and reports the CLI version.
3. **Given** the user runs `microvm` without arguments, **When** no subcommand is selected,
   **Then** the CLI presents concise help and next-action guidance, exits successfully with code
   `0`, and does not initialize the SDK runtime or access local or external resources.
4. **Given** the user enters an unknown command or invalid option, **When** the CLI parses the
   input, **Then** it presents a clear error and a non-success exit status without panicking.
5. **Given** a contributor needs to add a future command, **When** they use the CLI foundation,
   **Then** argument parsing and terminal presentation capabilities are available without
   changing the SDK's lifecycle behavior.

### Edge Cases

- The executable name MUST remain `microvm`; users MUST NOT need to type the repository name or
  an SDK package name to access CLI commands.
- Help and version commands MUST work without initializing a local database, contacting the
  Artifacts Registry, discovering a VM, or starting Firecracker.
- The CLI MUST remain usable when no local VM, kernel, root filesystem, registry cache, or
  runtime directory exists; unavailable runtime resources belong to later operational features.
- The example SDK function MUST remain deterministic when called repeatedly and MUST not depend
  on process state, environment variables, network access, or CLI initialization.
- A dependency that is not used by the baseline CLI MUST NOT be added merely for visual appeal
  or anticipated future functionality.
- Build or dependency resolution failures MUST produce actionable package-manager output and
  MUST NOT be hidden by application-level handling.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The project MUST expose two distinct crates in the `sdk` and `cli` directories:
  the reusable SDK package MUST be named `taumaru-microvm`, and the CLI package MUST be named
  `taumaru-microvm-cli`.
- **FR-002**: The SDK crate MUST contain the shared project core and MUST expose at least one
  documented public example function with a deterministic result. Its Rustdoc MUST identify the
  function as a temporary bootstrap demonstration API that may be replaced before the first
  stable release.
- **FR-003**: The example SDK function MUST return normally without panic, process termination,
  stdout or stderr output, logging, tracing, or hidden global side effects.
- **FR-004**: The CLI crate MUST consume the SDK crate and MUST NOT duplicate MicroVM lifecycle
  behavior.
- **FR-005**: The CLI executable MUST be named exactly `microvm`, so users can access every
  command by beginning with `microvm`.
- **FR-006**: The CLI MUST provide baseline help and version commands and MUST display concise
  help with next-action guidance and exit successfully with code `0` when invoked without a
  subcommand, without initializing the SDK runtime or accessing local or external resources.
- **FR-007**: The CLI MUST return clear, user-facing parse errors and a non-success exit status
  for unknown commands and invalid options without panicking.
- **FR-008**: The CLI foundation MUST include Clap with derive support as its only direct runtime
  dependency in the initial baseline unless planning identifies a documented incompatibility.
  Additional direct dependencies MUST be introduced alongside a baseline capability that uses
  them, such as styled terminal output, tables, progress reporting, or machine-readable output,
  and each dependency MUST have a documented purpose.
- **FR-009**: The SDK crate MUST include package metadata suitable for eventual crates.io
  publication and declare the MIT license, while the CLI crate MUST use the MIT license for its
  project metadata, remain configured for binary distribution, and be excluded from crates.io
  publication.
- **FR-010**: The initial setup MUST include tests for package boundaries, the example SDK
  function, CLI help and version behavior, invalid CLI input, and the SDK's absence of panic or
  unsolicited output.
- **FR-011**: All source code, identifiers, comments, documentation, tests, examples,
  configuration, generated files, CLI text, and error messages MUST be written in English,
  regardless of the language used in the request.
- **FR-012**: This feature MUST establish the project foundation only. Firecracker lifecycle
  operations, `firectl` execution, registry access, local database schema, VM persistence, and
  production CLI subcommands are out of scope for this feature.

### Key Entities *(include if feature involves data)*

- **SDK Crate**: The reusable public library package containing the shared project core and the
  example exported function.
- **CLI Crate**: The binary-oriented package that consumes the SDK and owns command parsing,
  presentation, and process exit behavior.
- **`microvm` Executable**: The user-facing command name for the CLI package.
- **Example SDK Function**: A minimal deterministic public API used to verify SDK consumption;
  it is a temporary bootstrap demonstration rather than a stable compatibility commitment.
- **CLI Dependency Set**: The direct dependencies required for argument parsing and the baseline
  terminal experience, each justified by a current capability.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: From a clean checkout with dependencies available, 100% of baseline build and
  test checks pass for both packages without manual source changes.
- **SC-002**: In 100% of baseline CLI smoke tests, `microvm --help` and `microvm --version`
  complete successfully and identify the command as `microvm`.
- **SC-003**: A minimal consumer can call the example SDK function and receive its documented
  result in 100% of repeated test runs without process output, panic, or termination.
- **SC-004**: The CLI dependency review finds zero unused direct dependencies and a documented
  purpose for every dependency included in the initial foundation.
- **SC-005**: A contributor can add the first real CLI subcommand through the established CLI
  boundary without creating a second SDK or MicroVM lifecycle implementation.
- **SC-006**: Reviewers judge the initial CLI interaction as clear and consistent with the
  project's calm, semantic, English-only design rules for help, version, empty-input, and error
  paths.

## Assumptions

- The project continues to use Rust and a Cargo workspace because the requested deliverables
  are Rust crates and the existing repository is already a Rust package.
- Clap is the default command parser for the CLI and is the only direct runtime dependency in the
  initial baseline. Supporting dependencies for color handling, tables, progress indicators, or
  structured output will be added only when an implemented capability requires them.
- The SDK example function is intentionally trivial and is a boundary demonstration, not a
  substitute for MicroVM creation or lifecycle behavior. It is temporary and may be replaced
  before the first stable release.
- The first CLI baseline does not require Firecracker, KVM, a registry connection, a local
  database, or existing VM state.
- The SDK is configured for future publication, but actual publication to crates.io and binary
  release distribution are separate release features. MIT is the license for every project
  component.
- The initial implementation targets Linux while keeping the public SDK boundary independent of
  CLI terminal behavior and host-specific process details.
- The SDK package is named `taumaru-microvm`, the CLI package is named `taumaru-microvm-cli`,
  their crate directories are `sdk` and `cli`, and the `microvm` executable name is fixed for
  this feature.
