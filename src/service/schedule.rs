use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

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

    /// A validator new to the set takes one of the next turns, and one that left the set is no
    /// longer checked; the others keep their order and take their keys from the new set. Returns
    /// how many were added and removed.
    pub fn update(&mut self, validators: Vec<Validator>) -> (usize, usize) {
        let mut new_set: HashMap<_, _> = validators
            .into_iter()
            .map(|validator| (validator.account.clone(), validator))
            .collect();

        let before = self.validators.len();
        // Each validator that stays is taken out of the new set.
        self.validators
            .retain_mut(|validator| match new_set.remove(&validator.account) {
                Some(update) => {
                    *validator = update;
                    true
                }
                None => false,
            });
        let removed = before - self.validators.len();

        // What is left of the new set is the newcomers.
        let added = new_set.len();
        for validator in new_set.into_values() {
            self.validators.push_front(validator);
        }

        (added, removed)
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

    fn validator(index: u8) -> Validator {
        Validator {
            account: vec![index],
            authority_discovery_key: [index; 32],
        }
    }

    fn validators(count: u8) -> Vec<Validator> {
        (0..count).map(validator).collect()
    }

    async fn next_account(schedule: &mut Schedule) -> u8 {
        schedule.next_check().await.unwrap().account[0]
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

    #[tokio::test(start_paused = true)]
    async fn newcomer_is_checked_next_and_the_step_follows_the_set() {
        let start = Instant::now();
        let mut schedule = Schedule::new(validators(2), Duration::from_secs(300));
        assert_eq!(next_account(&mut schedule).await, 0);

        assert_eq!(schedule.update(validators(3)), (1, 0));
        assert_eq!(next_account(&mut schedule).await, 2);
        assert_eq!(start.elapsed(), Duration::from_secs(150));
        assert_eq!(next_account(&mut schedule).await, 1);
        assert_eq!(start.elapsed(), Duration::from_secs(250));
        assert_eq!(next_account(&mut schedule).await, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn departed_validator_is_no_longer_checked() {
        let mut schedule = Schedule::new(validators(3), Duration::from_secs(300));
        assert_eq!(next_account(&mut schedule).await, 0);

        assert_eq!(schedule.update(vec![validator(0), validator(2)]), (0, 1));
        let turns = [
            next_account(&mut schedule).await,
            next_account(&mut schedule).await,
            next_account(&mut schedule).await,
        ];
        assert_eq!(turns, [2, 0, 2]);
    }

    #[tokio::test(start_paused = true)]
    async fn newcomer_to_an_empty_set_is_checked_at_once() {
        let start = Instant::now();
        let mut schedule = Schedule::new(Vec::new(), TEN_MINUTES);
        assert!(schedule.next_check().await.is_none());
        tokio::time::advance(TEN_MINUTES).await;

        schedule.update(validators(1));
        assert_eq!(next_account(&mut schedule).await, 0);
        assert_eq!(start.elapsed(), TEN_MINUTES);
    }

    #[tokio::test(start_paused = true)]
    async fn changed_key_is_used_from_the_next_check() {
        let mut schedule = Schedule::new(validators(1), TEN_MINUTES);
        let rotated = Validator {
            account: vec![0],
            authority_discovery_key: [9; 32],
        };

        assert_eq!(schedule.update(vec![rotated]), (0, 0));
        let validator = schedule.next_check().await.unwrap();
        assert_eq!(validator.authority_discovery_key, [9; 32]);
    }
}
