//! What the monitor reads from the chain over RPC.

use std::time::Duration;

use anyhow::{Context, anyhow, ensure};
use subxt::{
    OnlineClient, OnlineClientAtBlock, SubstrateConfig,
    ext::scale_decode::DecodeAsType,
    rpcs::{RpcClient, client::ReconnectingRpcClient, utils::validate_url_is_secure},
    storage::{Address, DynamicAddress},
};

use crate::account::Account;

/// How long the RPC endpoint may take to connect or to answer a read. The client retries a failed
/// connection without end, the first one included, and a call made while it reconnects waits
/// rather than fails, so without this limit the monitor would wait in silence.
const RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// A validator of the current session with the key it publishes its addresses under.
#[derive(Clone)]
pub struct Validator {
    /// The stash account.
    pub account: Account,
    /// An sr25519 public key.
    pub authority_discovery_key: [u8; 32],
}

/// The validators of one session, read at one block.
pub struct ValidatorSet {
    pub session: u32,
    pub validators: Vec<Validator>,
}

pub struct Chain {
    client: OnlineClient<SubstrateConfig>,
}

impl Chain {
    /// Once connected, the client reconnects by itself whenever the connection drops.
    pub async fn connect(rpc: &str) -> anyhow::Result<Self> {
        let client = with_deadline(open_client(rpc))
            .await
            .context("failed to connect to the RPC endpoint")?;
        tracing::info!(genesis_hash = ?client.genesis_hash(), "connected to the chain");

        Ok(Self { client })
    }

    pub fn genesis_hash(&self) -> [u8; 32] {
        self.client.genesis_hash().0
    }

    pub async fn current_session(&self) -> anyhow::Result<u32> {
        with_deadline(self.read_session()).await
    }

    pub async fn current_validators(&self) -> anyhow::Result<ValidatorSet> {
        with_deadline(self.read_validators()).await
    }

    async fn read_session(&self) -> anyhow::Result<u32> {
        let at = self.at_current_block().await?;

        fetch(&at, session::current_index()).await
    }

    async fn read_validators(&self) -> anyhow::Result<ValidatorSet> {
        let at = self.at_current_block().await?;

        let session = fetch(&at, session::current_index()).await?;
        let accounts = fetch(&at, session::validators()).await?;
        let keys = fetch(&at, authority_discovery::keys()).await?;

        let validators = pair(accounts, keys)?;
        tracing::info!(
            block = at.block_number(),
            session,
            validators = validators.len(),
            "read the validators of the current session",
        );

        Ok(ValidatorSet { session, validators })
    }

    async fn at_current_block(&self) -> anyhow::Result<OnlineClientAtBlock<SubstrateConfig>> {
        self.client
            .at_current_block()
            .await
            .context("failed to get the latest finalized block")
    }
}

async fn with_deadline<T>(operation: impl Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
    tokio::time::timeout(RPC_TIMEOUT, operation)
        .await
        .unwrap_or_else(|_| {
            Err(anyhow!(
                "the RPC endpoint did not answer in {} s",
                RPC_TIMEOUT.as_secs()
            ))
        })
}

async fn open_client(rpc: &str) -> anyhow::Result<OnlineClient<SubstrateConfig>> {
    // The same check as `OnlineClient::from_url`, which a client built by hand skips.
    validate_url_is_secure(rpc)?;
    let rpc_client = ReconnectingRpcClient::builder().build(rpc).await?;
    let client = OnlineClient::from_rpc_client(RpcClient::new(rpc_client)).await?;

    Ok(client)
}

/// Match the accounts from `Session::Validators` with the keys from `AuthorityDiscovery::Keys`.
///
/// Neither entry names the other, but they come in the same order: when a session starts,
/// the session pallet fills both from one list.
fn pair(accounts: Vec<Vec<u8>>, keys: Vec<[u8; 32]>) -> anyhow::Result<Vec<Validator>> {
    ensure!(
        accounts.len() == keys.len(),
        "the chain has {} validators but {} authority discovery keys",
        accounts.len(),
        keys.len(),
    );

    accounts
        .into_iter()
        .zip(keys)
        .map(|(account, authority_discovery_key)| {
            let account = Account::try_from(account.as_slice())?;
            anyhow::Ok(Validator {
                account,
                authority_discovery_key,
            })
        })
        .collect()
}

async fn fetch<V: DecodeAsType>(
    at: &OnlineClientAtBlock<SubstrateConfig>,
    address: DynamicAddress<(), V>,
) -> anyhow::Result<V> {
    let name = format!("{}::{}", address.pallet_name(), address.entry_name());

    at.storage()
        .fetch(address, ())
        .await
        .with_context(|| format!("failed to read {name}"))?
        .decode()
        .with_context(|| format!("failed to decode {name}"))
}

// Laid out like subxt's generated API, but dynamic: generated types belong to one runtime,
// and the monitor reads any relay chain.
mod session {
    use subxt::{dynamic, storage::DynamicAddress};

    pub fn current_index() -> DynamicAddress<(), u32> {
        dynamic::storage("Session", "CurrentIndex")
    }

    pub fn validators() -> DynamicAddress<(), Vec<Vec<u8>>> {
        dynamic::storage("Session", "Validators")
    }
}

mod authority_discovery {
    use subxt::{dynamic, storage::DynamicAddress};

    pub fn keys() -> DynamicAddress<(), Vec<[u8; 32]>> {
        dynamic::storage("AuthorityDiscovery", "Keys")
    }
}

#[cfg(test)]
mod tests {
    use super::pair;
    use crate::account::Account;

    #[test]
    fn pair_matches_keys_to_accounts_by_position() {
        let validators = pair(vec![vec![1; 20], vec![2; 20]], vec![[3; 32], [4; 32]]).unwrap();

        let pairs: Vec<_> = validators
            .into_iter()
            .map(|validator| (validator.account, validator.authority_discovery_key))
            .collect();
        assert_eq!(
            pairs,
            [
                (Account::Id20([1; 20]), [3; 32]),
                (Account::Id20([2; 20]), [4; 32]),
            ],
        );
    }

    #[test]
    fn pair_rejects_lists_of_different_length() {
        assert!(pair(vec![vec![1; 20], vec![2; 20]], vec![[3; 32]]).is_err());
    }
}
