pub(crate) mod download;

use crate::cli::{Cli, Command};
use crate::context::CliContext;

pub(crate) async fn run(cli: Cli) -> Result<u8, crate::error::CliError> {
    let Some(command) = cli.command else {
        return Ok(0);
    };

    let context = CliContext::new()?;
    match command {
        Command::Download(arguments) => download::run(&context, arguments).await,
    }
}
