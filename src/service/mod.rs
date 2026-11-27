//! The monitor as a service, which checks the validators one at a time in turn rather than in
//! passes over the whole set.

mod schedule;

use std::{collections::HashSet, time::Duration};

use anyhow::bail;
use tokio::task::JoinSet;

use self::schedule::Schedule;
use crate::{chain::Validator, check, network::Network};

/// The first check that fails stops the service.
pub async fn run(
    network: Network,
    validators: Vec<Validator>,
    interval: Duration,
) -> anyhow::Result<()> {
    let mut schedule = Schedule::new(validators, interval);
    let mut checks = JoinSet::new();
    // The accounts whose checks run.
    let mut checking = HashSet::new();
    loop {
        tokio::select! {
            Some(validator) = schedule.next_check() => {
                // A validator whose last check still runs lets its turn pass, so two checks of it
                // never run at once.
                if checking.insert(validator.account.clone()) {
                    let account = validator.account.clone();
                    let network = network.clone();
                    checks.spawn(async move {
                        check::run(network, validator).await?;
                        anyhow::Ok(account)
                    });
                }
            }
            Some(finished) = checks.join_next() => {
                let account = finished??;
                checking.remove(&account);
            }
            else => bail!("the current session has no validators"),
        }
    }
}
