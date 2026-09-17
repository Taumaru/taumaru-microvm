# Data Model: Bootstrap Two-Crate Workspace

This feature establishes package and command boundaries. It does not introduce persisted
MicroVM domain state, registry records, or Firecracker runtime entities.

## SDK Package

| Field | Type or value | Rules |
|-------|---------------|-------|
| Package name | `taumaru-microvm` | Stable public SDK identity for this feature. |
| Target kind | Library | Exposes the reusable SDK API. |
| Publication | Intended for crates.io | The package is prepared for future publication. |
| License | MIT | Applies to this project component. |
| Public module | `src/lib.rs` | Contains the example API and Rustdoc. |
| Runtime dependencies | None | Must remain silent and side-effect free. |

## CLI Package

| Field | Type or value | Rules |
|-------|---------------|-------|
| Package name | `taumaru-microvm-cli` | Binary-oriented package identity. |
| Target kind | Binary | Provides the user-facing command. |
| Publication | `publish = false` | Distributed as an installable binary, not a crates.io package. |
| License | MIT | Applies to this project component. |
| Binary name | `microvm` | Exact executable name required by the feature. |
| SDK relationship | Path dependency on `taumaru-microvm` | Must not duplicate lifecycle behavior. |
| Runtime dependency | Clap 4.6 with `derive` | Owns typed argument parsing and baseline help/version behavior. |

## Example SDK Function

| Field | Value | Rules |
|-------|-------|-------|
| Name | `example_message` | Public and documented in English. |
| Signature | `pub fn example_message() -> &'static str` | No input or mutable global state. |
| Result | `taumaru-microvm SDK is ready` | Deterministic across repeated calls. |
| Lifecycle | Temporary bootstrap API | May be replaced before the first stable release. |
| Side effects | None | No output, logging, tracing, process control, or environment access. |

## CLI Command Surface

| Surface | Input | Expected result |
|---------|-------|-----------------|
| Help | `microvm --help` | Success, usage text, command name `microvm`. |
| Version | `microvm --version` | Success, CLI version text. |
| Empty input | `microvm` | Concise help guidance, exit status `0`, no panic or runtime initialization. |
| Invalid input | Unknown command or option | Clear parse error and non-success exit status. |

## Relationships

```text
SDK Package (taumaru-microvm)
        ▲
        │ path dependency
        │
CLI Package (taumaru-microvm-cli)
        │
        ▼
Executable (microvm)
```

There are no lifecycle state transitions in this feature. Future VM states belong to the
host-local runtime model and must be introduced by a separate feature without changing the
package boundary described here.
