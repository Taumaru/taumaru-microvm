mod cli;
mod commands;
mod context;
mod error;
mod output;

use std::process::ExitCode;

use clap::{CommandFactory, Parser};

use crate::cli::Cli;

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::args_os().len() == 1 {
        return print_help();
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = error.exit_code();
            let _ = error.print();
            return ExitCode::from(exit_code as u8);
        }
    };

    match commands::run(cli).await {
        Ok(exit_code) => ExitCode::from(exit_code),
        Err(error) => {
            eprintln!(
                "{}",
                error.user_message(crate::context::TerminalCapabilities::detect().color)
            );
            ExitCode::from(error.exit_code())
        }
    }
}

fn print_help() -> ExitCode {
    let mut command = Cli::command();
    match command.print_help() {
        Ok(()) => {
            println!();
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Failed to render help: {error}");
            ExitCode::from(1)
        }
    }
}
