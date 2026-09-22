# Phase 1 Data Model: CLI `artifacts prune` Command

## Input

The command takes no positional arguments and no selection flags. The only
input is the established flag:

| Field | Type | Rules |
|---|---|---|
| `non_interactive` | `bool` | `--non-interactive`: no prompts of any kind, root required up front, deterministic output. Absent means interactive mode (elevation prompt plus preview confirmation when needed). |

No home-directory flag exists: home resolution follows the existing CLI
policy (`TAUMARU_HOME`, else `~/.taumaru-microvm`) and the resolved path is
passed explicitly to the SDK. Unknown flags and positionals are rejected by
the parser.

## Records read

The CLI opens no database and reads no files directly. Everything comes from
the existing SDK surface:

### `PruneSummary` (existing SDK type, read-only for the CLI)

Returned by the single `prune_unused_artifacts()` call and rendered verbatim:

| Field | Role in the CLI |
|---|---|
| `removed_kernels` | Rendered kernel identities, ascending; length is the kernel removal count |
| `removed_images` | Rendered `(distribution_id, image_id)` pairs, distribution-then-image order; length is the image removal count |
| `skipped_artifact_keys` | Rendered as the separate skipped group, ascending |
| `freed_bytes_kernels` | Rendered per-kind freed space through the shared byte helper |
| `freed_bytes_images` | Rendered per-kind freed space through the shared byte helper |
| `freed_bytes_total` | Rendered total freed space; the CLI never recomputes it |

### Preview candidates (existing SDK queries, read-only)

Before the interactive confirmation, the CLI derives the estimated reclaimed
set from already-public SDK queries: downloaded kernel/image identities minus
the identities referenced by `list_microvms()` rows, using the same
existence-based reference rule the SDK documents. The preview lists every
candidate identity plus the estimated freed space. The preview is an
estimate: the authoritative outcome is always the `PruneSummary` from the
deleting call, which may differ if inventory changed between preview and
deletion.

### `PruneFailure` (existing SDK type, read-only for the CLI)

One entry per failed candidate from `SdkError::PruneIncomplete`:

| Field | Role in the CLI |
|---|---|
| `artifact_key` | Rendered failed identity |
| `reason` | Rendered cause beside the identity |

## Records written

The CLI writes no inventory rows and creates no files. The only mutation is
the SDK prune call itself, which owns all file and row deletes. CLI-side
outputs:

### Prune report (new presentation, no persisted state)

Rendered after the SDK call returns:

| Group | Content |
|---|---|
| `removed` | Kernel identities, image identities, counts per kind, freed space per kind and in total |
| `skipped` | Artifact keys kept intact because of an active transfer, shown separately |
| `failed` | Artifact keys with causes, shown only on partial failure with the removed set so far |
| `nothing to prune` | Calm report when every removed list is empty, with success status |

## State machine

```text
non-interactive + unprivileged ──► hard error with rerun guidance (no prompt, no deletion)

interactive + unprivileged ──► elevation gate ──► declined/interrupted ──► cancelled, exit 130
        │                                                  │
        │ approved                                         ▼
        ▼                                           escalated child reruns
privileged ──► derive preview ──► nothing unreferenced ──► "nothing to prune", exit 0
        │                                                  │
        │ candidates exist                                 ▼ (interactive only)
        ├── non-interactive ──► delete, no prompts ────────┤
        │                                                  ▼
        └── interactive ──► show preview ──► declined/interrupted ──► cancelled, exit 130
                                │ confirmed
                                ▼
                        single prune_unused_artifacts() call
                                │
                                ├── Ok(summary) ──► removed + skipped report, exit 0
                                └── PruneIncomplete ──► removed so far + skipped + failed
                                                        with causes and retry step, exit 1
```

No confirmation is shown when the preview is empty. No selector, no
re-prompt loop, and no second SDK call exist on any path.

## Ownership and multi-command rules

- The `artifacts` group keeps one owner per subcommand: `download.rs` owns
  `download`, the new `prune.rs` owns `prune`. No shared mutable command
  state exists between them.
- The privilege flow (`privilege.rs`) is reused unchanged; the prune child
  argv is `["artifacts", "prune", "--non-interactive"]` and never
  re-escalates.
- Byte formatting reuses the existing binary-unit helper; no independent
  size measurement exists in the CLI.
- Other commands (`new`, `start`, `stop`, `ssh`, `ls`, `artifacts
  download`) are untouched; the only new observable capability is the
  `artifacts prune` entry.
