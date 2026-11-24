//! The command line: every parameter of the monitor is a flag.

use clap::Parser;

use crate::dht::Bootnode;

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
}

/// Parse the process arguments, or exit with clap's own message and status.
///
/// Keeping this here rather than calling `Cli::parse()` from `main` is what confines
/// the argument parser to this module: nothing else in the crate names clap.
pub fn parse() -> Cli {
    Cli::parse()
}
