mod candidate {
    include!("../../src/arena.rs");

    #[cfg(test)]
    mod decay_tests {
        use super::*;
        use std::time::{Duration, Instant};

        fn fill(arena: &mut Arena, count: usize) {
            for index in 0..count {
                let value = arena.alloc(|| [index as u64; 128]);
                assert_eq!(value[127], index as u64);
            }
        }

        #[test]
        fn low_demand_releases_only_after_grace_period() {
            let mut arena = Arena::new(1024 * 1024);
            let start = Instant::now();
            fill(&mut arena, 16_384);
            arena.clear_at(start);
            fill(&mut arena, 32);
            arena.clear_at(start);
            assert_eq!(arena.capacity(), 16 * 1024 * 1024);
            fill(&mut arena, 32);
            arena.clear_at(start + Duration::from_secs(29));
            assert_eq!(arena.capacity(), 16 * 1024 * 1024);
            fill(&mut arena, 32);
            arena.clear_at(start + Duration::from_secs(30));
            assert_eq!(arena.capacity(), 2 * 1024 * 1024);
        }

        #[test]
        fn shrink_invalidates_old_handles_and_can_regrow() {
            let mut arena = Arena::new(1024 * 1024);
            let start = Instant::now();
            fill(&mut arena, 16_384);
            let old_value = arena.alloc(|| 991_u64);
            arena.clear_at(start);
            arena.clear_at(start);
            arena.clear_at(start + Duration::from_secs(30));
            assert_eq!(arena.capacity(), 2 * 1024 * 1024);
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| *old_value)).is_err());
            fill(&mut arena, 16_384);
            assert!(arena.capacity() >= 16 * 1024 * 1024);
            arena.clear_at(start + Duration::from_secs(31));
        }

        #[test]
        fn renewed_large_demand_restarts_the_grace_period() {
            let mut arena = Arena::new(1024 * 1024);
            let start = Instant::now();
            fill(&mut arena, 16_384);
            arena.clear_at(start);
            arena.clear_at(start);
            fill(&mut arena, 16_384);
            arena.clear_at(start + Duration::from_secs(29));
            arena.clear_at(start + Duration::from_secs(30));
            arena.clear_at(start + Duration::from_secs(59));
            assert_eq!(arena.capacity(), 16 * 1024 * 1024);
            arena.clear_at(start + Duration::from_secs(60));
            assert_eq!(arena.capacity(), 2 * 1024 * 1024);
        }

        #[test]
        fn retains_the_recent_window_peak_with_geometric_slack() {
            let mut arena = Arena::new(1024 * 1024);
            let start = Instant::now();
            fill(&mut arena, 16_384);
            arena.clear_at(start);
            fill(&mut arena, 7 * 1024);
            arena.clear_at(start);
            arena.clear_at(start + Duration::from_secs(30));
            assert_eq!(arena.capacity(), 8 * 1024 * 1024);
            arena.clear_at(start + Duration::from_secs(31));
            arena.clear_at(start + Duration::from_secs(61));
            assert_eq!(arena.capacity(), 2 * 1024 * 1024);
        }

        #[test]
        fn all_values_drop_before_their_storage_is_released() {
            struct Payload {
                dropped: Rc<Cell<usize>>,
                padding: [u64; 127],
            }
            impl Drop for Payload {
                fn drop(&mut self) {
                    assert_eq!(self.padding[126], 317);
                    self.dropped.set(self.dropped.get() + 1);
                }
            }
            let dropped = Rc::new(Cell::new(0));
            let mut arena = Arena::new(1024 * 1024);
            let start = Instant::now();
            for _index in 0..16_384 {
                arena.alloc(|| Payload {
                    dropped: dropped.clone(),
                    padding: [317; 127],
                });
            }
            assert_eq!(dropped.get(), 0);
            arena.clear_at(start);
            assert_eq!(dropped.get(), 16_384);
            arena.clear_at(start);
            arena.clear_at(start + Duration::from_secs(30));
            assert_eq!(dropped.get(), 16_384);
            assert_eq!(arena.capacity(), 2 * 1024 * 1024);
        }

        #[test]
        fn steady_high_demand_does_not_shrink() {
            let mut arena = Arena::new(1024 * 1024);
            let start = Instant::now();
            for seconds in [0, 30, 60, 120] {
                fill(&mut arena, 16_384);
                arena.clear_at(start + Duration::from_secs(seconds));
                assert_eq!(arena.capacity(), 16 * 1024 * 1024);
            }
        }
    }
}
