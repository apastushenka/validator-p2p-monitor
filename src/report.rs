//! What one pass prints to stdout.

use serde::Serialize;

use crate::{chain::Validator, network::RecordLookup};

#[derive(Serialize)]
pub struct Report {
    validators: Vec<ValidatorReport>,
}

#[derive(Serialize)]
struct ValidatorReport {
    account: String,
    authority_discovery_key: String,
    record: RecordReport,
}

#[derive(Serialize)]
struct RecordReport {
    outcome: Outcome,
    versions: usize,
    peers_answered: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    /// Some peer returned the record, even if the lookup then ran out of time.
    Found,
    /// The lookup ended in time, and no peer returned the record.
    NotFound,
    /// The lookup ran out of time before any peer returned the record.
    Timeout,
}

impl Report {
    /// The lookups come in the order of the validators.
    pub fn new(validators: &[Validator], lookups: &[RecordLookup]) -> Self {
        Self {
            validators: validators
                .iter()
                .zip(lookups)
                .map(|(validator, lookup)| ValidatorReport {
                    account: to_hex(&validator.account),
                    authority_discovery_key: to_hex(&validator.authority_discovery_key),
                    record: RecordReport::new(lookup),
                })
                .collect(),
        }
    }
}

impl RecordReport {
    fn new(lookup: &RecordLookup) -> Self {
        let outcome = if !lookup.versions.is_empty() {
            Outcome::Found
        } else if lookup.timed_out {
            Outcome::Timeout
        } else {
            Outcome::NotFound
        };

        Self {
            outcome,
            versions: lookup.versions.len(),
            peers_answered: lookup.peers_answered,
        }
    }
}

fn to_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::{Outcome, RecordReport};
    use crate::network::RecordLookup;

    fn outcome(versions: usize, timed_out: bool) -> Outcome {
        let lookup = RecordLookup {
            versions: vec![b"record".to_vec(); versions],
            timed_out,
            peers_answered: 0,
        };
        RecordReport::new(&lookup).outcome
    }

    #[test]
    fn record_found_before_timeout_is_found() {
        assert!(matches!(outcome(1, true), Outcome::Found));
    }

    #[test]
    fn lookup_out_of_time_without_record_is_timeout() {
        assert!(matches!(outcome(0, true), Outcome::Timeout));
    }

    #[test]
    fn lookup_in_time_without_record_is_not_found() {
        assert!(matches!(outcome(0, false), Outcome::NotFound));
    }
}
