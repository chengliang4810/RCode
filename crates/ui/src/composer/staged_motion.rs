//! Presentation-only ownership of staged media. Removed items leave the draft
//! immediately; a short-lived clone keeps their exit visible without entering
//! a send, queue edit, or undo payload a second time.
use std::{collections::HashMap, time::Instant};

use crate::motion::{Presence, PresenceSample};

pub(super) struct StagedView<T> {
    pub item: T,
    pub sample: PresenceSample,
    pub exiting: bool,
    /// Layout reservation, independent of opacity at the moment exit starts.
    pub reservation: f32,
}

struct Retiring<T> {
    scope: String,
    id: String,
    item: T,
    index: usize,
    presence: Presence,
    exit_opacity: f32,
}

pub(super) struct StagedMotion<T> {
    entering: HashMap<(String, String), Presence>,
    retiring: Vec<Retiring<T>>,
    order: HashMap<String, Vec<String>>,
}

impl<T> Default for StagedMotion<T> {
    fn default() -> Self {
        Self {
            entering: HashMap::new(),
            retiring: Vec::new(),
            order: HashMap::new(),
        }
    }
}

impl<T: Clone> StagedMotion<T> {
    pub fn enter(&mut self, scope: &str, id: &str, index: usize, now: Instant, reduced: bool) {
        let key = (scope.to_owned(), id.to_owned());
        // Undo/restaging during exit continues from its current appearance.
        if let Some(at) = self
            .retiring
            .iter()
            .position(|x| x.scope == scope && x.id == id)
        {
            let mut previous = self.retiring.remove(at).presence;
            previous.revive(now, reduced);
            self.entering.insert(key, previous);
        } else {
            self.entering
                .entry(key)
                .or_insert_with(|| Presence::enter(index, now, reduced));
        }
    }

    pub fn retire(
        &mut self,
        scope: &str,
        id: &str,
        item: T,
        index: usize,
        now: Instant,
        reduced: bool,
    ) {
        self.prune(now, reduced);
        // The business list has already removed earlier departures. Locate
        // this item in the last actual view, which still includes those slots.
        let index = self
            .order
            .get(scope)
            .and_then(|ids| ids.iter().position(|x| x == id))
            .unwrap_or_else(|| {
                let mut index = index;
                let mut earlier: Vec<_> = self
                    .retiring
                    .iter()
                    .filter(|x| x.scope == scope && x.id != id)
                    .map(|x| x.index)
                    .collect();
                earlier.sort_unstable();
                for departing in earlier {
                    if departing <= index {
                        index += 1;
                    }
                }
                index
            });
        let mut presence = self
            .entering
            .remove(&(scope.to_owned(), id.to_owned()))
            .unwrap_or_else(|| Presence::present(now));
        presence.exit(now, reduced);
        let exit_sample = presence.sample(now, reduced);
        if !exit_sample.finished {
            self.retiring.retain(|x| x.scope != scope || x.id != id);
            self.retiring.push(Retiring {
                scope: scope.to_owned(),
                id: id.to_owned(),
                item,
                index,
                presence,
                exit_opacity: exit_sample.opacity,
            });
        }
    }

    pub fn prune(&mut self, now: Instant, reduced: bool) {
        self.entering.retain(|_, p| p.sample(now, reduced).active);
        let finished: Vec<_> = self
            .retiring
            .iter()
            .filter(|x| x.presence.sample(now, reduced).finished)
            .map(|x| (x.scope.clone(), x.id.clone(), x.index))
            .collect();
        self.retiring
            .retain(|x| !x.presence.sample(now, reduced).finished);
        for (scope, id, _) in &finished {
            if let Some(order) = self.order.get_mut(scope) {
                order.retain(|x| x != id);
            }
        }
        // A completed departure closes a slot even when no strip was painted
        // between it finishing and the next removal.
        for item in &mut self.retiring {
            if let Some(index) = self
                .order
                .get(&item.scope)
                .and_then(|ids| ids.iter().position(|id| id == &item.id))
            {
                item.index = index;
            } else {
                let closed = finished
                    .iter()
                    .filter(|(scope, _, index)| scope == &item.scope && *index < item.index)
                    .count();
                item.index = item.index.saturating_sub(closed);
            }
        }
    }

    pub fn retiring_count(&self, scope: &str) -> usize {
        self.retiring.iter().filter(|x| x.scope == scope).count()
    }

    pub fn forget(&mut self, scope: &str) {
        self.entering.retain(|(s, _), _| s != scope);
        self.retiring.retain(|x| x.scope != scope);
        self.order.remove(scope);
    }

