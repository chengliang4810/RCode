use super::*;
use crate::memory::{MemoryEvent, MemoryPanel};

impl Shell {
    pub(super) fn memory_available(&self, cx: &App) -> bool {
        self.memories
            .get(&self.panel_key(cx))
            .is_some_and(|memory| memory.read(cx).enabled())
    }

    pub(super) fn ensure_memory_panel(&mut self, window: &Window, cx: &mut Context<Self>) {
        let key = self.panel_key(cx);
        if key.is_empty() || self.memories.contains_key(&key) {
            return;
        }
        let memory = cx.new(|cx| MemoryPanel::new(self.state.clone(), key.clone(), cx));
        let owner = key.clone();
        let subscription =
            cx.subscribe_in(
                &memory,
                window,
                move |this, _, event, window, cx| match event {
                    MemoryEvent::OpenFile(path) if this.panel_key(cx) == owner => {
                        this.add_file_surface(path.clone(), window, cx);
                    }
                    _ => cx.notify(),
                },
            );
        self.memories.insert(key.clone(), memory);
        self.memory_subs.insert(key, subscription);
    }

    pub(super) fn add_memory_surface(&mut self, cx: &mut Context<Self>) {
        if !self.memory_available(cx) {
            return;
        }
        let key = self.panel_key(cx);
        self.set_surfaces_open(true, cx);
        push_unique_right_surface(
            self.right_tabs.entry(key).or_default(),
            RightSurface::Memory,
        );
        self.set_right_active(RightSurface::Memory, cx);
    }
}
