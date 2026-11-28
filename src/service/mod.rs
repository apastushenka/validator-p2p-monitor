//! The monitor as a service, which takes the validators in turn rather than in passes over the
//! whole set.

mod schedule;

use std::{cmp, pin::pin, time::Duration};

use futures::{Stream, StreamExt, stream};
use tokio::time::{self, Instant, Interval, MissedTickBehavior};
use tokio_util::task::JoinMap;

use self::schedule::Schedule;
use crate::{
    chain::{Chain, Validator, ValidatorSet},
    check::{self, Outcome},
    network::Network,
};

/// The longest the session goes unread. Half the check interval keeps a change of the set from
/// going unnoticed for a whole interval; this cap keeps the set fresh with a long interval too.
const MAX_SESSION_POLL: Duration = Duration::from_secs(10 * 60);

/// The first check that fails stops the service.
pub async fn run(chain: Chain, network: Network, interval: Duration) -> anyhow::Result<()> {
    let ValidatorSet { session, validators } = chain.current_validators().await?;
    let session_poll = cmp::min(interval / 2, MAX_SESSION_POLL);
    // Polled alongside the checks, so that trouble with the RPC holds up only the next update of
    // the set, not the checks.
    let mut validator_set_updates = pin!(watch_validator_set(&chain, session, session_poll));

    let mut schedule = Schedule::new(validators, interval);
    let mut checks = JoinMap::new();
    loop {
        tokio::select! {
            Some(validator) = schedule.next_check() => {
                // A validator whose last check is still running lets its turn pass, so two checks
                // of it never run at once.
                if checks.contains_key(&validator.account) {
                    tracing::warn!(
                        account = %validator.account.display(chain.ss58_prefix()),
                        "skipped a check: the last one is still running",
                    );
                } else {
                    checks.spawn(
                        validator.account,
                        check_task(network.clone(), validator, chain.ss58_prefix()),
                    );
                }
            }
            Some((_, finished)) = checks.join_next() => {
                finished??;
            }
            Some(set) = validator_set_updates.next() => {
                let (added, removed) = schedule.update(set.validators);
                tracing::info!(session = set.session, added, removed, "updated the validator set");
            }
        }
    }
}

#[tracing::instrument(
    name = "validator",
    skip_all,
    fields(account = %validator.account.display(ss58_prefix)),
)]
async fn check_task(
    network: Network,
    validator: Validator,
    ss58_prefix: u16,
) -> anyhow::Result<Outcome> {
    check::run(network, validator).await
}

/// The validators of each session after `session`, from a read of the session index every
/// `period`.
fn watch_validator_set(
    chain: &Chain,
    session: u32,
    period: Duration,
) -> impl Stream<Item = ValidatorSet> + '_ {
    let mut ticks = time::interval_at(Instant::now() + period, period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);

    stream::unfold((ticks, session), move |(mut ticks, session)| async move {
        let set = next_validator_set(chain, &mut ticks, session).await;
        let session = set.session;

        Some((set, (ticks, session)))
    })
}

/// Reads the session at each tick until one other than `known` begins. A failed read is only
/// logged, as the checks need the DHT rather than the RPC, and the next tick retries.
async fn next_validator_set(chain: &Chain, ticks: &mut Interval, known: u32) -> ValidatorSet {
    loop {
        ticks.tick().await;
        match read_validator_set(chain, known).await {
            Ok(Some(set)) => return set,
            Ok(None) => {}
            Err(err) => tracing::warn!(
                "{}; the validator set stays as it was",
                crate::message(&err)
            ),
        }
    }
}

/// The validators of the current session, unless it is still `known`.
async fn read_validator_set(chain: &Chain, known: u32) -> anyhow::Result<Option<ValidatorSet>> {
    let session = chain.current_session().await?;

    if session != known {
        let set = chain.current_validators().await?;
        Ok(Some(set))
    } else {
        Ok(None)
    }
}
