# CLI Snapshot Contract

## Interaction

- Preserve the existing snapshot command forms and output-folder/password prompts.
- In an interactive terminal, ask whether source IPv4 settings should be included. Explain before the choice that preserving them may conflict with addresses already in use when restored.
- A yes choice passes the preserve policy to the SDK.
- A no choice explains that only LAN exposure is saved, the archive copy's Taumaru-managed network file is sanitized, and restore allocates destination-local IPs.
- In non-interactive use, require an explicit policy and fail before archive creation if it is absent.
- The SDK owns snapshotting and archive creation; the CLI owns prompting, progress, and diagnostics.
- Progress distinguishes preparing the point-in-time view, sanitizing the private copy when selected, and streaming/completing the archive. Show real byte progress during disk payload work and do not show success until publication and cleanup complete.

## Result and errors

On success, report the archive path and selected address policy. On failure, explain what happened, why it matters, and the next action using the standard calm CLI format. Never alter or delete the source VM disk.
