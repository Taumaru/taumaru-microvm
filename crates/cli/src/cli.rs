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
    /// Select and download runtime, kernel, and distribution artifacts.
    Download(DownloadArgs),
}

#[derive(Debug, Args, Clone)]
pub(crate) struct DownloadArgs {
    /// Select a distribution by registry ID. Repeat for multiple distributions.
    #[arg(long = "distribution", value_name = "DISTRIBUTION_ID")]
    pub(crate) distributions: Vec<String>,

    /// Select a kernel for a distribution as DISTRIBUTION_ID=KERNEL_ID. Repeat per distribution.
    #[arg(long = "kernel", value_name = "DISTRIBUTION_ID=KERNEL_ID")]
    pub(crate) kernels: Vec<String>,

    /// Disable prompts and require complete explicit selections.
    #[arg(long = "non-interactive")]
    pub(crate) non_interactive: bool,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command};

    #[test]
    fn download_parser_accepts_repeatable_explicit_options() {
        let cli = Cli::try_parse_from([
            "microvm",
            "download",
            "--non-interactive",
            "--distribution",
            "distro-a",
            "--distribution",
            "distro-z",
            "--kernel",
            "distro-a=kernel-a",
            "--kernel",
            "distro-z=kernel-z",
        ])
        .expect("valid explicit download arguments");

        let Some(Command::Download(arguments)) = cli.command else {
            panic!("download command should be parsed");
        };
        assert!(arguments.non_interactive);
        assert_eq!(arguments.distributions, ["distro-a", "distro-z"]);
        assert_eq!(
            arguments.kernels,
            ["distro-a=kernel-a", "distro-z=kernel-z"]
        );
    }
}
