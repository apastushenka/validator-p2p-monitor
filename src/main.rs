mod chain;
mod cli;
mod report;

use std::process::ExitCode;

use crate::report::Report;

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
    let validators = chain::current_validators(&cli.rpc).await?;
    let report = Report::new(&validators);
    println!("{}", serde_json::to_string_pretty(&report)?);

    Ok(())
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