    pub fn views(
        &mut self,
        scope: &str,
        live: Vec<(String, T)>,
        now: Instant,
        reduced: bool,
    ) -> Vec<StagedView<T>> {
        self.prune(now, reduced);
        // A restored draft has no fresh entrance. In particular, navigation
        // never reconstructs a mount-based animation for existing media.
        for (id, _) in &live {
            if let Some(at) = self
                .retiring
                .iter()
                .position(|x| x.scope == scope && &x.id == id)
            {
                let mut presence = self.retiring.remove(at).presence;
                presence.revive(now, reduced);
                self.entering
                    .insert((scope.to_owned(), id.clone()), presence);
            }
        }
        let mut views: Vec<_> = live
            .into_iter()
            .map(|(id, item)| {
                let sample = self
                    .entering
                    .get(&(scope.to_owned(), id.clone()))
                    .map(|p| p.sample(now, reduced))
                    .unwrap_or_else(|| Presence::present(now).sample(now, reduced));
                (
                    id,
                    StagedView {
                        item,
                        sample,
                        exiting: false,
                        reservation: 1.0,
                    },
                )
            })
            .collect();
        let mut retiring: Vec<_> = self.retiring.iter().filter(|x| x.scope == scope).collect();
        retiring.sort_by_key(|x| x.index);
        for x in retiring {
            views.insert(
                x.index.min(views.len()),
                (
                    x.id.clone(),
                    StagedView {
                        item: x.item.clone(),
                        sample: x.presence.sample(now, reduced),
                        exiting: true,
                        reservation: (x.presence.sample(now, reduced).opacity
                            / x.exit_opacity.max(f32::EPSILON))
                        .clamp(0.0, 1.0),
                    },
                ),
            );
        }
        self.order.insert(
            scope.to_owned(),
            views.iter().map(|(id, _)| id.clone()).collect(),
        );
        views.into_iter().map(|(_, view)| view).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        Composer, STRIP_GAP, STRIP_PAD_TOP, STRIP_PAD_X, STRIP_THUMB, StagedAttachment,
        attachment_strip_columns, attachment_strip_height, attachment_strip_reserved_height,
        attachment_strip_slot_origin,
    };
    use super::*;
    use crate::{attachments::AttachmentContent, state::AppState};
    use gpui::{AppContext, Entity, TestAppContext};
    use std::sync::Arc;
    use std::time::Duration;

    fn file(id: &str, bytes: Arc<Vec<u8>>) -> StagedAttachment {
        StagedAttachment {
            id: id.into(),
            name: format!("{id}.txt"),
            content: AttachmentContent::File(bytes),
            mention: None,
        }
    }

    fn composer_without_render(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Composer>) {
        let (dir, _window) = super::super::tests::composer_focus_window(cx);
        cx.update(|cx| crate::motion::set_reduced_motion(cx, false));
        let state = cx.new(|_| AppState::new());
        let composer = cx.new(|cx| Composer::new(state, cx));
        (dir, composer)
    }

    fn assert_live_tiles_fit<T>(views: &[StagedView<T>], width: f32) {
        let height = attachment_strip_reserved_height(
            views.iter().map(|view| view.reservation).sum(),
            width,
        );
        let mut slot = 0.0;
        for view in views {
            let origin = attachment_strip_slot_origin(slot, width);
            if !view.exiting {
                assert!(
                    origin.1 + STRIP_THUMB <= height + 0.001,
                    "live tile at slot {slot} ends at {}, outside strip {height}",
                    origin.1 + STRIP_THUMB
                );
                assert!(origin.0 >= STRIP_PAD_X - 0.001);
                assert!(origin.0 + STRIP_THUMB <= width - STRIP_PAD_X + 0.001);
            }
            slot += view.reservation;
        }
    }

    #[test]
    fn deletion_keeps_only_a_temporary_view_and_never_crosses_drafts() {
        let now = Instant::now();
        let mut state = StagedMotion::default();
        state.retire("a", "file", "removed", 0, now, false);
        assert_eq!(state.retiring_count("a"), 1);
        assert!(state.views("b", Vec::new(), now, false).is_empty());
        let views = state.views("a", vec![("next".into(), "live")], now, false);
        assert_eq!(views.len(), 2);
        assert!(views[0].exiting);
        assert_eq!(views[1].item, "live");
        state.prune(now + Duration::from_secs(30), false);
        assert_eq!(state.retiring_count("a"), 0);
    }

