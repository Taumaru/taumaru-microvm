# CLI Contract: Bootstrap Command Surface

## Executable

- Executable name: `microvm`
- Package: `taumaru-microvm-cli`
- Publication: binary distribution; not crates.io publication
- Output language: English

## Baseline Commands

| Invocation | Exit status | Output contract |
|------------|-------------|-----------------|
| `microvm --help` | `0` | Displays usage and identifies the executable as `microvm`. |
| `microvm --version` | `0` | Displays the CLI version. |
| `microvm` | `0` | Displays concise help guidance without initializing runtime resources. |
| `microvm unknown` | Non-zero | Displays a clear parse error and usage guidance. |
| `microvm --unknown` | Non-zero | Displays a clear invalid-option error and usage guidance. |

## Parser Rules

- The parser uses Clap's derive API with an explicit command name of `microvm`.
- Help and version are available without a local database, registry connection, kernel,
  root filesystem, KVM, Firecracker process, or existing MicroVM.
- Terminal styling is automatic and terminal-aware. Piped output remains usable as plain text.
- The baseline has no production VM subcommands. Future subcommands must call the SDK rather than
  implement lifecycle behavior in the CLI.
- Parse failures are owned by the CLI and do not become SDK output or logging side effects.

## Compatibility Notes

The exact help wording and package version format may evolve, but the executable name, English
output, successful help/version behavior, and non-panicking error behavior are part of the
baseline contract.
