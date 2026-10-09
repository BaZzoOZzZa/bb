use std::time::Duration;

use tokio::time::Instant;

#[derive(Debug, Clone)]
pub struct DebounceScheduler {
    debounce: Duration,
    max_wait: Duration,
    first_scheduled_at: Option<Instant>,
    deadline: Option<Instant>,
}

impl DebounceScheduler {
    pub fn new(debounce: Duration, max_wait: Duration) -> Self {
        Self {
            debounce,
            max_wait: max_wait.max(debounce),
            first_scheduled_at: None,
            deadline: None,
        }
    }

    pub fn schedule(&mut self, now: Instant) {
        let first = *self.first_scheduled_at.get_or_insert(now);
        self.deadline = Some((now + self.debounce).min(first + self.max_wait));
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub fn is_due(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| deadline <= now)
    }

    pub fn reset(&mut self) {
        self.first_scheduled_at = None;
        self.deadline = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_debounce_is_capped_by_max_wait() {
        let start = Instant::now();
        let mut scheduler =
            DebounceScheduler::new(Duration::from_millis(50), Duration::from_millis(200));
        assert_eq!(scheduler.deadline(), None);
        scheduler.schedule(start);
        assert_eq!(
            scheduler.deadline(),
            Some(start + Duration::from_millis(50))
        );
        scheduler.schedule(start + Duration::from_millis(40));
        assert_eq!(
            scheduler.deadline(),
            Some(start + Duration::from_millis(90))
        );
        scheduler.schedule(start + Duration::from_millis(180));
        assert_eq!(
            scheduler.deadline(),
            Some(start + Duration::from_millis(200))
        );
        assert!(!scheduler.is_due(start + Duration::from_millis(199)));
        assert!(scheduler.is_due(start + Duration::from_millis(200)));
        scheduler.reset();
        assert_eq!(scheduler.deadline(), None);
        scheduler.schedule(start + Duration::from_millis(500));
        assert_eq!(
            scheduler.deadline(),
            Some(start + Duration::from_millis(550))
        );
    }
}
