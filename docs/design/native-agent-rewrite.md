# Native Agent Rewrite — Decisions

Status: DECIDED (design phase complete) · 2026-10-12

This document records the decisions taken for replacing RCode's external-agent
integration with a native, in-process agent, and for making the whole product
local-only. It is the durable record of a design discussion; nothing here is
implemented yet unless a section says so.

Reference implementation we are porting from: **the reference implementation**
Apache-2.0. See "Licensing" below for what that obliges.

---

## 1. Product direction

RCode stops driving external agent CLIs (Claude Code, Codex, Cursor, Devin,
Grok, Hermes, Pi, opencode, Antigravity) and stops speaking ACP. It implements
its own agent in Rust: it calls LLM APIs directly and runs its own tool-calling
loop.

**RCode becomes a pure-Rust, local-only desktop application.** No account, no
cloud sync, no remote service. This is a hard constraint, not a default.

Two consequences that shaped every decision below:

- Anything that only exists to talk to the edge (WorkOS auth, Durable Objects,
  the chat2/registry WebSocket rooms) goes away.
- Anything that exists to make *local* operation reliable (the durable command
  ledger, CRDT session docs) stays.

## 2. Architecture: where the new code lives

Chosen shape (option A from the discussion):

```text
OpenAI Chat / OpenAI Responses / Anthropic Messages
                   |
        rcode-model    (provider clients, streaming, retry)
                   |
        rcode-agent    (ReAct loop, tools, user questions, compaction)
                   |
        NativeHarness  (thin adapter: RunRequest/RunControls in,
                   |    RCode AgentEvent out, tool bridge, cancellation)
                   |
   existing RCode engine / Loro docs / GPUI UI   (unchanged)
```

`NativeHarness` implements the existing `Harness` trait
(`crates/harness/src/lib.rs`) and produces the existing `AgentEvent` stream
(`crates/proto/src/agent.rs`). The downstream — engine session management, Loro
persistence, RPC, GPUI rendering — does not change.

**We keep the internal `AgentEvent` contract.** What we remove is the *external*
protocol surface: ACP, each vendor's CLI wire, the shims and installers that
managed those CLIs. Mapping provider streams onto `AgentEvent` is an internal
adapter, not an ACP adapter.

### What gets deleted

harness crate, external integration (~22,000 lines):

- `acp/` (9,623 — Devin/Grok/Hermes/Antigravity)
- `opencode/` (7,264), `codex/` (4,860), `claude/` (4,076), `pi/` (2,058),
  `cursor/` (1,237, includes an embedded `shim.mjs`)
- infrastructure: `adapter_install` (863), `archive_install` (587),
  `install` (558), `executable` (996), `code_signature` (350), `scratch` (167),
  `skills` (952), `shell_env` (351), `mock` (652)

Kept from harness: `jsonrpc.rs`, `process/`, `redact.rs`, `catalog.rs`,
`model_context.rs`, and the `Harness` trait itself.

engine-side dead code (~10,000 lines): `harness_updates.rs` (3,748, exists only
to monitor external CLIs), `agent_accounts/` (4,412), `titles.rs` (504),
`model_catalog.rs` (464). UI: `settings/harnesses.rs` (1,429) and the agent
picker in `pickers.rs`.

Also deleted:

- `mcp` — the MCP *server* that let other agents drive RCode. (Note this
  is the opposite direction from the MCP *client* we keep for user-configured
  MCP servers; see §5.)
- `voice-media` (666) — only used by `codex/realtime_host.rs` and the UI;
  dies with the codex adapter. Takes `semver` with it.
- WorkOS auth (`crates/engine/src/auth.rs`), edge clients in `rcode-sync`,
  `rcode-client`'s edge transport.

## 3. Protocols

Three provider protocols:

