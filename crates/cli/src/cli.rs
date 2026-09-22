use clap::{Args, Parser, Subcommand};

/// Command-line interface for host-local MicroVM operations.
#[derive(Debug, Parser)]
#[command(
    name = "microvm",
    version,
    about = "Build and manage Firecracker MicroVMs on Linux."
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Manage local artifact inventory.
    Artifacts(ArtifactsArgs),
    /// Create a new MicroVM through guided prompts or explicit flags.
    New(NewArgs),
    /// Start a created MicroVM by name or interactive selection.
    Start(StartArgs),
    /// Stop a running MicroVM by name or interactive selection.
    Stop(StopArgs),
    /// Delete a MicroVM by name or interactive selection.
    Delete(DeleteArgs),
    /// List all MicroVMs with state and configured capacities.
    #[command(visible_alias = "list")]
    Ls(LsArgs),
    /// Connect to a running MicroVM over SSH by name or interactive selection.
    Ssh(SshArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct ArtifactsArgs {
    #[command(subcommand)]
    pub(crate) command: ArtifactsCommand,
}

#[derive(Debug, Subcommand, Clone)]
pub(crate) enum ArtifactsCommand {
    /// Select and download runtime, kernel, and distribution image artifacts.
    Download(DownloadArgs),
    /// Delete downloaded kernels and images no existing MicroVM references.
    Prune(PruneArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct DownloadArgs {
    /// Select an image by registry IDs as DISTRIBUTION_ID=IMAGE_ID. Repeat for multiple images.
    #[arg(long = "image", value_name = "DISTRIBUTION_ID=IMAGE_ID")]
    pub(crate) images: Vec<String>,

    /// Disable prompts and require complete explicit selections.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct PruneArgs {
    /// Disable prompts; root is required up front.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct NewArgs {
    /// Machine name; skips the name prompt when supplied.
    pub(crate) name: Option<String>,

    /// Equivalent to the positional name; must agree when both are given.
    #[arg(long = "name")]
    pub(crate) explicit_name: Option<String>,

    /// Select the single published image as DISTRIBUTION_ID=IMAGE_ID.
    #[arg(long = "image", value_name = "DISTRIBUTION_ID=IMAGE_ID")]
    pub(crate) image: Option<String>,

    /// Requested root-disk size in gigabytes (decimal values allowed).
    #[arg(long = "disk-gb")]
    pub(crate) disk_gb: Option<String>,

    /// Requested memory as xMB or xGB (for example 512MB or 1.5GB).
    #[arg(long = "memory")]
    pub(crate) memory: Option<String>,

    /// Requested virtual CPU count.
    #[arg(long = "vcpus")]
    pub(crate) vcpus: Option<String>,

    /// Expose the VM on the local network; absent means host-only.
    #[arg(long = "expose-lan")]
    pub(crate) expose_lan: bool,

    /// Disable prompts and require the complete explicit set.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,

    /// Trust already-validated values and skip registry revalidation.
    /// Internal only: set by privilege escalation for the elevated child.
    #[arg(long = "trusted-values", hide = true)]
    pub(crate) trusted_values: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct StartArgs {
    /// Machine name; skips the machine selector when supplied.
    pub(crate) name: Option<String>,

    /// Equivalent to the positional name; must agree when both are given.
    #[arg(long = "name")]
    pub(crate) explicit_name: Option<String>,

    /// Disable prompts; the name is required and root is required up front.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,
}
#[derive(Debug, Args, Clone)]
pub(crate) struct StopArgs {
    /// Machine name; skips the machine selector when supplied.
    pub(crate) name: Option<String>,

    /// Equivalent to the positional name; must agree when both are given.
    #[arg(long = "name")]
    pub(crate) explicit_name: Option<String>,

    /// Disable prompts; the name is required and root is required up front.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct DeleteArgs {
    /// Machine name; skips the machine selector when supplied.
    pub(crate) name: Option<String>,

    /// Equivalent to the positional name; must agree when both are given.
    #[arg(long = "name")]
    pub(crate) explicit_name: Option<String>,

    /// Disable prompts; the name is required and root is required up front.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct LsArgs {
    /// Disable prompts; root is required up front.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,
}

#[derive(Debug, Args, Clone)]
pub(crate) struct SshArgs {
    /// Machine name; skips the machine selector when supplied.
    /// With a trailing remote command present, all words after the name are remote.
    /// To run a remote command on the interactively selected machine, omit the
    /// name and put the command after `--`.
    pub(crate) name: Option<String>,

    /// Equivalent to the positional name; must agree when both are given.
    #[arg(long = "name")]
    pub(crate) explicit_name: Option<String>,

    /// Disable prompts; the name is required and rights are required up front.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,

    /// Remote command to run inside the guest instead of a shell.
    /// Everything after `--` is always remote command; without `--`, words
    /// after the machine name are remote command.
    #[arg(last = true)]
    pub(crate) command: Vec<String>,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{ArtifactsCommand, Cli, Command};

    #[test]
    fn download_parser_accepts_repeatable_explicit_options() {
        let cli = Cli::try_parse_from([
            "microvm",
            "artifacts",
            "download",
            "--non-interactive",
            "--image",
            "distro-a=image-a",
            "--image",
            "distro-a=image-a-debug",
            "--image",
            "distro-z=image-z",
        ])
        .expect("valid explicit download arguments");

        let Some(Command::Artifacts(arguments)) = cli.command else {
            panic!("download command should be parsed");
        };
        let ArtifactsCommand::Download(download) = arguments.command else {
            panic!("download command should be parsed");
        };
        assert!(download.non_interactive);
        assert_eq!(
            download.images,
            [
                "distro-a=image-a",
                "distro-a=image-a-debug",
                "distro-z=image-z"
            ]
        );
    }

    #[test]
    fn removed_distribution_and_kernel_flags_are_rejected() {
        assert!(Cli::try_parse_from(["microvm", "download"]).is_err());
        assert!(
            Cli::try_parse_from([
                "microvm",
                "artifacts",
                "download",
                "--distribution",
                "distro-a",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from(["microvm", "artifacts", "download", "--kernel", "x=y"]).is_err()
        );
    }

    #[test]
    fn new_parser_accepts_positional_name_and_single_image_set() {
        let cli = Cli::try_parse_from([
            "microvm",
            "new",
            "web-01",
            "--non-interactive",
            "--image",
            "distro-a=image-a",
            "--disk-gb",
            "20",
            "--memory",
            "2GB",
            "--vcpus",
            "2",
        ])
        .expect("valid explicit new arguments");

        let Some(Command::New(arguments)) = cli.command else {
            panic!("new command should be parsed");
        };
        assert_eq!(arguments.name.as_deref(), Some("web-01"));
        assert_eq!(arguments.image.as_deref(), Some("distro-a=image-a"));
        assert!(arguments.non_interactive);
        assert!(!arguments.expose_lan);
    }

    #[test]
    fn new_parser_rejects_out_of_scope_lifecycle_flags() {
        assert!(Cli::try_parse_from(["microvm", "new", "--kernel", "x"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "new", "--volume-path", "/tmp/x"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "new", "--lan-address", "1.2.3.4"]).is_err());
    }

    #[test]
    fn start_parser_accepts_positional_name_and_flag() {
        let cli = Cli::try_parse_from(["microvm", "start", "web-01", "--non-interactive"])
            .expect("valid explicit start arguments");

        let Some(Command::Start(arguments)) = cli.command else {
            panic!("start command should be parsed");
        };
        assert_eq!(arguments.name.as_deref(), Some("web-01"));
        assert!(arguments.non_interactive);
    }

    #[test]
    fn stop_parser_accepts_positional_name_and_flag() {
        let cli = Cli::try_parse_from(["microvm", "stop", "web-01", "--non-interactive"])
            .expect("valid explicit stop arguments");

        let Some(Command::Stop(arguments)) = cli.command else {
            panic!("stop command should be parsed");
        };
        assert_eq!(arguments.name.as_deref(), Some("web-01"));
        assert!(arguments.non_interactive);
    }

    #[test]
    fn stop_parser_rejects_lifecycle_flags() {
        assert!(Cli::try_parse_from(["microvm", "stop", "--image", "x"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "stop", "--disk-gb", "20"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "stop", "--expose-lan"]).is_err());
    }

    #[test]
    fn delete_parser_accepts_positional_name_and_flag() {
        let cli = Cli::try_parse_from(["microvm", "delete", "web-01", "--non-interactive"])
            .expect("valid explicit delete arguments");

        let Some(Command::Delete(arguments)) = cli.command else {
            panic!("delete command should be parsed");
        };
        assert_eq!(arguments.name.as_deref(), Some("web-01"));
        assert!(arguments.non_interactive);
    }

    #[test]
    fn delete_parser_rejects_lifecycle_flags() {
        assert!(Cli::try_parse_from(["microvm", "delete", "--image", "x"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "delete", "--disk-gb", "20"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "delete", "--expose-lan"]).is_err());
    }

    #[test]
    fn ls_parser_accepts_flag_only_form_and_alias() {
        let cli = Cli::try_parse_from(["microvm", "ls", "--non-interactive"])
            .expect("valid explicit ls arguments");
        let Some(Command::Ls(arguments)) = cli.command else {
            panic!("ls command should be parsed");
        };
        assert!(arguments.non_interactive);

        let cli = Cli::try_parse_from(["microvm", "list"])
            .expect("list alias should parse to the same variant");
        let Some(Command::Ls(arguments)) = cli.command else {
            panic!("list alias should parse to Ls");
        };
        assert!(!arguments.non_interactive);
    }

    #[test]
    fn ls_parser_rejects_name_and_lifecycle_flags() {
        assert!(Cli::try_parse_from(["microvm", "ls", "web-01"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "ls", "--name", "web-01"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "ls", "--image", "x"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "ls", "--disk-gb", "20"]).is_err());
    }

    #[test]
    fn prune_parser_accepts_flag_only_form() {
        let cli = Cli::try_parse_from(["microvm", "artifacts", "prune", "--non-interactive"])
            .expect("valid explicit prune arguments");
        let Some(Command::Artifacts(arguments)) = cli.command else {
            panic!("prune command should be parsed");
        };
        let ArtifactsCommand::Prune(prune) = arguments.command else {
            panic!("prune command should be parsed");
        };
        assert!(prune.non_interactive);

        let cli =
            Cli::try_parse_from(["microvm", "artifacts", "prune"]).expect("bare prune parses");
        let Some(Command::Artifacts(arguments)) = cli.command else {
            panic!("prune command should be parsed");
        };
        let ArtifactsCommand::Prune(prune) = arguments.command else {
            panic!("prune command should be parsed");
        };
        assert!(!prune.non_interactive);
    }

    #[test]
    fn prune_parser_rejects_positionals_and_selection_flags() {
        assert!(Cli::try_parse_from(["microvm", "artifacts", "prune", "web-01"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "artifacts", "prune", "--image", "x=y"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "artifacts", "prune", "--name", "x"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "artifacts", "prune", "--disk-gb", "20"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "artifacts", "prune", "--memory", "2GB"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "artifacts", "prune", "--vcpus", "2"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "artifacts", "prune", "--expose-lan"]).is_err());
    }

    #[test]
    fn ssh_parser_accepts_name_and_trailing_remote_command() {
        let cli = Cli::try_parse_from([
            "microvm",
            "ssh",
            "--non-interactive",
            "web-01",
            "--",
            "uname",
            "-a",
        ])
        .expect("valid explicit ssh arguments");

        let Some(Command::Ssh(arguments)) = cli.command else {
            panic!("ssh command should be parsed");
        };
        assert_eq!(arguments.name.as_deref(), Some("web-01"));
        assert!(arguments.non_interactive);
        assert_eq!(arguments.command, ["uname", "-a"]);
    }

    #[test]
    fn ssh_parser_treats_everything_after_separator_as_remote_command() {
        let cli = Cli::try_parse_from(["microvm", "ssh", "--", "web-01", "uname", "-a"])
            .expect("remote command after separator");

        let Some(Command::Ssh(arguments)) = cli.command else {
            panic!("ssh command should be parsed");
        };
        assert!(arguments.name.is_none());
        assert_eq!(arguments.command, ["web-01", "uname", "-a"]);
    }

    #[test]
    fn ssh_parser_rejects_lifecycle_flags() {
        assert!(
            Cli::try_parse_from(["microvm", "ssh", "--", "--image", "x"]).is_err()
                || Cli::try_parse_from(["microvm", "ssh", "--image", "x"]).is_err()
        );
        assert!(Cli::try_parse_from(["microvm", "ssh", "--disk-gb", "20"]).is_err());
        assert!(Cli::try_parse_from(["microvm", "ssh", "--expose-lan"]).is_err());
    }
}
