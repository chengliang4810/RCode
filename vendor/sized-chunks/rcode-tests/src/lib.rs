#[cfg(test)]
mod panic_safety;

#[cfg(test)]
mod aliasing {
    use im::OrdSet;
    use sized_chunks::Chunk;
    use std::collections::BTreeSet;
    use typenum::U16;

    // Original safe-code reproducer from im-rs#207. Under Miri the upstream
    // force_copy implementation reports an invalidated read pointer.
    #[test]
    fn ordset_original_aliasing_reproducer() {
        let set: OrdSet<String> = (0..11).map(|value| value.to_string()).collect();
        let expected: BTreeSet<_> = (0..11).map(|value| value.to_string()).collect();
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            expected.iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn chunk_moves_overlapping_owned_values_in_both_directions() {
        let mut chunk: Chunk<String, U16> = (0..12).map(|value| value.to_string()).collect();
        for _ in 0..4 {
            chunk.pop_front();
        }
        for value in 12..17 {
            chunk.push_back(value.to_string());
        }
        // Moving the live interval towards the front overlaps its source.
        assert_eq!(
            &chunk[..],
            &(4..17).map(|value| value.to_string()).collect::<Vec<_>>()
        );
        while chunk.len() > 8 {
            chunk.pop_back();
        }
        // Moving back towards the end also overlaps, this time in reverse.
        chunk.push_front("3".into());
        assert_eq!(
            &chunk[..],
            &(3..12).map(|value| value.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ordset_clones_preserve_previous_values_during_insert_and_remove() {
        let original: OrdSet<String> = (0..128).map(|value| format!("{value:03}")).collect();
        let mut edited = original.clone();
        for value in (0..128).step_by(2) {
            edited.remove(&format!("{value:03}"));
        }
        for value in 128..192 {
            edited.insert(format!("{value:03}"));
        }
        assert_eq!(original.len(), 128);
        assert_eq!(edited.len(), 128);
        assert!(original.contains("000"));
        assert!(!edited.contains("000"));
        assert!(!original.contains("191"));
        assert!(edited.contains("191"));
    }
}