1. **OpenAI Chat Completions** — the original project has this.
2. **OpenAI Responses API** — the original project does **not** have this; new work.
   Estimated 600–1,000 lines of implementation plus 500–1,000 of tests
   (estimate, not measured). Reuses the existing HTTP/SSE/retry/cancel
   pipeline; no new SDK dependency.
3. **Anthropic Messages** — the original project has this.

The original project's abstraction (`model/src/protocol/`) normalizes all providers into
`ModelRequest` → `ModelStream` → `ModelStreamEvent`. Adding a protocol is a new
adapter module, not an architecture change.

### Porting notes

- `model` production code: 24 files, 5,757 lines. The shared
  protocol/runtime/HTTP/SSE core is 13 files, 3,068 lines.
- Only internal dependency is `time`; external deps are async-trait,
  futures, serde, tokio, reqwest, rand, url — all common.
- **Dependency conflict**: The original project pins `reqwest 0.13.4`; RCode is on `0.12`.
  Decide between allowing both versions or upgrading RCode and validating.
- the original project's `ResolvedProvider` derives `Debug` and its tests assert the output
  contains the API key and base URL. **Do not log it.**

## 4. Model configuration

Multiple providers, multiple models each, selected per conversation. Stored in
RCode's existing settings (not a new config file).

Sub-agents select their model by **profile name**, using the original project's four-tier alias
scheme: `fable` / `opus` / `sonnet` / `haiku`. A profile maps to a
provider+model, so a sub-agent can run on a different vendor than its parent.

The original project's mechanism, for reference: the host injects a
`Fn(Option<&str>) -> SubagentLlmSource` factory; an explicit `model` override
wins over agent-definition frontmatter, `inherit` means the parent's model, and
unknown tiers are rejected rather than silently falling back.

## 5. Tools

Full the original project tool set, except `artifact` (share-as-link) and `workflow`
(orchestration), which are dropped.

Built-in tools are invoked by **direct function call**, not wrapped in MCP.
MCP is a separate layer for **user-configured external MCP servers**. The two
paths meet only at the model-visible tool list.

Naming: all PascalCase. This renames the original project's `folder_operations` →
`FolderOperations` and `cron_register`/`cron_list`/`cron_remove` →
`CronRegister`/`CronList`/`CronRemove`. Name collision rule: built-ins keep the
bare name; MCP tools are prefixed `mcp__<server>__<tool>`.

Tool inventory (the original project source, production lines):

| Tool | Where | Lines |
|---|---|---|
| `Read` | `mcp-packages/workspace/src/filesystem/read.rs` | 335 |
| `Write` | `.../write.rs` | 250 |
| `Edit` | `.../edit.rs` | 293 |
| `Glob` | `.../glob.rs` | 441 |
| `Grep` | `.../grep.rs` | 644 |
| `FolderOperations` | `.../folder.rs` | 487 |
| `Bash` | `mcp-packages/workspace/src/terminal.rs` | 894 |
| `WebSearch` | `mcp-packages/web/src/web_search.rs` | 255 |
| `WebFetch` | `mcp-packages/web/src/web_fetch.rs` | 258 |
| `TodoWrite` | `middlewares/src/tools/todo.rs` | 224 |
| `AskUserQuestion` | `middlewares/src/tools/ask_user_tool.rs` | 171 |
| `Agent` | subagent machinery | — |
| `SkillTool` / `DiscoverSkillsTool` | skills machinery | — |
| `Cron*` | `mcp-packages/cron` | — |
| ToolSearch (`ExecuteExtraTool`/`SearchExtraTools`) | `middlewares/src/tool_search/` | — |

Supporting modules worth porting: `transaction.rs` (160, edit transactions),
`write_sandbox.rs` (478, write permission), `file_observation.rs` (289,
external-modification detection), `file_rewind.rs` (196), `shell_tasks.rs`
(668, background tasks), `path_hints.rs` (148).

Total tool implementation: roughly 6,900 production lines.

