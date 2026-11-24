//! The command line: every parameter of the monitor is a flag.

use clap::Parser;

#[derive(Parser)]
#[command(version)]
pub struct Cli {
    /// RPC endpoint of a node of the chain: wss, or ws on localhost.
    #[arg(long, value_name = "URL")]
    pub rpc: String,
}

/// Parse the process arguments, or exit with clap's own message and status.
///
/// Keeping this here rather than calling `Cli::parse()` from `main` is what confines
/// the argument parser to this module: nothing else in the crate names clap.
pub fn parse() -> Cli {
    Cli::parse()
}
