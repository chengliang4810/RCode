# Todo panel

Agent 状态来源、周期提醒和结束检查见 [待办维护](todo-lifecycle.md)。

The agent's checklist is a tray above the composer, not just a `Todo · 2/5 done`
chip in the transcript. It follows the latest checklist of the selected chat and
shows what is being worked on without scrolling.

## Data

Native Agent's `TodoWrite` tool supplies the full checklist on each update.
The event adapter emits `ToolCall::Todo`; the transcript fold and `latest_todo`
projection select the most recent list. An empty list clears the panel.

`TodoItem` is `{ text, done, status? }` (`crates/proto/src/agent.rs`). Completion
uses `done`; `status` carries the in-progress state when present. Missing or
unknown status falls back to `done`. `TodoItem::new` keeps those fields
consistent, and `item.status()` returns the effective state.

## Behaviour

`crates/ui/src/todo_panel.rs`, mounted by the composer above the queue tray (one
step narrower than it, which is itself narrower than the composer).

- **Scope.** Todo parts of the chat's own transcript only. Subagent traffic lives
  in separate docs and never reaches it. Hidden when the chat has no todo.
- **Collapsed.** `Todo · 2/5` plus the in-progress item (else the first unfinished
  one); a finished list reads `Todo · 5/5 · All done`.
- **Expanded.** Every item, in the agent's order, with a status glyph: check
  (success), spinner (in progress and the turn is live; a still ring when idle, so a
  stopped run does not look busy), empty ring (pending). Completed items recede,
  the current item is the brightest.
- **Long lists.** More than 6 items fold to a window of 3 with the current item in
  the middle (one before, one after; clamped at the ends; a finished list shows its
  last three). `N earlier` / `N later` rows at the edges reveal the hidden items in
  place and hide them again. Items are never reordered (`todo_panel::rows`).
- **Open state.** Per chat, in memory for the app run (like the right-pane flags in
  `shell::SessionPanels`; the repo keeps no per-chat UI state on disk). Default:
  open while work remains, compact once everything is done, including when a new
  turn starts on an old finished list. When the last item completes and the turn
  is idle, an explicit choice is reset once so the panel tidies itself to the
  compact done state; opening it by hand afterwards sticks.
- **Done state.** Offers a dismiss button; a dismissal holds until the agent writes
  a different list.
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
the first todo.

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