**Important:** RCode already has `workspace_files.rs`, `workspace_search.rs`
and `terminals.rs`, but those serve the **UI** (file tree, editor, `@`
mentions, cmd+K, the user's own terminal tabs) — they are not an agent tool
execution layer. The apparent duplication is two different consumers, which is
fine.

## 6. Direct execution and user questions

RCode runs in a fully trusted mode. Tools execute directly; there is no permission
selector, approval policy, approval request/response, or per-run allowlist.
Legacy permission fields are ignored when reading existing chat/run settings and
are omitted from new writes. Tool argument/path validation, cancellation, timeouts,
and user-configured lifecycle hooks still apply.

`AskUserQuestion` retains its input bridge: the loop suspends, sends typed questions
and stable option ids to the UI, awaits the user's answer, then resumes. Dismissing
the prompt or cancelling the run remains distinct from submitting an empty answer.
The same bridge is used by subagents and the mobile client.

## 7. Persistence — keep Loro

**Decision: keep Loro.** the original project's storage layer (`ThreadStore`, SQLite via
`sqlx`, `~/.rcode/threads/threads.db`) is **not** ported.

Why: the mobile app is a **remote control**, not a read-only viewer. It sends
messages, steers mid-turn, interrupts, attaches images, and picks the model —
through the **durable command ledger**. Commands are written into the session
doc and cannot be lost: the phone can go offline mid-send, the desktop can
restart, and the command still executes afterwards.

That is exactly what the CRDT is for. The phone and the desktop are **peers
sharing a document**, not a client and a server.

So the split is:

| Layer | What |
|---|---|
| In-memory message model | the original project's `BaseMessage` / `MessageContent` — good for the ReAct loop and provider conversion |
| Persistence | **RCode's Loro session doc** — already has `messages` / `commands` / `queue` |
| the original project's SQLite `ThreadStore` | **not ported** — would be a second source of truth |

Compaction markers: The original project marks messages `truncated` / `excluded` instead of
deleting them. RCode's doc schema already carries a `status?` field per message
and a `MessageStatus` enum, which this can extend.

What changes is only the **transport**: edge WebSocket becomes a direct
connection (§8).

### Why an offline command queue matters

Concretely: you type "fix the login bug" on your phone in a lift with no
signal. With a plain request, it fails and you retype it. With the ledger, the
command is written locally as `Pending`, syncs when connectivity returns, the
desktop picks it up, executes, and writes back `Applied` along with the result.
Statuses are `Pending` / `Applied` / `Rejected` / `Expired` / `Superseded` /
`Cancelled` (`crates/doc/src/commands.rs`).

## 8. Mobile: keep it, re-point the transport

The mobile app stays and is **adapted**, not deleted. It is local-only: the
phone connects to the desktop over **user-provided tunnelling** (frp, Tailscale,
Cloudflare Tunnel, ngrok, …). We do not implement tunnelling.

What is already there and stays:

- `apps/ios` (9.6 MB) — a complete UIKit app: transcript virtualisation,
  composer with queue/steer/interrupt, sessions list, settings, Live Activity,
  TestFlight release flow, APNs.
- `crates/mobile` — `layout/` (~4,000 lines of markdown measurement, row
  heights, display lists) is **platform-neutral and network-free**; `orb/`,
  `caption/`, `wallpaper/` are pure rendering. All reusable as-is.

What changes:

- `crates/client` (11,815 lines) — **adapt**. Delete `auth.rs` (598, WorkOS);
  rewrite `live/room.rs` (724) and `live/relay.rs` (381). Keep `session/`
  (~2,200, incremental session mirroring) and `demo/` (~2,400).
- `rcode-sync` — **split**. Delete the edge clients (`ChatClient`,
  `RegistryClient`); keep the local `DocsStore` (SQLite snapshots + command
  ledger).
- `rcode-preview` — **local HTTP preview only, no cross-device P2P**. The edge
  signalling and peer transport are removed.

