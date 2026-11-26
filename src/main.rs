mod chain;
mod cli;
mod network;
mod report;

use std::process::ExitCode;

use futures::{StreamExt, TryStreamExt, stream};

use crate::{
    chain::{Chain, Validator},
    network::{Network, RecordLookup},
    report::Report,
};

/// Record lookups that run at once. Without a limit, hundreds of lookups start from the same
/// few peers at once, and most of them end without the record.
const MAX_LOOKUPS: usize = 32;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}: {}", env!("CARGO_PKG_NAME"), message(&err));
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let cli = cli::parse();
    let chain = Chain::connect(&cli.rpc).await?;
    let network = Network::new(chain.genesis_hash(), &cli.bootnodes).await?;
    network.join().await?;
    let validators = chain.current_validators().await?;
    let lookups = find_records(&network, &validators).await?;
    let report = Report::new(&validators, &lookups);
    println!("{}", serde_json::to_string_pretty(&report)?);

    Ok(())
}

/// Look up the record of each validator, and return the lookups in the validators' order.
async fn find_records(
    network: &Network,
    validators: &[Validator],
) -> anyhow::Result<Vec<RecordLookup>> {
    let mut lookups: Vec<_> = stream::iter(validators.iter().enumerate())
        .map(|(index, validator)| async move {
            let lookup = network
                .find_record(validator.authority_discovery_key)
                .await?;
            anyhow::Ok((index, lookup))
        })
        .buffer_unordered(MAX_LOOKUPS)
        .try_collect()
        .await?;
    lookups.sort_by_key(|(index, _)| *index);

    Ok(lookups.into_iter().map(|(_, lookup)| lookup).collect())
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
