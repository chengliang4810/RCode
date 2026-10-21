# Native Agent Rewrite — Progress

Status: IN PROGRESS · branch `rewrite/native-agent` (local only) · 2026-10-13

Companion to [`native-agent-rewrite.md`](native-agent-rewrite.md) (the decisions)
and [`rewrite-decision-log.md`](rewrite-decision-log.md) (the choices taken
unattended overnight). This file says what actually exists.

## What is implemented

| Piece | Lines | State |
|---|---|---|
| `crates/model` — provider protocol layer | ~6,600 | Ported from the original project, compiles, tests pass |
| — OpenAI Chat Completions | — | ported |
| — Anthropic Messages | — | ported |
| — **OpenAI Responses** | ~1,850 | **new** — the original project had no implementation |
| `crates/agent` — ReAct loop | 630 | done, tested |
| — message / permission / tool model | 1,345 | done, tested |
| — built-in tools (7) | 1,673 | done, tested |
| — compaction | 569 | done, tested |
| `crates/harness/src/native.rs` | 1,205 | done, tests pass — **but not wired** |
| External adapters deleted | ~31,000 | done (acp/opencode/codex/claude/pi/cursor + infra) |
| engine-side dead code deleted | ~10,000 | done |
| Release profile + icon compression | — | measured −15.7 MB |

**109 tests pass** across `rcode-agent` (91) and `rcode-model` (21 by the
final run).

## What is not wired yet — the honest list

**The chain is connected end to end**: a run resolves a provider, builds a
client, streams from the model, executes tools, and reports events the engine
already knows how to fold. What remains is the interactive surface.

1. ~~`NativeHost::reason` returns `AgentError::Host`.~~ **Done** — resolves the
   run's provider, builds the client, folds the stream into the loop's terms.
   Provider/model config lives in `{data_dir}/providers.json`.
2. ~~`NativeHarness` is not registered in the engine.~~ **Done** —
   `registry_with_native(data_dir)` registers it on the production path.
3. ~~The tool registry handed to runs is empty.~~ **Done** — seven built-ins.
4. **Permission prompts do not reach the UI.** Under `PermissionMode::Default`
   a tool needing approval is denied rather than prompted. See decision D16.
   **The agent is therefore only usable with `auto_approve` until this lands.**
5. **The provider settings UI does not exist.** The file, its loader and its
   diagnostics do; nothing writes it yet, so `providers.json` has to be
   authored by hand.
6. **Voice is non-functional.** The engine side is stubbed — it was built
   entirely on the Codex CLI's WebRTC helper. See decision D19.
7. **No resume.** Every run starts with an empty transcript; the loop has no
   replay path.
8. **`AskUserQuestion` is not implemented** as a tool, so the loop never asks a
   direct question even though the bridge for it is wired end to end.
9. **Mobile transport unchanged.** Still pointed at the edge. See §8 of the
   design doc for the plan.
10. **Rename to RCode not started.** Deliberately last.

## Known stale surfaces (compile, but describe the old product)

These were kept because removing them is a UI rewrite rather than a reference
cleanup. See decision D20.

- `crates/ui/src/settings/harnesses.rs` (1,429 lines) — the **Providers page
  still shows the old "install a CLI" copy** for harnesses that no longer
  exist. Should be rewritten around providers/models/skills.
- `crates/ui/src/shell/harness_updates.rs` + `state.rs` — still call removed
  engine RPC methods. The UI-side state is local so nothing crashes, but the
  page's "check for updates" button will receive `UnknownMethod`.

## Environment notes

- **The D: drive filled to 100% mid-work** and stopped every build. The
  workspace `target/` reaches ~37 GB. Three finished worktrees' caches were
  removed to recover. **Build with `CARGO_INCREMENTAL=0`** to slow the refill.
- Pre-existing and unrelated: `cargo build --workspace --tests` fails on
  `crates/preview/src/login.rs`, which uses `tokio::net::UnixStream`
  unconditionally and does not compile on Windows.

## Verification

- `cargo check --workspace` — clean.
- Offline: **281 tests pass** across `rcode-agent` (156), `rcode-harness`
  (104) and `rcode-model` (21), including ten deterministic cancellation
  tests covering every wait point.
- **Live runs (2026-10-13, after the fixes): three consecutive full rounds,
  6/6 scenarios each, ~22s per round** (before the tool-result fix the same
  suite needed 105–125s because the blind model retried a dozen tools).
  Passing: plain conversation, read-through-a-tool (the fix's regression: the
  model reports the file's content it actually received), write-and-verify,
  the gated permission flow (prompt observed before approval, file created
  only after), the Write sandbox refusal, and unknown-model rejection.
  tool results never reached the model (empty content, lost ids/error state),
  reasoning signatures dropped and thinking blocks fabricated on replay,
  Bash running bash syntax through cmd, and model=None chats failing on a
  placeholder default.
- **Live rounds 2+ are blocked**: the test proxy now answers every request
  with a paywall message as model output (its backing account is out of
  credits). Resume by topping up that account or pointing `providers.json`
  at another endpoint; the suite is `cargo test -p rcode-harness --test
  native_live -- --ignored --test-threads=1` with `RCODE_LIVE_KEY` and
  `RCODE_LIVE_URL` set.

## What a first run needs

1. A `providers.json` in the data directory — see `crates/harness/src/providers.rs`
   for the shape. The minimum is one provider with one model and a key.
2. `auto_approve` on the run (see item 4), or `Bypass` permission mode, since
   the approval prompt is not wired.

## Measured binary size

| Step | Bytes | Δ |
|---|---|---|
| baseline | 166,235,136 | — |
| − on-device dictation (merged to `main`) | 142,224,896 | −22.90 MiB |
| − external adapters (measured, deletion in progress) | 135,940,096 | −28.89 MiB |
| − profile + icon compression | 150,544,384 (before the deletions land) | −15.69 MiB |

The last three overlap: the deletion and profile work were measured against
different baselines. A single figure needs one build with everything applied.

`opt-level = "s"` remains untaken — worth a further ~29 MB, but a runtime-speed
trade. See decision D18.

## Suggested next steps

1. Finish the deletion (WorkOS, the last `HarnessId::Native` match arms).
2. Build once with everything applied and record one honest size figure.
3. Wire `NativeHost::reason` to `rcode-model` — the blocker.
4. Register `NativeHarness`, populate the tool registry, then get one real
   conversation running end to end. Everything else is easier to judge with a
   working run in hand.
5. Provider/model settings, then permission prompts.
