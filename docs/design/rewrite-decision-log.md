# Decision Log — Autonomous Session (2026-10-12 night)

While the user slept, work continued autonomously under the standing directive:
"持续推进所有任务的完成，中途如果遇到需要抉择的事情，按照你的推荐执行。每次的抉择和选项需要记录，我醒来的时候需要再次确认是否偏离了我的预期。"

Every decision taken without the user is recorded here, with the options that
were on the table and why the recommendation was followed. **Review this list
on waking** — anything marked ⚠️ is a judgement call that could plausibly have
gone the other way.

---

## Confirmed by the user before sleeping (not autonomous)

| # | Decision |
|---|---|
| 1 | Branch: `rewrite/native-agent`, **local commits only, no push** |
| 2 | Order: **build new first, then delete old** |
| 3 | Scope: "推进全部" as far as possible |
| 4 | Rename to RCode: bundle id `com.rcode`, GitHub-releases instead of the old release feed, **remove every historical predecessor reference except `THIRD_PARTY_NOTICES.md`**, **do not rename the directory** |

---

## Autonomous decisions

### D1 — Where the new crates live
- **Options considered:** (a) new crates `crates/model` + `crates/agent`; (b) put everything inside `crates/harness`.
- **Chosen:** (a).
- **Why:** the user explicitly picked architecture "A" before sleeping (new crates, thin `NativeHarness`). Also keeps the provider protocol layer testable without the harness.
- **Reversible?** Yes, at the cost of moving files.

### D2 — `AgentRunner` owns no storage
- **Options:** (a) runner writes the session doc itself; (b) runner emits events, host persists.
- **Chosen:** (b).
- **Why:** the user decided Loro stays the single source of truth. A runner that also persisted would create the second source of truth we explicitly ruled out.
- **Reversible?** Yes.

### D3 — Cancellation via `CancellationToken`, not stream-drop
- **Options:** (a) dropping the event stream cancels the run; (b) an explicit token.
- **Chosen:** (b), with the token also threaded into every tool call.
- **Why:** a dropped stream would leave a half-run `Bash` alive. The user wants several conversations running concurrently, so cancellation has to be explicit and per-conversation.
- **Reversible?** Yes.

### D4 — `max_steps` ceiling defaults to 100 ⚠️
- **Options:** (a) no ceiling; (b) a fixed default; (c) per-conversation config only.
- **Chosen:** (b) 100 steps, overridable per `AgentConfig`.
- **Why:** a model that loops on tool calls forever would burn the user's API budget unattended. 100 is generous for real work but bounded.
- **Why it might be wrong:** a legitimate long refactor could exceed it. The user may want this higher, or exposed in settings. **Flagging for review.**

### D5 — `is_edit_tool` recognises names `Write`/`Edit`/`NotebookEdit`
- **Options:** (a) let each tool declare "is an edit"; (b) a name list in the runner.
- **Chosen:** (b) for now.
- **Why:** fewer moving parts; the tool set is fixed and known.
- **Why it might be wrong:** a name list drifts if tools are renamed or user-supplied MCP tools want the same treatment. A `Tool::mutates() -> bool` would be more robust. **Flagging.**

### D6 — Tools run sequentially within a turn ⚠️
- **Options:** (a) run the assistant turn's tool calls in parallel; (b) in order.
- **Chosen:** (b).
- **Why:** ordered results are easier to reason about, and mutating tools (Write/Edit/Bash) on the same file would race. The original project dispatches concurrently, so this is a **deliberate divergence** from the reference.
- **Trade-off:** slower for read-heavy turns (e.g. five `Read`s at once). Worth revisiting once there is a way to know a tool is read-only.

### D7 — Input requests route through `AgentHost::ask`
- **Options:** (a) separate permission callback and question callback; (b) one `ask`.
- **Chosen:** (b).
- **Why:** the user explicitly asked to unify permission confirmation and `AskUserQuestion` into one "agent needs input to continue" mechanism.
- **Note:** the enum `InputRequest::{Permission, Question}` keeps the two distinguishable for the UI.

### D8 — Directory search order for `rcode-model` port
- **Options:** (a) copy all 24 production files; (b) copy only the 13-file shared core, then add adapters.
- **Chosen:** (a) — full production set including the Anthropic and OpenAI Chat adapters.
- **Why:** the investigation concluded (a) is more stable; the shared core alone is not a compilable crate, and the adapters are wanted anyway.
- **Reversible?** Yes.

### D9 — Tests ported later, not now
- **Options:** (a) port the original project's test files alongside the code; (b) port production code first, tests after it compiles.
- **Chosen:** (b).
- **Why:** the original project keeps tests in sibling `*_test.rs` files (6,360 of 12,117 lines). Porting them doubles the surface for name/API mismatches before the code even compiles. **This leaves the port weaker than upstream until tests come back — flagging.**

