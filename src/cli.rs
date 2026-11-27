//! The command line: every parameter of the monitor is a flag.

use clap::Parser;

use crate::network::Bootnode;

#[derive(Parser)]
#[command(version)]
pub struct Cli {
    /// RPC endpoint of a node of the chain: wss, or ws on localhost.
    #[arg(long, value_name = "URL")]
    pub rpc: String,

    /// Boot nodes of the chain's network, as in the node's own --bootnodes: multiaddresses
    /// that end in /p2p/<peer id>, such as the bootNodes of the chain spec.
    #[arg(long, value_name = "ADDR", num_args = 1.., required = true)]
    pub bootnodes: Vec<Bootnode>,

    /// How often each validator is checked, in seconds. After the start, the first check of every
    /// validator begins within one interval.
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 600,
        value_parser = clap::value_parser!(u64).range(1..),
    )]
    pub check_interval_seconds: u64,
}

/// Parse the process arguments, or exit with clap's own message and status.
///
/// Keeping this here rather than calling `Cli::parse()` from `main` is what confines
/// the argument parser to this module: nothing else in the crate names clap.
pub fn parse() -> Cli {
    Cli::parse()
}
