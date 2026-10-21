# Goal and checklist layout QA

final result: passed

## Source and scope

- User image 1: `output/goal-todo-reference-before.png` (blank checklist labels and overlapping goal).
- User image 2, visual target: `output/goal-todo-reference-target.png`, 757 x 81 pixels. Target is the inset goal header attached directly to the input's rounded upper edge.
- Existing native Windows GPUI product, production Shell and Composer. This is a source repair, not a web prototype; browser hosting/deployment does not apply.

## Final evidence

- Combined source/implementation comparison: `output/goal-todo-verified/reference-comparison.png`.
- Full component context with checklist and paused goal: `output/goal-todo-verified/goal-paused.png`.
- Matching target state without checklist: `output/goal-todo-verified/goal-only-paused.png` (same sample objective as image 2).
- Narrow layout: `output/goal-todo-verified/goal-narrow-390.png`.
- Dark theme: `output/goal-todo-verified/goal-todo-dark.png`.
- Active and completed states: `goal-active.png`, `goal-complete.png` in the same directory; goal clear: `goal-cleared.png`.
- Focused before/after surface: `output/goal-todo-verified/todo-goal-composer-detail.png`.
- Capture metadata: native viewport and pixels 760 x 760, 96 DPI / density 1; narrow viewport and pixels 390 x 850. Source density was not supplied. For the focused comparison only, the native input-width crop (728 pixels) is normalized to the source width (757 pixels); source is left unchanged. The source is already a component crop, so no full-product pixel-equivalence claim is made.

## Comparison history and fixes

1. User evidence, P1: TodoWrite supplies `text`, but the native event decoder read only `content`. Fixed by preferring nonblank `text` and accepting legacy `content`. Native and legacy decoder tests pass.
2. User evidence, P1: old negative tray margins intersect the new goal header. Fixed by laying out checklist and queued messages separately above an attached goal/input stack; preserve old tray treatment when no goal exists. Wide/narrow bounds tests confirm separation.
3. First native capture, P1: goal/input negative-margin nesting undercounts height in the complete Shell; the input is clipped below the viewport. Explicit nonshrinking nodes alone were insufficient. Final fix: reserve 26 pixels as stack padding and position the 38-pixel goal header inside that reservation, leaving its lower 12 pixels behind the input. Native input bounds are x16,y671,w728,h49 inside the 760-pixel viewport; full input, attachment, model and send controls are now visible.
4. Native visual comparison, P2: icon-only goal actions were invisible because GPUI SVG color must be set on the SVG itself. Fixed with explicit existing theme color on each icon. Final light, dark and narrow captures show trash and pause/resume actions.
5. Historical data: exact matching journal results repair only blank labels, preserve statuses and known labels, require matching item counts and stop on truncated/unrelated output. A real engine shutdown/reopen test confirms durable recovery; repeated repair is idempotent.

## Required fidelity surfaces

- Fonts/typography: existing product UI font and size, medium status label, normal objective text. Long objectives truncate on one line with their full text and iteration available in the tooltip.
- Spacing/layout: inset rounded header meets the input directly. Checklist rows and folds have their own space above it. All goal actions fit above the input edge at 820/390 test widths; all persistent input controls remain inside native captures.
- Colors/tokens: retain the existing light/dark glass and border tokens. Native inactive-window glass is grayer than the reference app; this is deliberate product-theme reuse, not an unrelated palette redesign.
- Images/assets: no raster assets are needed. Existing goal, trash and play/stop icons use explicit SVG colors and accessible labels.
- Copy/content: actual task names are visible; no descriptive or promotional copy was added. Elapsed time and the rightmost edit entry are included in the follow-up described below. Existing pause/resume/clear behavior is retained with tooltips and keyboard/accessibility handlers.

## Interaction and validation

- Native fixture exercises pause and clear through the real composer handler, RPC and document watch; no model/provider calls.
- 142 composer tests, 12 checklist tests, 18 queue tests, 39 presentation/localization tests pass.
- Two native decoder tests, two label-recovery parser tests and one engine cold-reopen recovery test pass.
- Final source whitespace check and desktop build are recorded separately in task output.

## Findings

No remaining actionable P0/P1/P2 issue in the requested component scope. Final captures were inspected together with the reference; exact cross-product color/font equivalence is not claimed.

## Implementation checklist

- [x] Show checklist text for native and legacy calls.
- [x] Recover existing blank labels from complete matching local logs.
- [x] Prevent checklist/queue overlap with the goal.
- [x] Attach the goal to the input edge with compact icon actions.
- [x] Verify active/paused/completed/cleared, narrow and dark states.

## Live desktop verification