### D10 — Delete `mock.rs` only if it does not break too much ⚠️
- **Options:** (a) delete `MockHarness` with the other adapters; (b) keep it.
- **Chosen:** instructed the agent to **keep it if removing it breaks many tests** — it is test infrastructure, not external integration.
- **Why:** the user's stated goal is removing *external agent* integration; a mock harness is not that. But it is also not needed in a shipping build.
- **Flagging:** if it survives, it is dead weight in production; a `cfg(test)` gate may be the right end state.

### D11 — `rcode-preview`: report, do not touch ⚠️
- **Options:** (a) strip its P2P/edge-signalling half now; (b) leave it and report.
- **Chosen:** (b).
- **Why:** the user said "local preview only, no cross-device" but also that preview is wanted. Splitting it is a design change, not a mechanical delete, and I did not want to make that call unattended.
- **Flagging for review.**

### D12 — Sequential tool execution means no per-tool concurrency limit
Follows from D6; recorded so the divergence is visible.

### D13 — Gateway kills long-running subagents; core work moved in-house ⚠️

- **Observed:** six subagents died with the same error — a Cloudflare-fronted
  gateway returning an empty response after ~60s of stream silence. The pattern
  hits long tasks (builds, multi-file reads) and never hits this session's own
  tool calls.
- **Decision:** stopped delegating work that needs a long uninterrupted run.
  Wrote the ReAct loop, the tool set and compaction directly. Kept delegating
  short, bounded tasks.
- **Why:** three consecutive retries of the same task all died the same way;
  retrying was pure waste.
- **Consequence:** throughput is lower than fully parallel, but results land.
  **If the gateway can be avoided (direct API, different network), long
  subagents become viable again and would speed things up.**

### D14 — `rcode-agent` owns no storage, confirmed in practice
The loop emits events and calls `AgentHost::ask`; it never touches the session
doc. This is what lets several conversations run concurrently (D2) and keeps
Loro the single source of truth.

### D15 — Tool set implemented without an output-size taxonomy ⚠️
- **Options:** (a) per-tool `max_output` on the trait; (b) constants per tool.
- **Chosen:** (b) — each tool truncates with its own constant and a shared
  `truncate` helper.
- **Why:** faster to land; the values are easy to find and tune.
- **Flagging:** the tool-framework agent argued for a trait-level limit. It is
  the better long-term shape; revisit when output sizes are seen in practice.

### D16 — `NativeHarness` drops the loop's `InputRequested` event ⚠️
- **Options:** (a) forward the loop's `InputRequested` to the engine; (b) let
  the engine mint its own.
- **Chosen:** (b) — the engine's `request_input` bridge already mints a request
  id, parks the resolver `RespondInput` needs, and emits its own event.
  Forwarding both would put two questions on screen, one unanswerable.
- **Flagging:** this leaves the loop's permission-ask path **unreachable**
  through the engine — under `PermissionMode::Default` a tool needing approval
  is denied rather than prompted. **Permission prompts need real wiring before
  the agent is usable interactively.**

### D17 — `HarnessId::Native` added with an explicit serde rename
`#[serde(rename = "native")]` rather than relying on `rename_all`, because the
token is minted into session docs and settings and should be a decided
constant. Appended before `Mock`, so it is additive.

### D18 — Size work applied; `opt-level = "s"` deliberately left off ⚠️

Measured ladder (Windows x86_64, cold builds):

| Config | Size | Δ |
|---|---|---|
| baseline (`lto="thin"`, cgu=16) | 166,235,136 | — |
| + `rust-embed` compression | 165,786,624 | −0.45 MB |
| + `codegen-units = 1` | 154,908,672 | −11.3 MB |
| + `lto = "fat"` (**applied**) | **150,544,384** | **−15.7 MB (−9.4%)** |
| + `opt-level = "s"` (**NOT applied**) | 121,553,408 | −44.7 MB |

- **Applied:** `lto = "fat"`, `codegen-units = 1`, `rust-embed` `compression`.
- **`opt-level = "s"` is the single largest lever left (−29 MB) and I left it
  off.** It is a GPUI render app with hot rendering paths; the win is a
  runtime-speed trade that belongs to the product owner, not a free one. The
  numbers are recorded in the profile comment at the point of change so the
  decision is visible where it would be made.
- `panic = "abort"` still not added (three `catch_unwind` sites).
- Build time cost: 20m → 31m cold. Almost all of it the final `link.exe`.

### D19 — Engine voice: stub rather than delete ⚠️

The deletion agent found that engine's voice feature is built entirely on
`rcode_harness::codex::realtime` — the Codex CLI's WebRTC helper. Removing the
Codex adapter left it with nothing to call.

- **Options:** (a) delete voice entirely; (b) stub the engine side.
- **Chosen:** (b).
- **Why:** the user explicitly said to keep `voice-session` (the call
  lifecycle state machine, 1,179 lines, self-contained). The dependency on an
  external CLI is what has to go, not the feature's shape. Stubbing keeps the
  RPC handlers and types in place, so a local voice implementation can be
  dropped in later, and the user sees "voice unavailable" rather than a crash.
