use std::time::{Duration, Instant};

const LARGE_CAPACITY: usize = 4096;
const MIN_RETAINED_CAPACITY: usize = 512;
const LOW_USAGE_GRACE: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(crate) struct DispatchNodeCapacityDecay {
    low_usage_since: Option<Instant>,
    low_usage_peak: usize,
}

impl DispatchNodeCapacityDecay {
    pub(crate) fn clear<T>(&mut self, nodes: &mut Vec<T>) {
        self.clear_with_clock(nodes, Instant::now);
    }

    fn clear_with_clock<T>(&mut self, nodes: &mut Vec<T>, now: impl FnOnce() -> Instant) {
        let used = nodes.len();
        nodes.clear();
        let capacity = nodes.capacity();
        if capacity <= LARGE_CAPACITY || used > capacity / 4 {
            self.reset();
            return;
        }
        let now = now();
        let since = *self.low_usage_since.get_or_insert(now);
        self.low_usage_peak = self.low_usage_peak.max(used);
        if now.saturating_duration_since(since) < LOW_USAGE_GRACE {
            return;
        }
        let retained = self
            .low_usage_peak
            .saturating_mul(2)
            .max(MIN_RETAINED_CAPACITY)
            .checked_next_power_of_two()
            .unwrap_or(capacity);
        if retained < capacity {
            nodes.shrink_to(retained);
        }
        self.reset();
    }

    fn reset(&mut self) {
        self.low_usage_since = None;
        self.low_usage_peak = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn small_and_busy_buffers_do_not_query_the_clock() {
        let mut policy = DispatchNodeCapacityDecay::default();
        let mut small = vec![0_u8; 100];
        policy.clear_with_clock(&mut small, || panic!("small buffers need no clock"));
        assert!(small.is_empty());
        let mut busy = vec![0_u8; 16_384];
        policy.clear_with_clock(&mut busy, || panic!("busy buffers need no clock"));
        assert!(busy.is_empty());
        assert_eq!(busy.capacity(), 16_384);
    }

    #[test]
    fn low_usage_retains_capacity_until_a_later_clear_after_grace() {
        let mut policy = DispatchNodeCapacityDecay::default();
        let mut nodes = Vec::<u64>::with_capacity(16_384);
        let started = Instant::now();
        nodes.resize(100, 7);
        policy.clear_with_clock(&mut nodes, || started);
        assert_eq!(nodes.capacity(), 16_384);
        nodes.resize(100, 9);
        policy.clear_with_clock(&mut nodes, || started + Duration::from_secs(29));
        assert_eq!(nodes.capacity(), 16_384);
        nodes.resize(100, 11);
        policy.clear_with_clock(&mut nodes, || started + LOW_USAGE_GRACE);
        assert!(nodes.is_empty());
        assert!(nodes.capacity() >= MIN_RETAINED_CAPACITY);
        assert!(nodes.capacity() <= 1024);
    }

    #[test]
    fn retained_capacity_covers_the_largest_demand_in_the_grace_window() {
        let mut policy = DispatchNodeCapacityDecay::default();
        let mut nodes = Vec::<u64>::with_capacity(16_384);
        let started = Instant::now();
        nodes.resize(1800, 1);
        policy.clear_with_clock(&mut nodes, || started);
        nodes.resize(100, 2);
        policy.clear_with_clock(&mut nodes, || started + LOW_USAGE_GRACE);
        assert!(nodes.capacity() >= 3600);
        assert!(nodes.capacity() <= 8192);
    }

    #[test]
    fn renewed_busy_demand_restarts_the_observation_window() {
        let mut policy = DispatchNodeCapacityDecay::default();
        let mut nodes = Vec::<u64>::with_capacity(16_384);
        let started = Instant::now();
        nodes.resize(100, 1);
        policy.clear_with_clock(&mut nodes, || started);
        nodes.resize(9000, 2);
        policy.clear_with_clock(&mut nodes, || panic!("busy demand resets without a clock"));
        nodes.resize(100, 3);
        policy.clear_with_clock(&mut nodes, || started + Duration::from_secs(29));
        nodes.resize(100, 4);
        policy.clear_with_clock(&mut nodes, || started + Duration::from_secs(31));
        assert_eq!(nodes.capacity(), 16_384);
        nodes.resize(100, 5);
        policy.clear_with_clock(&mut nodes, || started + Duration::from_secs(59));
        assert!(nodes.capacity() <= 1024);
    }

    #[test]
    fn regrowth_preserves_new_contents_after_reclamation() {
        let mut policy = DispatchNodeCapacityDecay::default();
        let mut nodes = Vec::<usize>::with_capacity(16_384);
        let started = Instant::now();
        policy.clear_with_clock(&mut nodes, || started);
        policy.clear_with_clock(&mut nodes, || started + LOW_USAGE_GRACE);
        nodes.extend(0..16_384);
        assert_eq!(nodes, (0..16_384).collect::<Vec<_>>());
        policy.clear(&mut nodes);
        assert!(nodes.is_empty());
        assert!(nodes.capacity() >= 16_384);
    }

    #[test]
    fn clears_drop_each_value_once_before_reclaiming_capacity() {
        struct DropCounter(Arc<AtomicUsize>);
        impl Drop for DropCounter {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let dropped = Arc::new(AtomicUsize::new(0));
        let mut nodes = Vec::with_capacity(16_384);
        let mut policy = DispatchNodeCapacityDecay::default();
        let started = Instant::now();
        nodes.extend((0..100).map(|_| DropCounter(dropped.clone())));
        policy.clear_with_clock(&mut nodes, || started);
        assert_eq!(dropped.load(Ordering::Relaxed), 100);
        nodes.extend((0..80).map(|_| DropCounter(dropped.clone())));
        policy.clear_with_clock(&mut nodes, || started + LOW_USAGE_GRACE);
        assert_eq!(dropped.load(Ordering::Relaxed), 180);
        drop(nodes);
        assert_eq!(dropped.load(Ordering::Relaxed), 180);
    }

    #[test]
    fn backwards_test_clock_does_not_reclaim_early() {
        let mut policy = DispatchNodeCapacityDecay::default();
        let mut nodes = Vec::<u64>::with_capacity(16_384);
        let started = Instant::now();
        policy.clear_with_clock(&mut nodes, || started + Duration::from_secs(10));
        policy.clear_with_clock(&mut nodes, || started);
        assert_eq!(nodes.capacity(), 16_384);
    }
}
