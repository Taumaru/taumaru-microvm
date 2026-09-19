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
}