- **Flagging:** voice is **non-functional** until a non-Codex runtime exists.

### D20 — The Providers settings page was NOT deleted ⚠️

- **Options:** (a) delete `crates/ui/src/settings/harnesses.rs` (1,429 lines)
  and the agent picker in `pickers.rs`; (b) leave them and report.
- **Chosen:** (b).
- **Why:** they are wired into the Settings shell through
  `SettingsSection::Harnesses` — a route, a `HarnessesPage` entity and the
  section enum. Removing them means restructuring the shell's match arms and
  entity management, which is a rewrite rather than a reference cleanup, and I
  would rather the user see that than have me improvise it.
- **Flagging — this leaves stale UI:** the page still shows the *old*
  "install a CLI" copy for harnesses that no longer exist. It compiles and does
  not crash, but it is now describing the wrong product. **It should be
  rewritten to show provider/model/skill configuration** once the native agent
  is live.
- Same bucket: `crates/ui/src/shell/harness_updates.rs` and `state.rs` still
  call removed engine RPC methods. They compile, and the UI-side state is
  local, but the page's "check for updates" button will get `UnknownMethod`.

---

## Resolved by the user after waking (2026-10-13)

| # | Decision |
|---|---|
| **D4** | **No default step ceiling.** The loop runs until it answers or the caller cancels. The ceiling stays configurable, but unset by default. |
| **D10** | **Gate `mock.rs` behind `cfg(test)`** so it stops shipping in the production binary while staying available to tests. |
| **D11** | **Split `rcode-preview` now** — remove the edge signalling and P2P half, keep local HTTP preview. |
| **D13** | Gateway is flaky; **just retry** rather than avoiding long subagents. |
| **D15** | **Keep per-tool constants** — not a problem worth a refactor. |
| **D16** | **Must fix.** Permission prompts have to reach the UI; `Default` mode is unusable without it. |
| **D18** | **Do not enable `opt-level = "s"`.** A runtime slowdown is not acceptable, whatever the size win. |
| **D19** | **Delete voice entirely.** The user does not need it — this supersedes the earlier "keep `voice-session`" instruction. |
| **D20** | Leave the Providers page alone for now; the user will look at it running and decide. |

### D21 — Permission prompts reuse the question bridge

The fix for D16, decided while implementing it: rather than adding a variant to
`RunControls::request_input`, a permission confirmation is expressed as an
ordinary `UserInputQuestion` whose options are the decision.

- **Why:** the user asked for one mechanism for "the agent needs input to
  continue" (D7). The bridge, the resolver, the `InputRequested` /
  `InputResolved` events and the QuestionPanel already exist and already work;
  a parallel permission channel would be a second path to keep in sync.
- **Shape:** question id `permission:<tool call id>`, options
  `["Allow", "Allow always", "Deny"]`. The answer comes back as
  `UserInputAnswer { labels }`, which the harness maps to
  `InputAnswer::Allowed { always }` or `Denied`.
- **Consequence:** a permission prompt looks like a question in the UI. That is
  a real trade — a permission wants an Allow/Deny affordance, not a question
  card — but it is the honest first step: it makes `Default` mode work at all,
  and a dedicated UI can follow without touching the transport.


## ⚠️ Mistake made during the autonomous session

### M1 — Committed another agent's in-progress work

`git commit` for "Add the agent's message, permission and tool layers" was
meant to contain only three new files under `crates/agent/src/`. A `git reset
HEAD <paths>` to unstage unrelated changes did not take effect as intended, so
the commit also captured everything the *deletion* agent had already `git add`-ed
— the `crates/mcp` removal (2,900+ lines) among it.

**Consequence:** commit `96395d0d` contains a partial deletion. At that commit
`cargo check -p rcode-harness` fails (4 errors: `catalog_failure`, `codex`,
`skills` no longer resolvable from `lib.rs`), because the deletion agent was
mid-flight.

**Not fatal:** the deletion agent is still running and will finish the cleanup;
its final state is what matters. The commits are local only, and the branch was
never pushed.

**If the deletion agent dies before finishing** (three agents have already been
killed by the same gateway error), the branch head will not compile and the
cleanup must be completed by hand. Check `git status` / `cargo check -p
rcode-harness` before doing anything else.

**Lesson:** when several agents share one working tree, stage explicit paths and
verify `git status` *after* staging, before committing.

---


These were deliberately left for the user rather than guessed:

1. **`reqwest` 0.12 vs 0.13** — allowing both is easy but bloats the dependency tree; upgrading RCode touches every HTTP call site. The port agent was told to try 0.12 first and report.
2. **Whether ported tools should reuse RCode's UI-layer file/PTY code** — the investigation leant "no" (different authorisation models), but this is a real architecture choice.
3. **Exact provider/model settings schema.**
4. **`auto_classifier` for `AutoMode`** — it costs an extra LLM call per gated tool. The user asked for all four modes, so it is in scope, but the cost model is worth confirming.
