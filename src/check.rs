//! One check of one validator: what the monitor finds out about it in one go. For now that is
//! whether it published a valid record in the DHT.

use std::fmt;

use crate::{
    chain::Validator,
    network::{Network, RecordLookup},
    record::{self, Record},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A version passed verification, even if the lookup then ran out of time.
    Found,
    /// Peers returned the record, but no version could be decoded and verified.
    Invalid,
    /// The lookup ended in time, and no peer returned the record.
    NotFound,
    /// The lookup ran out of time before any peer returned the record.
    Timeout,
}

impl Outcome {
    /// `records` holds the newest of the lookup's versions that passed verification.
    fn new(lookup: &RecordLookup, records: &[Record]) -> Self {
        if !records.is_empty() {
            Self::Found
        } else if !lookup.versions.is_empty() {
            Self::Invalid
        } else if lookup.timed_out {
            Self::Timeout
        } else {
            Self::NotFound
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Found => "found",
            Self::Invalid => "invalid",
            Self::NotFound => "not_found",
            Self::Timeout => "timeout",
        })
    }
}

#[tracing::instrument(
    name = "validator",
    skip_all,
    fields(account = %to_hex(&validator.account)),
)]
pub async fn run(network: Network, validator: Validator) -> anyhow::Result<Outcome> {
    let lookup = network
        .find_record(validator.authority_discovery_key)
        .await?;
    let verified = lookup
        .versions
        .iter()
        .filter_map(|value| Record::decode(value, &validator.authority_discovery_key).ok());
    let records = record::newest(verified);
    let outcome = Outcome::new(&lookup, &records);

    tracing::info!(
        %outcome,
        versions = lookup.versions.len(),
        peers_answered = lookup.peers_answered,
        "checked",
    );

    Ok(outcome)
}

fn to_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use libp2p::PeerId;

    use super::Outcome;
    use crate::{network::RecordLookup, record::Record};

    fn outcome(versions: usize, verified: usize, timed_out: bool) -> Outcome {
        let lookup = RecordLookup {
            versions: vec![b"record".to_vec(); versions],
            timed_out,
            peers_answered: 0,
        };
        let records: Vec<_> = (0..verified)
            .map(|_| Record {
                peer_id: PeerId::random(),
                addresses: Vec::new(),
                creation_time: None,
            })
            .collect();

        Outcome::new(&lookup, &records)
    }

    #[test]
    fn record_verified_before_timeout_is_found() {
        assert!(matches!(outcome(1, 1, true), Outcome::Found));
    }

    #[test]
    fn versions_that_all_fail_verification_are_invalid() {
        assert!(matches!(outcome(2, 0, false), Outcome::Invalid));
        assert!(matches!(outcome(2, 0, true), Outcome::Invalid));
    }

    #[test]
    fn lookup_out_of_time_without_record_is_timeout() {
        assert!(matches!(outcome(0, 0, true), Outcome::Timeout));
    }

    #[test]
    fn lookup_in_time_without_record_is_not_found() {
        assert!(matches!(outcome(0, 0, false), Outcome::NotFound));
    }
}
