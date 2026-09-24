# CLI Contract: Snapshot and Restore

## Snapshot command

The command remains microvm snapshot [NAME] [OUTPUT_PATH] with the existing address-policy option. Interactive VM selection, output-folder selection, and IPv4 policy prompts remain available. Non-interactive callers provide a VM name and address policy; the output path keeps its existing default.

The command has no --password option and never prompts for an archive password. Its elevated child invocation carries only non-secret snapshot arguments. Completion reports the output path and archive size and discloses that the archive is unencrypted and contains the VM disk and SSH credentials.

## Restore command

The command remains microvm restore [ARCHIVE_PATH] with the existing --non-interactive option. Interactive use may still prompt for a missing archive path. Non-interactive use requires the archive path and never waits for password input.

The command has no --password option and never prompts for an archive password. Supplying the removed option is a parse error; its value must not be echoed. A password-encrypted legacy archive produces a clear compatibility error and does not change destination state.

## Presentation

Snapshot and restore progress describes preparing, writing, reading, verifying, and installing the archive without claiming encryption or decryption. SDK operations remain silent; the CLI owns progress, errors, and the plaintext disclosure.
