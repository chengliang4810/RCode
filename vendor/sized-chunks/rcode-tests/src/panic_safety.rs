use sized_chunks::{Chunk, InlineArray, RingBuffer};
use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use typenum::U16;

const ELEMENTS: usize = 8;

// Keep ownership out of this probe, so the unfixed baseline deterministically
// records repeated drops instead of corrupting the allocator or aborting.
struct DropProbe<'a> {
    id: usize,
    drops: &'a [Cell<usize>; ELEMENTS],
    panic_on: usize,
    panicked: &'a Cell<bool>,
}

impl Drop for DropProbe<'_> {
    fn drop(&mut self) {
        let count = &self.drops[self.id];
        count.set(count.get() + 1);
        if self.id == self.panic_on && !self.panicked.replace(true) {
            panic!("intentional element destructor panic");
        }
    }
}

fn probes<'a>(
    drops: &'a [Cell<usize>; ELEMENTS],
    panic_on: usize,
    panicked: &'a Cell<bool>,
) -> impl Iterator<Item = DropProbe<'a>> {
    (0..ELEMENTS).map(move |id| DropProbe {
        id,
        drops,
        panic_on,
        panicked,
    })
}

fn assert_dropped_once(drops: &[Cell<usize>; ELEMENTS]) {
    assert_eq!(
        drops.each_ref().map(Cell::get),
        [1; ELEMENTS],
        "every element must be dropped exactly once after unwinding"
    );
}

#[derive(Clone, Copy)]
enum Remove {
    All,
    Left,
    Right,
}

impl Remove {
    fn panic_ids(self) -> std::ops::Range<usize> {
        match self {
            Self::All => 0..ELEMENTS,
            Self::Left => 0..4,
            Self::Right => 4..ELEMENTS,
        }
    }
}

#[test]
fn chunk_removal_is_safe_when_any_removed_destructor_panics() {
    for operation in [Remove::All, Remove::Left, Remove::Right] {
        for panic_on in operation.panic_ids() {
            let drops = [const { Cell::new(0) }; ELEMENTS];
            let panicked = Cell::new(false);
            let mut chunk: Chunk<_, U16> = probes(&drops, panic_on, &panicked).collect();
            let result = catch_unwind(AssertUnwindSafe(|| match operation {
                Remove::All => chunk.clear(),
                Remove::Left => chunk.drop_left(4),
                Remove::Right => chunk.drop_right(4),
            }));
            assert!(result.is_err());
            for value in chunk.iter() {
                assert_eq!(drops[value.id].get(), 0, "a dropped element remained live");
            }
            drop(chunk);
            assert_dropped_once(&drops);
        }
    }
}

#[test]
fn inline_array_clear_is_safe_when_any_destructor_panics() {
    for panic_on in 0..ELEMENTS {
        let drops = [const { Cell::new(0) }; ELEMENTS];
        let panicked = Cell::new(false);
        let mut array = InlineArray::<_, [usize; 128]>::new();
        array.extend(probes(&drops, panic_on, &panicked));
        assert!(catch_unwind(AssertUnwindSafe(|| array.clear())).is_err());
        assert!(array.is_empty());
        drop(array);
        assert_dropped_once(&drops);
    }
}

#[test]
fn ring_removal_is_safe_for_contiguous_and_wrapped_ranges() {
    for wrapped in [false, true] {
        for operation in [Remove::All, Remove::Left, Remove::Right] {
            for panic_on in operation.panic_ids() {
                let drops = [const { Cell::new(0) }; ELEMENTS];
                let panicked = Cell::new(false);
                let mut ring = RingBuffer::<_, U16>::new();
                if wrapped {
                    // Advancing an empty ring's origin to 12 makes the final
                    // 8 live elements cross the backing array's boundary.
                    for _ in 0..12 {
                        ring.push_back(DropProbe {
                            id: 0,
                            drops: &drops,
                            panic_on: usize::MAX,
                            panicked: &panicked,
                        });
                        let _ = ring.pop_front();
                    }
                    drops[0].set(0);
                }
                ring.extend(probes(&drops, panic_on, &panicked));
                let result = catch_unwind(AssertUnwindSafe(|| match operation {
                    Remove::All => ring.clear(),
                    Remove::Left => ring.drop_left(4),
                    Remove::Right => ring.drop_right(4),
                }));
                assert!(result.is_err());
                for value in ring.iter() {
                    assert_eq!(drops[value.id].get(), 0, "a dropped element remained live");
                }
                drop(ring);
                assert_dropped_once(&drops);
            }
        }
    }
}

#[test]
fn bounds_panics_do_not_change_live_values() {
    let mut chunk: Chunk<_, U16> = (0..ELEMENTS).collect();
    assert!(catch_unwind(AssertUnwindSafe(|| chunk.drop_left(ELEMENTS + 1))).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| chunk.drop_right(ELEMENTS + 1))).is_err());
    assert_eq!(&chunk[..], &(0..ELEMENTS).collect::<Vec<_>>());

    let mut ring: RingBuffer<_, U16> = (0..ELEMENTS).collect();
    assert!(catch_unwind(AssertUnwindSafe(|| ring.drop_left(ELEMENTS + 1))).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| ring.drop_right(ELEMENTS + 1))).is_err());
    assert_eq!(
        ring.iter().copied().collect::<Vec<_>>(),
        (0..ELEMENTS).collect::<Vec<_>>()
    );
}

// On the unfixed crate this test produces a real double-free; run that
// baseline under Miri only, so the regression runner cannot corrupt its heap.
#[cfg(miri)]
#[test]
fn chunk_clear_does_not_double_free_owned_values() {
    struct OwnedDrop<'a> {
        _allocation: Box<usize>,
        panicked: &'a Cell<bool>,
    }
    impl Drop for OwnedDrop<'_> {
        fn drop(&mut self) {
            if !self.panicked.replace(true) {
                panic!("intentional element destructor panic");
            }
        }
    }
    let panicked = Cell::new(false);
    let mut chunk = Chunk::<_, U16>::new();
    for value in 0..ELEMENTS {
        chunk.push_back(OwnedDrop {
            _allocation: Box::new(value),
            panicked: &panicked,
        });
    }
    assert!(catch_unwind(AssertUnwindSafe(|| chunk.clear())).is_err());
    drop(chunk);
}
