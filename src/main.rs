mod address;
mod chain;
mod check;
mod cli;
mod network;
mod record;
mod service;

use std::{
    io::{self, IsTerminal},
    process::ExitCode,
    time::Duration,
};

use tracing_subscriber::{EnvFilter, filter::LevelFilter};

use crate::{chain::Chain, network::Network};

#[tokio::main]
async fn main() -> ExitCode {
    init_logging();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{}", message(&err));
            ExitCode::FAILURE
        }
    }
}

/// `RUST_LOG` sets the levels, and without it the level is `info`.
fn init_logging() {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(io::stdout().is_terminal())
        .init();
}

async fn run() -> anyhow::Result<()> {
    let cli = cli::parse();
    let chain = Chain::connect(&cli.rpc).await?;
    let network = Network::new(chain.genesis_hash(), &cli.bootnodes).await?;
    network.join().await?;

    let interval = Duration::from_secs(cli.check_interval_seconds);
    service::run(chain, network, interval).await
}

/// The error chain without the causes the message already contains: a subxt error
/// repeats its source in its own text.
fn message(err: &anyhow::Error) -> String {
    let mut message = String::new();
    for cause in err.chain().map(ToString::to_string) {
        if !message.contains(&cause) {
            if !message.is_empty() {
                message.push_str(": ");
            }
            message.push_str(&cause);
        }
    }
    message
}
