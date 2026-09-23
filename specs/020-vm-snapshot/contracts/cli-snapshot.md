# CLI Snapshot Contract

## Command forms

    microvm snapshot
    microvm snapshot <NAME>
    microvm snapshot <NAME> <OUTPUT_PATH>

The command also accepts:

    --password <PASSWORD>

With no output path, use ./<NAME>.tmvmsnap. With no VM name, interactive mode lists available VMs and lets the operator select one. If the output path exists at publication time, fail with an actionable message and preserve that file.

## Password input

A supplied password is passed to the SDK and never included in success or error output. Without --password, prompt for it without echo only when both stdin and stderr are interactive. Without an interactive terminal, return an actionable error and do not wait for input.

The explicit password option is retained for scripts as required. Operating systems may expose command-line arguments in shell history or process metadata; the hidden prompt is the safer interactive option. If privilege escalation is needed, perform the hidden prompt in the elevated child so the secret is not copied into the elevation command. A supplied --password argument necessarily travels through the existing command-line/elevation path.

Do not echo either password entry or confirmation. Do not place a password in progress events, debug formatting, or error values.

## Privilege, cancellation, and output

The command uses the SDK for all snapshot and lifecycle behavior. It uses existing CLI home, terminal, VM selection, and privilege conventions. Online Device Mapper setup requires the privilege level already used for VM lifecycle operations.

For Ctrl+C, allow the elevated child to observe cancellation and finish SDK cleanup before any forced termination fallback. Report that the operation was cancelled only after cleanup completes. On success, print the selected VM and final path and may report encrypted byte size. Never report the password.

Examples:

- Interactive operator: run microvm snapshot and choose a listed VM, then enter a hidden password.
- Script: supply a VM name, output path if desired, and --password.
- Non-interactive use without a VM name or password fails immediately with a useful message rather than prompting.
