//! What one pass prints to stdout.

use serde::Serialize;

use crate::chain::Validator;

#[derive(Serialize)]
pub struct Report {
    validators: Vec<ValidatorReport>,
}

#[derive(Serialize)]
struct ValidatorReport {
    account: String,
    authority_discovery_key: String,
}

impl Report {
    pub fn new(validators: &[Validator]) -> Self {
        Self {
            validators: validators
                .iter()
                .map(|validator| ValidatorReport {
                    account: to_hex(&validator.account),
                    authority_discovery_key: to_hex(&validator.authority_discovery_key),
                })
                .collect(),
        }
    }
}

fn to_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}
