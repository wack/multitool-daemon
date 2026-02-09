mod colors;
pub mod controller;

pub use colors::EnableColors;

use clap::{Parser, Subcommand};
use tracing::level_filters::LevelFilter;

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum LogFormat {
    Text,
    JSON,
}

impl Default for LogFormat {
    fn default() -> Self {
        Self::Text
    }
}

#[derive(Debug, Parser, Clone)]
pub struct Cli {
    #[arg(long, env = "LOG_LEVEL", default_value = "info", global = true)]
    pub log_level: LevelFilter,

    #[arg(long, env = "LOG_FORMAT", default_value = "text", global = true)]
    pub log_format: LogFormat,

    #[arg(long, default_value = "auto", global = true)]
    pub enable_colors: EnableColors,

    #[command(subcommand)]
    pub cmd: Option<CliCommand>,
}

#[derive(Debug, Subcommand, Clone)]
pub enum CliCommand {
    /// Print the CLI version and exit
    Version,
    /// Run the HelmRelease Kubernetes controller
    Controller(controller::Controller),
}

impl CliCommand {
    pub async fn dispatch(self) -> miette::Result<()> {
        match self {
            CliCommand::Version => {
                println!("{} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
                Ok(())
            }
            CliCommand::Controller(ctrl) => ctrl.dispatch().await,
        }
    }
}
