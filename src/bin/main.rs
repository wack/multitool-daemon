use clap::{CommandFactory, Parser};
use rust_cli_template::cli::Cli;

#[tokio::main]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    dispatch_command(cli).await
}

fn empty_command() -> miette::Result<()> {
    Cli::command()
        .print_long_help()
        .expect("unable to print help message");
    Ok(())
}

async fn dispatch_command(cli: Cli) -> miette::Result<()> {
    match cli.cmd {
        None => empty_command(),
        Some(cmd) => cmd.dispatch().await,
    }
}
