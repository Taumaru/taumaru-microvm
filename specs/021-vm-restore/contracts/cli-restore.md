# CLI Restore Contract

## Forms

- Interactive: `microvm restore`
- Explicit archive with interactive password: `microvm restore <ARCHIVE_PATH>`
- Non-interactive: `microvm restore <ARCHIVE_PATH> --password <PASSWORD>`

## Interaction

- If archive path is missing in an interactive terminal, prompt for it.
- If password is missing in an interactive terminal, request it once without echo or confirmation.
- If either value is missing in a non-interactive terminal, fail immediately with an actionable diagnostic.
- Never print the supplied password.
- Display progress using the CLI renderer. Progress follows recovered payload bytes and operation stages; success appears only after archive authentication, verification, network setup, and local commit complete.

## Result and errors

On success, report archived VM name, destination-local identity, and stopped state. On failure, explain what happened, why it matters, and the next action using the standard calm CLI format. The command delegates restore, persistence, filesystem, and network behavior to the SDK.
