use std::{collections::VecDeque, time::Duration};

use tokio::time::{self, Instant};

use crate::chain::Validator;

/// The validators of the set, taking turns at a pace that stays even whatever the size of the set.
pub struct Schedule {
    /// How often each validator comes up: the step between turns is this divided by the number of
    /// validators.
    interval: Duration,
    /// The validator whose turn is next comes first.
    validators: VecDeque<Validator>,
    /// When the first validator takes its turn.
    next_turn: Instant,
}

impl Schedule {
    /// The first turn comes at once.
    pub fn new(validators: Vec<Validator>, interval: Duration) -> Self {
        Self {
            interval,
            validators: validators.into(),
            next_turn: Instant::now(),
        }
    }

    /// Waits for the next turn, and returns its validator. Ends with `None` rather than waiting
    /// when the set is empty. Cancel-safe, as `select!` needs: the schedule moves on only once the
    /// wait has ended.
    pub async fn next_check(&mut self) -> Option<Validator> {
        if self.validators.is_empty() {
            return None;
        }
        time::sleep_until(self.next_turn).await;

        Some(self.take_turn())
    }

    /// The validator whose turn it is, which goes to the back. A turn taken late moves the later
    /// ones back rather than bunching them up.
    fn take_turn(&mut self) -> Validator {
        let step = self.interval.div_f64(self.validators.len() as f64);
        self.next_turn = self.next_turn.max(Instant::now()) + step;

        let validator = self
            .validators
            .pop_front()
            .expect("a turn is taken only with validators");
        self.validators.push_back(validator.clone());

        validator
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::Instant;

    use super::Schedule;
    use crate::chain::Validator;

    const TEN_MINUTES: Duration = Duration::from_secs(600);

    fn validators(count: u8) -> Vec<Validator> {
        (0..count)
            .map(|index| Validator {
                account: vec![index],
                authority_discovery_key: [index; 32],
            })
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn first_checks_spread_over_one_interval() {
        let start = Instant::now();
        let mut schedule = Schedule::new(validators(4), Duration::from_secs(400));

        for (account, after) in [(0, 0), (1, 100), (2, 200), (3, 300)] {
            let validator = schedule.next_check().await.unwrap();
            assert_eq!(validator.account, [account]);
            assert_eq!(start.elapsed(), Duration::from_secs(after));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn validator_comes_up_again_one_interval_later() {
        let start = Instant::now();
        let mut schedule = Schedule::new(validators(1), TEN_MINUTES);
        schedule.next_check().await.unwrap();

        schedule.next_check().await.unwrap();
        assert_eq!(start.elapsed(), TEN_MINUTES);
    }

    #[tokio::test(start_paused = true)]
    async fn late_turn_moves_the_later_ones_back() {
        let start = Instant::now();
        let mut schedule = Schedule::new(validators(2), Duration::from_secs(200));
        schedule.next_check().await.unwrap();

        tokio::time::advance(Duration::from_secs(250)).await;
        schedule.next_check().await.unwrap();
        assert_eq!(start.elapsed(), Duration::from_secs(250));
        let validator = schedule.next_check().await.unwrap();
        assert_eq!(validator.account, [0]);
        assert_eq!(start.elapsed(), Duration::from_secs(350));
    }

    #[tokio::test]
    async fn empty_schedule_ends_at_once() {
        let mut schedule = Schedule::new(Vec::new(), TEN_MINUTES);

        assert!(schedule.next_check().await.is_none());
    }
}