Rebuilt the locked desktop target and gracefully restarted the user's existing RCode window. New process 3008 runs `D:/projects/zeron/target/debug/rcode.exe`, title `RCode`, nonzero HWND, responding. Actual selected conversation shows a completed goal and a 7/7 completed checklist (collapsed by the existing automatic rule), with a separate checklist row, attached goal header, visible clear action, and complete input controls. Evidence: `output/goal-todo-live-after.png` and its native capture metadata. No provider request was sent by validation.


## Follow-up: completed todos, active runtime and goal editor (2026-10-21)

final result: passed

- Source: `output/goal-edit-verified/reference-goal.png`; combined reference/native component comparison: `output/goal-edit-verified/reference-comparison.png`. The reference is unchanged; the native crop alone is normalized from 728 pixels to the reference width. Source display density is unknown.
- Todo visibility: the latest fully completed checklist is hidden immediately, even during an active turn. A subsequent list with pending work reappears. The transcript history remains available.
- Runtime: the header ticks each second while active; duration switches between seconds, minutes, hours and days. Paused/completed goals freeze the accumulated active time, resume preserves it, and restart recovery excludes offline time after the last durable activity. Old goal records deserialize with their recorded time intact.
- Editing: the rightmost header action opens a real right-pane tab. The multiline input has Save and Reload, supports Ctrl+S, validates 1–4000 characters, preserves a dirty draft across state updates and tab closing/reopening, and rejects stale goal IDs at the host. Saving preserves usage and pauses the goal; obsolete runner callbacks cannot mutate the edited objective. Side-chat goal actions use the side chat's own state.
- Native evidence: `goal-active.png`, `goal-paused.png`, `todos-finished-hidden.png`, `new-todos-visible.png`, `goal-narrow-390.png`, `goal-editor-open.png`, `goal-editor-saved.png`, `goal-todo-dark.png`, and `goal-cleared.png`, all under `output/goal-edit-verified/`. The fixture uses production Shell/Composer, native IPC and document watches, with isolated synthetic data and no provider requests.
- Native geometry: 760 x 760 client, input x16/y671/w728/h49; 390 x 850 narrow client; 1240 x 760 editor client. All header actions and full input remain visible. Rightmost edit follows clear and pause/resume. Existing fonts, icons, theme and attached-header geometry are retained; no promotional copy or extra explanatory blocks were added. The only new helper text explains the pause caused by saving an active goal.
- Validation: 2 protocol clock/backward-compatibility tests, 5 engine goal lifecycle/edit/restart tests, 4 UI goal/layout/draft tests, 15 checklist tests and 6 agent goal tests pass. Native fixture verifies pause, completion hiding, subsequent work visibility, draft retention, edit save/synchronization and clear through real handlers. Final desktop build/restart verification follows below.
- Findings: no remaining actionable P0/P1/P2 issue within the requested surfaces. Cross-product font/color pixel equivalence is not claimed; the native glass surface follows the product theme.


### Follow-up desktop verification

`cargo build --locked -p rcode` passed. Gracefully closed old process 3008 and launched the rebuilt `D:/projects/zeron/target/debug/rcode.exe`. New process 34372 has title `RCode`, nonzero HWND 10358242 and responds normally. The actual restored conversation now hides its completed 7/7 checklist and shows the recorded goal runtime as 9 minutes, plus the rightmost edit entry. Evidence: `output/goal-edit-verified/live-desktop.png`, 1171 x 667 client at 96 DPI. Four side-chat regression tests also pass, for 36 focused tests in this follow-up. Native fixture completed successfully without a provider request.


## Follow-up: save goal edits without pausing (2026-10-21)

final result: passed

This supersedes the earlier save-and-pause behavior and its helper text. Saving preserves the current run state, execution identity, active clock and usage. The objective revision increments atomically; stale editor drafts are rejected using both goal ID and revision. Pause/resume and replacement still retain their existing semantics.

The runner injects the latest saved objective before the next reasoning step. An edit during a model/tool step forces another reasoning step before verification. Verification commits compare the objective revision under the store lock, so an old pass, failure or malformed result cannot complete or pause the edited objective. Multiple saves during a step adopt the latest content without cancelling the current request. The pause warning was removed from both locales and the editor.

Validation: 8 agent goal tests, 5 engine goal tests and 4 UI goal/layout/draft tests pass. The native fixture saves an active goal through production IPC and checks that status remains active, execution ID and clock start remain unchanged, revision advances, and the new objective reaches the document watch. Native screenshots and result: `output/goal-next-round-verified/goal-editor-saved.png` and `result.txt`. No provider request was used. Existing native light/dark/narrow/todo visibility regression scenes also passed.


Desktop verification for this follow-up: locked RCode build passed; old window closed normally. Process 14052 runs the rebuilt `D:/projects/zeron/target/debug/rcode.exe`, title `RCode`, HWND 5441226, responding normally. Process metadata: `output/goal-next-round-verified/live-process.json`. The original user's completed conversation was preserved.
