//! What one pass prints to stdout.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    chain::Validator,
    network::RecordLookup,
    record::{self, Record},
};

#[derive(Serialize)]
pub struct Report {
    validators: Vec<ValidatorReport>,
}

#[derive(Serialize)]
struct ValidatorReport {
    account: String,
    authority_discovery_key: String,
    lookup: LookupReport,
    /// The newest of the versions that passed verification. Several stay when they differ and
    /// none has a creation time.
    records: Vec<PublishedRecord>,
}

#[derive(Serialize)]
struct LookupReport {
    outcome: Outcome,
    versions: usize,
    peers_answered: u32,
}

#[derive(Serialize)]
struct PublishedRecord {
    peer_id: String,
    addresses: Vec<String>,
    /// RFC 3339 in UTC.
    creation_time: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    /// A version passed verification, even if the lookup then ran out of time.
    Found,
    /// Peers returned the record, but no version could be decoded and verified.
    Invalid,
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
                .map(|(validator, lookup)| {
                    let verified = lookup.versions.iter().filter_map(|value| {
                        Record::decode(value, &validator.authority_discovery_key).ok()
                    });
                    let records = record::newest(verified);

                    ValidatorReport {
                        account: to_hex(&validator.account),
                        authority_discovery_key: to_hex(&validator.authority_discovery_key),
                        lookup: LookupReport::new(lookup, &records),
                        records: records.into_iter().map(PublishedRecord::from).collect(),
                    }
                })
                .collect(),
        }
    }
}

impl LookupReport {
    /// `records` holds the newest of the lookup's versions that passed verification.
    fn new(lookup: &RecordLookup, records: &[Record]) -> Self {
        let outcome = if !records.is_empty() {
            Outcome::Found
        } else if !lookup.versions.is_empty() {
            Outcome::Invalid
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

impl From<Record> for PublishedRecord {
    fn from(record: Record) -> Self {
        Self {
            peer_id: record.peer_id.to_string(),
            addresses: record.addresses.iter().map(ToString::to_string).collect(),
            creation_time: record.creation_time.and_then(rfc3339),
        }
    }
}

/// None past year 9999, which RFC 3339 cannot write. `OffsetDateTime::from(SystemTime)` would
/// panic there instead.
fn rfc3339(time: SystemTime) -> Option<String> {
    let nanos = time.duration_since(UNIX_EPOCH).ok()?.as_nanos();

    OffsetDateTime::from_unix_timestamp_nanos(i128::try_from(nanos).ok()?)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

fn to_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use libp2p::PeerId;

    use super::{LookupReport, Outcome, rfc3339};
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

        LookupReport::new(&lookup, &records).outcome
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

    #[test]
    fn creation_time_is_rfc3339_in_utc() {
        let time = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789);

        assert_eq!(
            rfc3339(time).as_deref(),
            Some("2023-11-14T22:13:20.123456789Z")
        );
    }

    #[test]
    fn creation_time_past_year_9999_is_none() {
        let time = UNIX_EPOCH + Duration::from_secs(300_000_000_000);

        assert_eq!(rfc3339(time), None);
    }
}