    #[test]
    fn navigation_does_not_replay_and_undo_does_not_duplicate_a_view() {
        let now = Instant::now();
        let mut state = StagedMotion::default();
        let views = state.views("a", vec![("file".into(), 1)], now, false);
        assert!(!views[0].sample.active);
        state.retire("a", "file", 1, 0, now, false);
        let views = state.views(
            "a",
            vec![("file".into(), 1)],
            now + Duration::from_millis(20),
            false,
        );
        assert_eq!(views.len(), 1);
        assert!(!views[0].exiting);
        assert_eq!(state.retiring_count("a"), 0);
    }

    #[test]
    fn reduced_motion_releases_removed_resources_immediately() {
        let now = Instant::now();
        let mut state = StagedMotion::default();
        state.enter("a", "file", 0, now, true);
        state.retire("a", "file", vec![0u8; 8], 0, now, true);
        assert_eq!(state.retiring_count("a"), 0);
        state.forget("a");
        assert!(state.entering.is_empty());
    }

    #[test]
    fn adjacent_removals_keep_the_last_visible_order() {
        let now = Instant::now();
        let mut state = StagedMotion::default();
        state.views(
            "a",
            ["A", "B", "C", "D"].map(|id| (id.into(), id)).to_vec(),
            now,
            false,
        );
        state.retire("a", "B", "B", 1, now, false);
        // B already left the business list, so C's business index is now 1.
        state.retire("a", "C", "C", 1, now, false);
        let views = state.views("a", vec![("A".into(), "A"), ("D".into(), "D")], now, false);
        assert_eq!(
            views.iter().map(|view| view.item).collect::<Vec<_>>(),
            ["A", "B", "C", "D"]
        );
        assert_eq!(
            views.iter().map(|view| view.exiting).collect::<Vec<_>>(),
            [false, true, true, false]
        );
    }

    #[test]
    fn adjacent_removals_without_a_painted_order_still_reserve_each_departure() {
        let now = Instant::now();
        let mut state = StagedMotion::default();
        state.retire("a", "B", "B", 1, now, false);
        state.retire("a", "C", "C", 1, now, false);
        let views = state.views("a", vec![("A".into(), "A"), ("D".into(), "D")], now, false);
        assert_eq!(
            views.iter().map(|view| view.item).collect::<Vec<_>>(),
            ["A", "B", "C", "D"]
        );
    }

    #[test]
    fn finished_departures_close_their_slots_before_the_next_view() {
        let now = Instant::now();
        let duration = crate::motion::ATTACHMENT_OUT
            .total()
            .mul_f32(crate::motion::speed_scale());
        let mut state = StagedMotion::default();
        state.views(
            "a",
            ["A", "B", "C", "D"].map(|id| (id.into(), id)).to_vec(),
            now,
            false,
        );
        state.retire("a", "B", "B", 1, now, false);
        state.retire("a", "C", "C", 1, now + duration / 2, false);
        let views = state.views(
            "a",
            vec![("A".into(), "A"), ("D".into(), "D")],
            now + duration + duration / 4,
            false,
        );
        assert_eq!(
            views.iter().map(|view| view.item).collect::<Vec<_>>(),
            ["A", "C", "D"]
        );
        assert!(views[1].exiting);
    }

