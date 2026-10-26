#[cfg(test)]
mod tests {
    use std::sync::{Arc, Weak};
    use taffy::{geometry::Size, prelude::TaffyMaxContent, style::Style, TaffyTree};

    fn context(tree: &mut TaffyTree<Arc<Vec<u8>>>) -> (taffy::NodeId, Weak<Vec<u8>>) {
        let payload = Arc::new(vec![0x5a; 65_536]);
        let owner = Arc::downgrade(&payload);
        let node = tree
            .new_leaf_with_context(Style::default(), payload)
            .unwrap();
        (node, owner)
    }

    #[test]
    fn clear_releases_all_node_contexts_immediately() {
        let mut tree = TaffyTree::new();
        let owners: Vec<_> = (0..64).map(|_| context(&mut tree).1).collect();
        assert!(owners.iter().all(|owner| owner.strong_count() == 1));
        tree.clear();
        assert_eq!(tree.total_node_count(), 0);
        assert!(owners.iter().all(|owner| owner.upgrade().is_none()));
    }

    #[test]
    fn remove_releases_only_the_removed_nodes_context() {
        let mut tree = TaffyTree::new();
        let (removed, removed_owner) = context(&mut tree);
        let (retained, retained_owner) = context(&mut tree);
        tree.remove(removed).unwrap();
        assert!(removed_owner.upgrade().is_none());
        assert_eq!(retained_owner.strong_count(), 1);
        assert!(tree.get_node_context(removed).is_none());
        assert!(tree.get_node_context(retained).is_some());
        tree.clear();
        assert!(retained_owner.upgrade().is_none());
    }

    #[test]
    fn clear_does_not_keep_contexts_at_unreused_frame_slots() {
        let mut tree = TaffyTree::new();
        let owners: Vec<_> = (0..500).map(|_| context(&mut tree).1).collect();
        tree.clear();
        for _frame in 0..20 {
            for _node in 0..3 {
                tree.new_leaf(Style::default()).unwrap();
            }
            tree.clear();
        }
        assert!(owners.iter().all(|owner| owner.upgrade().is_none()));
    }

    #[test]
    fn removing_parent_keeps_detached_child_contexts_alive() {
        let mut tree = TaffyTree::new();
        let (child, child_owner) = context(&mut tree);
        let (parent, parent_owner) = context(&mut tree);
        tree.add_child(parent, child).unwrap();
        tree.remove(parent).unwrap();
        assert!(parent_owner.upgrade().is_none());
        assert_eq!(child_owner.strong_count(), 1);
        assert!(tree.get_node_context(child).is_some());
        assert!(tree.parent(child).is_none());
        tree.clear();
        assert!(child_owner.upgrade().is_none());
    }

    #[test]
    fn measurement_still_uses_live_context_after_clear_and_remove() {
        let mut tree = TaffyTree::new();
        let discarded = tree.new_leaf_with_context(Style::default(), 17.0).unwrap();
        tree.remove(discarded).unwrap();
        tree.clear();
        let measured = tree.new_leaf_with_context(Style::default(), 42.0).unwrap();
        tree.compute_layout_with_measure(
            measured,
            Size::MAX_CONTENT,
            |_known, _available, _node, context, _style| Size {
                width: *context.unwrap(),
                height: 13.0,
            },
        )
        .unwrap();
        assert_eq!(
            tree.layout(measured).unwrap().size,
            Size {
                width: 42.0,
                height: 13.0
            }
        );
        assert_eq!(tree.get_node_context(measured), Some(&42.0));
    }
}
