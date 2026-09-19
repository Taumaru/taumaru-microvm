pub(crate) mod download;

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
    }
}
