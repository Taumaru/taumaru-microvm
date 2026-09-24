# Implementation Plan: Unencrypted MicroVM Snapshots and Restore

**Branch**: 022-remove-snapshot-encryption | **Date**: 2026-09-24 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from specs/022-remove-snapshot-encryption/spec.md

## Summary

Replace the password-encrypted age wrapper with a plain Zstandard-compressed TAR archive using manifest format version 3. Remove password inputs from SDK operations, public request/result types, and CLI commands. Keep payload SHA-256 validation, add the Zstandard frame checksum, and reject old age-encrypted archives before staging. Preserve snapshot consistency, restore atomicity, address policy, privilege flow, and filesystem access defaults selected in clarification. This is a source-breaking SDK change; plan a workspace version change from 0.1.0 to 0.2.0 with migration guidance.

## Technical Context

**Language/Version**: Rust 2024 edition; current workspace release is 0.1.0.

**Primary Dependencies**: Zstandard 0.13, TAR 0.4, SHA-256 0.11, Clap 4.6, and Inquire 0.9. Remove age 0.11 from the SDK and workspace after the legacy-format fixture and archive tests no longer require it.

**Storage**: Portable .tmvmsnap files containing a Zstandard-compressed TAR stream and a versioned manifest. No local database schema change.

**Testing**: SDK archive unit tests, snapshot and restore integration tests, public API coverage, and CLI command-surface tests. Run the Rust workspace format, check, clippy, and test gates before implementation completion.

**Target Platform**: Linux hosts that support the existing Firecracker and KVM snapshot/restore workflows.

**Project Type**: Publishable Rust SDK library plus a thin Rust CLI.

**Performance Goals**: Preserve bounded 128 KiB streaming and caller-owned progress/cancellation behavior. Keep the current Zstandard level 3. Do not introduce another full-payload copy or a user-visible timing target.

**Constraints**: The archive has no password or encryption; payload and frame checksums do not provide authenticity. Support only the new plain archive format version 3. Reject age-encrypted legacy archives with an actionable error. Remove all password prompts, fields, arguments, help text, and privilege-forwarding. Apply normal output-file creation permissions through the effective process umask and destination directory policy. Preserve no-overwrite publication and cleanup. Treat SDK signature and result-field changes as breaking; use version 0.2.0 and migration notes.

**Scale/Scope**: Retain the existing restore limits: 3 TiB maximum archive, 2 TiB root disk, 512 MiB kernel, 1 MiB SSH-key members, and 1 MiB manifest. No new inventory entities, database migration, encryption option, or archive-size policy.

## Constitution Check

**Pre-Phase 0 Gate: PASS**

| Principle / Gate | Plan response |
|---|---|
| SDK-first shared core | Archive creation and restore remain in SDK adapters/managers; CLI remains a presentation layer. |
| Panic-free and silent SDK | Preserve typed archive errors, caller-owned progress, cancellation, and no unsolicited SDK output. |
| Explicit local state and lifecycle | Keep snapshot source state unchanged, restore stopped, and retain existing restore journal/cleanup behavior; no persistence migration is needed. |
| Closed for modification, open for extension | Reuse the existing archive boundary and replace its encoder/reader behavior; do not create a second lifecycle path. |
| Calm, accessible CLI and English artifacts | Remove password flows, use clear plaintext disclosure, and preserve path/address prompts and script behavior. |
| Breaking public API migration | Remove password inputs and rename the size field in a planned 0.2.0 pre-1.0 release with migration notes and legacy archive guidance. |

**Post-Phase 1 Gate: PASS** — The design changes only the archive adapter, public snapshot/restore contracts, and CLI presentation. It introduces no new infrastructure dependency, global side effect, database change, or lifecycle engine.

## Project Structure

### Documentation

    specs/022-remove-snapshot-encryption/
    ├── plan.md
    ├── research.md
    ├── data-model.md
    ├── quickstart.md
    └── contracts/
        ├── archive.md
        ├── cli.md
        └── sdk.md

### Source Code

    Cargo.toml
    crates/
    ├── sdk/
    │   ├── Cargo.toml
    │   ├── src/
    │   │   ├── lib.rs
    │   │   ├── adapters/archive/
    │   │   │   ├── mod.rs
    │   │   │   ├── manifest.rs
    │   │   │   ├── restore.rs
    │   │   │   └── tar_zstd.rs
    │   │   ├── domain/
    │   │   │   ├── restore.rs
    │   │   │   └── snapshot.rs
    │   │   ├── manager.rs
    │   │   ├── manager/restore.rs
    │   │   └── error.rs
    │   └── tests/
    │       ├── public_api.rs
    │       ├── snapshot.rs
    │       └── restore.rs
    └── cli/
        ├── src/
        │   ├── cli.rs
        │   ├── commands/snapshot.rs
        │   ├── commands/restore.rs
        │   └── output/human.rs
        └── tests/command_surface.rs

**Structure Decision**: Keep the existing two-crate boundary and archive adapter. Rename age_tar_zstd.rs to tar_zstd.rs to match its remaining format responsibilities. Update existing SDK and CLI modules and tests in place. Reuse the existing RestoreArchive typed error. No new source module, database migration, or independent lifecycle implementation is needed.

## Complexity Tracking

No constitution violations or additional architectural layers are introduced.
