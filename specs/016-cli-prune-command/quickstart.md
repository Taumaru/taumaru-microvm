# Quickstart: Prune Unused Artifacts From the CLI

This feature is the thin CLI surface over the existing SDK prune operation.
It adds `microvm artifacts prune` beside `microvm artifacts download`;
acquisition, creation, and lifecycle behavior are separate operations.

## Host prerequisites

The host must be Linux with an explicit SDK home containing
`state/inventory.db` and downloaded artifacts. The CLI resolves the home
with the existing policy (`TAUMARU_HOME`, else `~/.taumaru-microvm`) and
passes it explicitly to the SDK. Pruning requires root access through the
existing privilege flow; interactive use requires a terminal.

## Prepare a mixed inventory

Use the existing flows first. Prune does not acquire, verify, or repair
anything; it reclaims what the SDK inventory records as unreferenced:

```bash
microvm artifacts download
microvm new
```

Machines may be running, stopped, or never started: existence alone
protects their artifacts.

## Prune unreferenced kernels and images

```bash
microvm artifacts prune
```

The flow:

1. Without root, the command requests elevation through the existing
   privilege flow before doing anything else.
2. The command shows a preview listing every kernel and image identity to
   delete plus the estimated freed space, and asks for confirmation.
3. On confirmation it deletes, then reports the removed kernels and
   images with counts per kind, the freed space per kind and in total,
   and any skipped active transfers as a separate group.

Pruning when everything is referenced (or nothing is downloaded) reports
nothing to prune with success status and shows no confirmation. Scripted
use deletes without prompting after the root gate:

```bash
microvm artifacts prune --non-interactive
```

Declining the preview or interrupting the confirmation cancels with no
deletions (exit `130`).

## Expected failure handling

A partial failure still shows the removed set with freed space so far,
names each failed artifact with its cause, and exits nonzero; fix the
cause and rerun to reclaim the rest. Missing rights in non-interactive
mode, an unavailable home, and other SDK failures each explain what
happened, why nothing (or only part) was pruned, and what to do next.
Every failure is a calm triplet with no panic and no stack trace.

## Verification commands

Run the repository gates after implementation:

```bash
cargo fmt --all -- --check
cargo check --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

The automated suite covers the `artifacts prune` parse surface
(flag-only, no positionals or selection flags), the escalated child argv
(`artifacts prune --non-interactive`), the preview/result/empty report
rendering with text-first groups, the partial-failure mapping that keeps
the removed set, the cancellation prefix exit, and the non-interactive
privilege error through the existing surface tests. Real deletions stay
out of the default suite (they require rights and a seeded inventory) and
are covered by manual quickstart runs.
