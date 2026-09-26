pub(crate) mod autostart;
pub(crate) mod delete;
pub(crate) mod download;
pub(crate) mod ls;
pub(crate) mod new;
pub(crate) mod prune;
pub(crate) mod restore;
pub(crate) mod snapshot;
pub(crate) mod ssh;
pub(crate) mod start;
pub(crate) mod stop;

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
            ArtifactsCommand::Prune(prune) => prune::run(&context, prune).await,
        },
        crate::cli::Command::New(arguments) => new::run(&context, arguments).await,
        crate::cli::Command::Start(arguments) => start::run(&context, arguments).await,
        crate::cli::Command::Ssh(arguments) => ssh::run(&context, arguments).await,
        crate::cli::Command::Stop(arguments) => stop::run(&context, arguments).await,
        crate::cli::Command::Snapshot(arguments) => snapshot::run(&context, arguments).await,
        crate::cli::Command::Restore(arguments) => restore::run(&context, arguments).await,
        crate::cli::Command::Delete(arguments) => delete::run(&context, arguments).await,
        crate::cli::Command::Ls(arguments) => ls::run(&context, arguments).await,
        crate::cli::Command::Autostart(arguments) => autostart::run(&context, arguments).await,
    }
}