Engine side: the RPC server already exists (`crates/engine/src/lib.rs` —
`ipc_port`, `start_host_relay`); it currently binds localhost. Serving the LAN
is a matter of binding a different address.

**Auth**: the desktop generates a token; the phone pairs by scanning a QR code
or typing it. The user may change the token themselves, minimum 8 characters,
no case requirement. Recommended hardening on top of that minimum: generate a
strong default, warn on weak values, rate-limit failures.

**Priority**: the mobile app is not in the desktop binary's dependency closure,
so it does not affect desktop size. **Finish the desktop agent work first.**

## 9. Compaction and skills

**Compaction**: port the original project's full implementation (~3,850 lines:
`stages/compact.rs`, progress/pressure helpers, `compact_v2`). It is optional in
The original project; we take it. Its markers land in the Loro doc (§7).

**Skills**: keep — port `SkillTool` / `DiscoverSkillsTool` and the discovery /
loading machinery.

## 10. Concurrency

Multiple conversations per project, and multiple projects, run concurrently.
Each conversation advances independently.

RCode already supports this: runs are keyed by `chat_id`
(`crates/engine/src/sessions.rs`), with per-chat steer and cancel, and no global
run semaphore. The native agent just needs an independent run context and
cancellation token per conversation.

## 11. Licensing

The original project is Apache-2.0. That permits copying, modifying and commercial use. It does
**not** permit stripping attribution. Concretely, when porting:

- keep the Apache licence text and the applicable copyright notices,
- state that files were modified,
- do not present the derived code as unencumbered original work.

RCode is MIT; it does not have to become Apache-2.0. Add an entry to
`THIRD_PARTY_NOTICES.md` alongside the existing ones.

## 12. Sizing, for context

Measured on this machine (Windows x86_64, `cargo build --release -p rcode`):

| | Bytes | MiB |
|---|---|---|
| Baseline `rcode.exe` | 166,235,136 | 158.5 |
| After gating on-device dictation off | 142,224,896 | 135.6 |
| After *also* excluding the external-agent adapters | 135,940,096 | 129.6 |

**On-device dictation (ONNX Runtime, statically linked)** is implemented and
merged: `voice` is an optional dependency behind a non-default
`dictation` feature, with `crates/ui/src/dictation/rt.rs` as the single place
the UI reaches the runtime. Measured: −24,010,240 bytes (−22.90 MiB, −14.44%).

**Excluding the external-agent adapters** measured a further −30,295,040 bytes
(−28.89 MiB, −18.22%) — far more than the 3–6 MiB this document originally
predicted. The reason is worth recording: the adapters' *own* source is small,
but they were the sole consumers of whole dependency stacks (`zip` + `sha2` for
archive install/verify, `semver` for Antigravity releases, `deser-hjson` +
`serde_yaml_ng` for harness config, and the reqwest/rustls/HTTP/SSE chains those
drivers pulled in). **The bloat was in the dependency graph those integrations
activated, not in the integration source.** `cargo bloat` showed the harness at
3.6 MiB because it measures `.text` per crate and cannot see this.

Measurement caveat: the run used `#[cfg(any())]` plus empty stub types, so the
workspace would link. It is a measurement, not a production deletion.

Together: 158.5 → 129.6 MiB, a 18.2% reduction from these two changes alone,
before any of the zero-cost work (asset compression, profile tuning) or the
packaging compression ratio (158 MB → 33 MB setup.exe, ≈4.8×).


## 13. Open items

- Whether to allow both `reqwest` 0.12 and 0.13, or upgrade.
- Measured size gain from deleting the external-agent adapters (experiment
  restarted; previous attempts died on a gateway error).
- Whether the ported tool implementations reuse RCode's UI-layer file/PTY code
  or stay self-contained. Current lean: self-contained, because the UI layer's
  authorisation model (synced chat/space → owned folder) is not the agent's
  sandbox model.
- Exact shape of the provider/model settings schema.