    #[test]
    fn integer_strip_geometry_matches_the_original_wrap_layout() {
        for columns in [1, 3, 5] {
            let width =
                2.0 * STRIP_PAD_X + columns as f32 * STRIP_THUMB + (columns - 1) as f32 * STRIP_GAP;
            assert_eq!(attachment_strip_columns(width), columns);
            for count in 0..=12 {
                assert_eq!(
                    attachment_strip_reserved_height(count as f32, width),
                    attachment_strip_height(count, width)
                );
                for slot in 0..count {
                    assert_eq!(
                        attachment_strip_slot_origin(slot as f32, width),
                        (
                            STRIP_PAD_X + (slot % columns) as f32 * (STRIP_THUMB + STRIP_GAP),
                            STRIP_PAD_TOP + (slot / columns) as f32 * (STRIP_THUMB + STRIP_GAP),
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn adjacent_exits_move_live_rows_inside_the_same_reserved_height() {
        let now = Instant::now();
        let duration = crate::motion::ATTACHMENT_OUT
            .total()
            .mul_f32(crate::motion::speed_scale());
        for columns in [1, 3] {
            let width =
                2.0 * STRIP_PAD_X + columns as f32 * STRIP_THUMB + (columns - 1) as f32 * STRIP_GAP;
            let mut state = StagedMotion::default();
            state.views(
                "a",
                ["A", "B", "C", "D", "E"].map(|id| (id.into(), id)).to_vec(),
                now,
                false,
            );
            state.retire("a", "B", "B", 1, now, false);
            state.retire("a", "C", "C", 1, now, false);
            let mut previous_height = f32::MAX;
            for progress in [0.0, 0.125, 0.5, 0.875, 0.99, 1.0] {
                let views = state.views(
                    "a",
                    ["A", "D", "E"].map(|id| (id.into(), id)).to_vec(),
                    now + duration.mul_f32(progress),
                    false,
                );
                assert_live_tiles_fit(&views, width);
                let height = attachment_strip_reserved_height(
                    views.iter().map(|view| view.reservation).sum(),
                    width,
                );
                assert!(height <= previous_height + 0.001);
                previous_height = height;
            }
            assert_eq!(previous_height, attachment_strip_height(3, width));
        }
    }

    #[test]
    fn adding_and_undo_during_exit_keep_live_tiles_inside_the_strip() {
        let now = Instant::now();
        let duration = crate::motion::ATTACHMENT_OUT
            .total()
            .mul_f32(crate::motion::speed_scale());
        for columns in [1, 3] {
            let width =
                2.0 * STRIP_PAD_X + columns as f32 * STRIP_THUMB + (columns - 1) as f32 * STRIP_GAP;
            let mut state = StagedMotion::default();
            state.views(
                "a",
                ["A", "B", "C", "D"].map(|id| (id.into(), id)).to_vec(),
                now,
                false,
            );
            state.retire("a", "B", "B", 1, now, false);
            let halfway = now + duration / 2;
            state.enter("a", "E", 4, halfway, false);
            let views = state.views(
                "a",
                ["A", "C", "D", "E"].map(|id| (id.into(), id)).to_vec(),
                halfway,
                false,
            );
            assert_live_tiles_fit(&views, width);
            assert_eq!(
                views
                    .iter()
                    .find(|view| view.item == "E")
                    .unwrap()
                    .reservation,
                1.0
            );
            let views = state.views(
                "a",
                ["A", "B", "C", "D", "E"].map(|id| (id.into(), id)).to_vec(),
                halfway,
                false,
            );
            assert_live_tiles_fit(&views, width);
            assert_eq!(views.len(), 5);
            assert!(views.iter().all(|view| !view.exiting));
            assert_eq!(state.retiring_count("a"), 0);
        }
    }

    #[test]
    fn exit_reservation_starts_full_even_when_entrance_is_partly_visible() {
        let now = Instant::now();
        let mut state = StagedMotion::default();
        state.enter("a", "B", 1, now, false);
        let entering = Presence::enter(1, now, false);
        let removed_at = now + entering.duration() * 3 / 4;
        state.retire("a", "B", "B", 0, removed_at, false);
        let views = state.views("a", Vec::new(), removed_at, false);
        assert_eq!(views.len(), 1);
        assert!(views[0].sample.opacity > 0.0 && views[0].sample.opacity < 1.0);
        assert_eq!(views[0].reservation, 1.0);
        let finished = removed_at
            + crate::motion::ATTACHMENT_OUT
                .total()
                .mul_f32(crate::motion::speed_scale());
        let views = state.views("a", Vec::new(), finished, false);
        assert!(views.is_empty());
        assert_eq!(attachment_strip_reserved_height(0.0, 216.0), 0.0);
    }

    #[gpui::test]
    fn removal_updates_the_business_draft_and_releases_bytes_without_rendering(
        cx: &mut TestAppContext,
    ) {
        let (_dir, composer) = composer_without_render(cx);
        let bytes = composer.update(cx, |composer, cx| {
            let bytes = Arc::new(vec![7u8; 1024]);
            let weak = Arc::downgrade(&bytes);
            composer
                .attachments
                .insert(composer.current_key.clone(), vec![file("removed", bytes)]);
            composer.remove_attachment("removed", cx);
            assert!(
                composer.staged().is_empty(),
                "exit views are never part of a send"
            );
            assert_eq!(
                composer
                    .attachment_motion
                    .retiring_count(&composer.current_key),
                1
            );
            assert!(composer.staged_motion_cleanup.is_some());
            weak
        });
        assert!(
            bytes.upgrade().is_some(),
            "only the exit view keeps the bytes alive"
        );
        cx.run_until_parked();
        cx.executor().advance_clock(
            crate::motion::ATTACHMENT_OUT
                .total()
                .mul_f32(crate::motion::speed_scale())
                + Duration::from_millis(1),
        );
        cx.run_until_parked();
        assert!(
            bytes.upgrade().is_none(),
            "cleanup must not depend on another draw"
        );
        composer.read_with(cx, |composer, _| {
            assert_eq!(
                composer
                    .attachment_motion
                    .retiring_count(&composer.current_key),
                0
            );
            assert!(composer.staged_motion_cleanup.is_none());
        });
    }

    #[gpui::test]
    fn ordinary_and_queue_edit_drafts_do_not_share_departing_appshots(cx: &mut TestAppContext) {
        let (_dir, composer) = composer_without_render(cx);
        composer.update(cx, |composer, cx| {
            let mut ordinary = crate::appshots::tests::shot();
            ordinary.id = "ordinary".into();
            let mut removed = ordinary.clone();
            removed.id = "removed-ordinary".into();
            composer
                .appshots
                .insert(composer.current_key.clone(), vec![ordinary, removed]);
            composer.remove_appshot("removed-ordinary", cx);
            assert_eq!(
                composer
                    .appshot_motion
                    .retiring_count(&composer.current_key),
                1
            );
            let mut queued = crate::appshots::tests::shot();
            queued.id = "queued".into();
            composer.swap_in_queued_draft("queued text".into(), vec![], vec![queued], cx);
            assert_eq!(
                composer
                    .appshot_motion
                    .retiring_count(&composer.current_key),
                0
            );
            assert!(composer.staged_motion_cleanup.is_none());
            composer.remove_appshot("queued", cx);
            assert!(composer.staged_appshots().is_empty());
            assert_eq!(
                composer
                    .appshot_motion
                    .retiring_count(&composer.current_key),
                1
            );
            composer.clear_queue_edit_local(cx);
            assert_eq!(composer.staged_appshots()[0].id, "ordinary");
            assert_eq!(
                composer
                    .appshot_motion
                    .retiring_count(&composer.current_key),
                0
            );
            assert!(composer.staged_motion_cleanup.is_none());
            let live = composer
                .staged_appshots()
                .iter()
                .map(|item| (item.id.clone(), item.clone()))
                .collect();
            let views =
                composer
                    .appshot_motion
                    .views(&composer.current_key, live, Instant::now(), false);
            assert_eq!(views.len(), 1);
            assert!(!views[0].exiting);
            assert!(
                !views[0].sample.active,
                "restoring the ordinary draft must not replay entrance"
            );
        });
    }

    #[gpui::test]
    fn navigation_forgets_exit_views_and_does_not_replay_returning_media(cx: &mut TestAppContext) {
        let (_dir, composer) = composer_without_render(cx);
        composer.update(cx, |composer, cx| {
            let key = composer.current_key.clone();
            composer.attachments.insert(
                key.clone(),
                vec![
                    file("kept", Arc::new(vec![1])),
                    file("removed", Arc::new(vec![2])),
                ],
            );
            composer
                .attachment_motion
                .enter(&key, "kept", 0, Instant::now(), false);
            composer.remove_attachment("removed", cx);
            assert_eq!(composer.attachment_motion.retiring_count(&key), 1);
            composer
                .state
                .update(cx, |state, _| state.selected_chat = Some("other".into()));
            composer.on_state_changed(cx);
            assert_eq!(composer.attachment_motion.retiring_count(&key), 0);
            assert!(composer.staged_motion_cleanup.is_none());
            composer
                .state
                .update(cx, |state, _| state.selected_chat = None);
            composer.on_state_changed(cx);
            let live = composer
                .staged()
                .iter()
                .map(|item| (item.id.clone(), item.clone()))
                .collect();
            let views = composer
                .attachment_motion
                .views(&key, live, Instant::now(), false);
            assert_eq!(views.len(), 1);
            assert!(!views[0].sample.active);
        });
    }

    struct StripTestRoot {
        composer: Entity<Composer>,
        width: f32,
    }

    impl gpui::Render for StripTestRoot {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::prelude::*;
            self.composer.update(cx, |composer, cx| {
                composer.set_available_width(self.width, cx)
            });
            gpui::div()
                .w(gpui::px(self.width))
                .child(self.composer.clone())
        }
    }

    #[gpui::test]
    fn real_strip_keeps_live_tiles_inside_after_consecutive_business_removals(
        cx: &mut TestAppContext,
    ) {
        let (_dir, _) = super::super::tests::composer_focus_window(cx);
        cx.update(|cx| crate::motion::set_reduced_motion(cx, false));
        for width in [122.0, 250.0] {
            let (root, visual) = cx.add_window_view(|_, cx| {
                let state = cx.new(|_| AppState::new());
                let composer = cx.new(|cx| Composer::new(state, cx));
                composer.update(cx, |composer, cx| {
                    composer.fixture_reset_feedback_attachments(cx);
                    composer.focus_pending = false;
                });
                StripTestRoot { composer, width }
            });
            visual.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(1400.0)));
            visual.update(|window, cx| window.draw(cx).clear());
            visual.run_until_parked();
            let composer = root.read_with(visual, |root, _| root.composer.clone());
            composer.update(visual, |composer, cx| {
                composer.fixture_remove_feedback_attachment('B', cx);
                composer.fixture_remove_feedback_attachment('C', cx);
                assert_eq!(composer.fixture_feedback_attachment_count(), 5);
                assert_eq!(
                    composer
                        .staged()
                        .iter()
                        .map(|item| item.id.as_str())
                        .collect::<Vec<_>>(),
                    [
                        "fixture-A",
                        "fixture-D",
                        "fixture-E",
                        "fixture-F",
                        "fixture-G"
                    ]
                );
            });
            for settled in [false, true] {
                if settled {
                    visual.executor().advance_clock(
                        crate::motion::ATTACHMENT_OUT
                            .total()
                            .mul_f32(crate::motion::speed_scale())
                            + Duration::from_millis(1),
                    );
                    visual.run_until_parked();
                }
                visual.update(|window, cx| window.draw(cx).clear());
                let strip = visual
                    .debug_bounds("composer-attachment-strip")
                    .expect("real strip mounts");
                for selector in [
                    "composer-att-thumb-fixture-A",
                    "composer-att-thumb-fixture-D",
                    "composer-att-thumb-fixture-E",
                    "composer-att-thumb-fixture-F",
                    "composer-att-thumb-fixture-G",
                ] {
                    let tile = visual
                        .debug_bounds(selector)
                        .expect("remaining live tile mounts");
                    assert!(
                        tile.bottom() <= strip.bottom() + gpui::px(0.1),
                        "{selector} clipped at width {width}"
                    );
                }
            }
            composer.read_with(visual, |composer, _| {
                assert!(composer.staged_motion_cleanup.is_none());
                assert_eq!(
                    composer
                        .attachment_motion
                        .retiring_count(&composer.current_key),
                    0
                );
            });
        }
    }

    #[gpui::test]
    fn real_queue_pending_states_keep_rows_without_ack_or_transport(cx: &mut TestAppContext) {
        let (_dir, composer) = composer_without_render(cx);
        composer.update(cx, |composer, cx| {
            composer.state.update(cx, |state, _| {
                state.selected_chat = Some("fixture-chat".into());
                state.queue = vec![
                    rcode_doc::QueuedMessage::new("fixture-edit", "edited 中文", "fixture-device"),
                    rcode_doc::QueuedMessage::new(
                        "fixture-remove",
                        "removed English",
                        "fixture-device",
                    ),
                ];
            });
            let before = composer.state.read(cx).queue.clone();
            composer.fixture_feedback_queue_state(1, cx);
            composer.begin_queue_edit("fixture-edit".into(), cx);
            assert_eq!(
                composer.queue_edit_pending_id.as_deref(),
                Some("fixture-edit")
            );
            assert!(composer.queue_edit_task.is_none());
            composer.fixture_feedback_queue_state(2, cx);
            composer.remove_queued("fixture-remove".into(), cx);
            assert!(composer.queue_removing.contains("fixture-remove"));
            composer.fixture_feedback_queue_state(3, cx);
            // The action is consumed, while finish_queue_edit rejects a
            // second commit until the first one is acknowledged.
            assert!(composer.commit_queue_edit(cx));
            assert!(composer.queue_edit_finishing);
            assert_eq!(composer.editing_queued.as_deref(), Some("fixture-edit"));
            assert_eq!(composer.state.read(cx).queue, before);
            assert!(composer.state.read(cx).engine().is_none());
            assert!(composer.queue_edit_task.is_none());
            assert!(composer.send_task.is_none());
            composer.fixture_feedback_queue_state(0, cx);
            assert!(composer.queue_edit_pending_id.is_none());
            assert!(composer.queue_removing.is_empty());
            assert!(composer.editing_queued.is_none());
            assert!(!composer.queue_edit_finishing);
        });
    }
}
