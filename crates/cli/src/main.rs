use clap::{CommandFactory, Parser};

#[derive(Debug, Parser)]
#[command(
    name = "microvm",
    version,
    about = "Build and manage Firecracker MicroVMs on Linux."
)]
struct Cli {}

fn main() {
    if std::env::args_os().len() == 1 {
        print_help();
        return;
    }

    let _ = Cli::parse();
}

fn print_help() {
    let mut command = Cli::command();

    if let Err(error) = command.print_help() {
        eprintln!("Failed to render help: {error}");
        std::process::exit(1);
    }

    println!();
}
