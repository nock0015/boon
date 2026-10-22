//! Counts requested completed source ticks, independently of projected rows.
use std::collections::HashSet;
use std::time::{Duration, Instant};

pub(super) struct SelectedTickProgress {
    remaining: HashSet<i32>,
    total: usize,
    last_emit: Instant,
}
impl SelectedTickProgress {
    pub(super) fn new(selected: &HashSet<i32>, now: Instant) -> Self {
        Self { remaining: selected.clone(), total: selected.len(), last_emit: now }
    }
    pub(super) fn snapshot(&self) -> (usize, usize) {
        (self.total - self.remaining.len(), self.total)
    }
    // Called only after all selected-tick processing succeeds. Duplicate source
    // visits, postgame projection suppression, and EOF cannot inflate the count.
    pub(super) fn observe(&mut self, tick: i32, now: Instant) -> bool {
        if !self.remaining.remove(&tick) { return false; }
        if now.duration_since(self.last_emit) >= Duration::from_millis(100) {
            self.last_emit = now;
            true
        } else { false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unique_plan_and_actual_visits_only() {
        let start = Instant::now();
        let selected = [1, 1, 17, 33].into_iter().collect();
        let mut progress = SelectedTickProgress::new(&selected, start);
        assert_eq!(progress.snapshot(), (0, 3));
        assert!(!progress.observe(2, start + Duration::from_secs(1)));
        assert!(progress.observe(1, start + Duration::from_secs(1)));
        assert!(!progress.observe(1, start + Duration::from_secs(2)));
        assert_eq!(progress.snapshot(), (1, 3));
        assert!(progress.observe(33, start + Duration::from_secs(2)));
        // EOF with requested tick 17 unobserved remains incomplete.
        assert_eq!(progress.snapshot(), (2, 3));
    }
    #[test]
    fn throttling_preserves_every_observation_and_final_snapshot() {
        let start = Instant::now();
        let mut progress = SelectedTickProgress::new(&[1, 17, 33].into_iter().collect(), start);
        assert!(!progress.observe(1, start));
        assert!(!progress.observe(17, start + Duration::from_millis(99)));
        assert_eq!(progress.snapshot(), (2, 3));
        assert!(progress.observe(33, start + Duration::from_millis(100)));
        assert_eq!(progress.snapshot(), (3, 3));
    }
    #[test]
    fn empty_plan_is_zero_over_zero() {
        let progress = SelectedTickProgress::new(&HashSet::new(), Instant::now());
        assert_eq!(progress.snapshot(), (0, 0));
    }
}
