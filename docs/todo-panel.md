# Todo panel

Agent 状态来源、周期提醒和结束检查见 [待办维护](todo-lifecycle.md)。

The agent's checklist is a tray above the composer, not just a `Todo · 2/5 done`
chip in the transcript. It follows the latest checklist of the selected chat and
shows what is being worked on without scrolling.

## Data

Native Agent's `TodoWrite` tool supplies the full checklist on each update.
The event adapter emits `ToolCall::Todo`; the transcript fold and `latest_todo_write`
projection select the most recent list and retain its message/tool identity.
Empty and completed writes remain lifecycle boundaries even while the tray is hidden.

`TodoItem` is `{ text, done, status? }` (`crates/proto/src/agent.rs`). Completion
uses `done`; `status` carries the in-progress state when present. Missing or
unknown status falls back to `done`. `TodoItem::new` keeps those fields
consistent, and `item.status()` returns the effective state.

## Behaviour

`crates/ui/src/todo_panel.rs`, mounted in the composer's attached surface stack.
When a goal exists, the todo tray attaches to its top edge; otherwise it attaches
directly to the input. Each layer steps inward by 12px and clips its lower 12px,
sharing the goal header's rounded top, border and background. Hidden strips do
not paint through the next translucent surface. The queue remains above this
stack, so it cannot separate a todo tray from its goal.

- **Scope.** Todo parts of the chat's own transcript only. Subagent traffic lives
  in separate docs and never reaches it. Hidden when the chat has no todo.
- **Collapsed by default.** A new checklist starts with the single-line header:
  `Todo · 2/5` plus the in-progress item (else the first unfinished one). Click
  the header to expand it.
- **Expanded.** Every item, in the agent's order, with a status glyph: check
  (success), spinner (in progress and the turn is live; a still ring when idle, so a
  stopped run does not look busy), empty ring (pending). Completed items recede,
  the current item is the brightest.
- **Long lists.** More than 6 items fold to a window of 3 with the current item in
  the middle (one before, one after; clamped at the ends; a finished list shows its
  last three). `N earlier` / `N later` rows at the edges reveal the hidden items in
  place and hide them again. Items are never reordered (`todo_panel::rows`).
- **Open state.** Per chat, in memory for the app run (like the right-pane flags in
  `shell::SessionPanels`; the repo keeps no per-chat UI state on disk). A manual
  expansion survives progress updates and chat navigation. A checklist created
  after dismissal, completion or clearing starts collapsed again.
- **Removal.** An idle checklist offers a dismiss button. Completing every item
  or writing an empty list hides it automatically. The panel retains the last
  accepted write and its dismissal/completion state, independently of the loaded
  history window. Historical replay cannot undo that state or expose an older
  checklist in its place. If the initial transcript tail has no checklist,
  loading older history does not create one. A genuinely later tool invocation
  can show new work, even when its text matches a dismissed checklist. Live plan
  updates that reuse a tool ID can start new work after completion as well.
- **No focus stealing.** Nothing here takes focus; updates only re-render.
- **Accessibility.** The header, dismiss and fold rows are buttons with
  `aria_label`, tab stops and a focus ring, activated by Enter/Space. The icon-only
  dismiss button has a tooltip, and the header's tooltip names the action it will
  take.
- **Motion.** The list fades in on toggle (`FADE_QUICK`) and the tray fades in on
  mount. Those, and the shared mini glyph spinner, all follow the global
  reduce-motion flag, which also covers the pause-in-background setting.

The transcript chip keeps working: `Todo · 2/5 done`, and its expanded detail marks
the current item `[~]` (`[x]` done, `[ ]` pending).

## Performance

The composer re-renders far more often than the transcript changes, so
`TodoCache` rescans only when the selected chat, `transcript_revision` or the
transcript length changes. The scan walks back from the newest entry and stops at
the first todo. Per-chat presentation state retains only the current checklist;
a weak reference to the existing transcript replay baseline distinguishes a
historical refresh from an ordinary live update without retaining the full
history. Repeated renders of the same candidate do not reapply a rejected replay.

## Testing

```sh
cargo test --locked -p rcode-proto todo_item_tests
cargo test --locked -p rcode-harness --lib      # native tool-event mapping
cargo test --locked -p rcode-doc todo
cargo test --locked -p rcode-ui --lib todo_panel
```

Live: `RCODE_HARNESS=mock RCODE_MOCK_TODO=1 RCODE_MOCK_DELAY_MS=900` (see
`CONTRIBUTORS.md`) walks an 8-item list through every state.

## Not done

- Mobile (iOS/Android) panel: a later PR in the stack.
- The `[x]` / `[ ]` rendering in MCP `read_chat` and the mobile transcript does not
  yet distinguish in-progress.
