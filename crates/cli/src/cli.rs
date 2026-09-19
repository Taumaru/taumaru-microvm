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
        let ArtifactsCommand::Download(download) = arguments.command;
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
}
