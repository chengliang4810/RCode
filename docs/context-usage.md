# Context usage

Selected conversations show a context ring beneath the composer, including
project-less and remote conversations. Hovering opens the anchored details;
clicking does not pin them. The card shows a single row with capacity and used/total counts
plus a percentage, followed by a progress bar. Chinese counts use compact units,
for example `13.4万/100万 (13.4%)`. It updates with the replicated context snapshot. The ring appears after recorded context data is available.
The new-conversation page shows no ring. Moving away from both the ring and card
closes the panel, with a short grace period for crossing their eight-pixel gap.
The panel uses the same backdrop blur and trigger-based positioning as session statistics.
Context occupancy is separate from [session token billing](session-statistics.md).
The ring and progress bar fill by the same used/capacity fraction; remaining
tokens are omitted. The ring turns amber at 75% and red at 90%; its drawing clamps
at a full circle while the details preserve over-capacity measurements. A faint
track means the harness has not reported enough data to calculate a percentage.
Measured zero is displayed as 0.0% in the details.

Native Agent reports context occupancy separately from billing `Usage` events.
The runner takes the latest model call's normalized input-token usage; model
adapters include cached input in that value. Capacity comes from the selected
model configuration and its context-window options. A conversation without a
measurement does not display a context ring; the new-conversation page has none.
Mock conversations need an explicit context snapshot to display a measurement.

`AgentEvent::ContextUsage` updates one atomic `meta.contextUsage` value in the
session document. Partial measurements preserve known fields; a zero capacity is
ignored. New non-resumed runs clear old usage. Post-turn updates can refresh the
snapshot without reopening a completed turn, and subagent events cannot change
the parent meter.

The existing document sync carries the value through the relay and local storage.
`WatchDocMessages` includes a typed `TranscriptUpdate` envelope with the snapshot,
including context-only commits and the opening reset. Old readers ignore the
additive field; old hosts decode as unavailable. Thin-document rebuilding also
preserves the snapshot. UI rendering reads the in-memory state, with no polling,
provider calls, filesystem access, or new remote endpoint.

Validation covers model usage normalization, compaction to zero, cache accounting,
subagent isolation, snapshot and incremental import, thin rebuilds, context-only
watch updates, reconnects, and old/new watch compatibility. Native acceptance should check measured usage, compaction, chat switching and
hover/focus behavior on each supported platform. Deterministic test counters do
not establish accuracy for every live provider.
