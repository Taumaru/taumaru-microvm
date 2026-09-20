pub(crate) mod download;
pub(crate) mod new;
pub(crate) mod ssh;
pub(crate) mod start;

use crate::cli::{ArtifactsCommand, Cli};
use crate::context::CliContext;

pub(crate) async fn run(cli: Cli) -> Result<u8, crate::error::CliError> {
    let Some(command) = cli.command else {
        return Ok(0);
    };

    let context = CliContext::new()?;
    match command {
        crate::cli::Command::Artifacts(arguments) => match arguments.command {
            ArtifactsCommand::Download(download) => download::run(&context, download).await,
        },
        crate::cli::Command::New(arguments) => new::run(&context, arguments).await,
        crate::cli::Command::Ssh(arguments) => ssh::run(&context, arguments).await,
        crate::cli::Command::Start(arguments) => start::run(&context, arguments).await,
    }
}
