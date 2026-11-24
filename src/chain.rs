//! What the monitor reads from the chain over RPC.

use anyhow::{Context, ensure};
use subxt::{
    OnlineClient, OnlineClientAtBlock, SubstrateConfig,
    ext::scale_decode::DecodeAsType,
    storage::{Address, DynamicAddress},
};

/// A validator of the current session with the key it publishes its addresses under.
pub struct Validator {
    /// The stash account: 20 or 32 bytes, depending on the chain.
    pub account: Vec<u8>,
    /// An sr25519 public key.
    pub authority_discovery_key: [u8; 32],
}

pub async fn current_validators(rpc: &str) -> anyhow::Result<Vec<Validator>> {
    let client = OnlineClient::<SubstrateConfig>::from_url(rpc)
        .await
        .context("failed to connect to the RPC endpoint")?;
    let at = client
        .at_current_block()
        .await
        .context("failed to get the latest finalized block")?;

    let accounts = fetch(&at, session::validators()).await?;
    let keys = fetch(&at, authority_discovery::keys()).await?;

    pair(accounts, keys)
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

    Ok(accounts
        .into_iter()
        .zip(keys)
        .map(|(account, authority_discovery_key)| Validator {
            account,
            authority_discovery_key,
        })
        .collect())
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

    #[test]
    fn pair_matches_keys_to_accounts_by_position() {
        let validators = pair(vec![vec![1; 20], vec![2; 20]], vec![[3; 32], [4; 32]]).unwrap();

        let pairs: Vec<_> = validators
            .into_iter()
            .map(|validator| (validator.account, validator.authority_discovery_key))
            .collect();
        assert_eq!(pairs, [(vec![1; 20], [3; 32]), (vec![2; 20], [4; 32])]);
    }

    #[test]
    fn pair_rejects_lists_of_different_length() {
        assert!(pair(vec![vec![1; 20], vec![2; 20]], vec![[3; 32]]).is_err());
    }
}
