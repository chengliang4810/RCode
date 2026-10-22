# RCode — Architecture

RCode is a native coding agent under active development. The agent runs in the
application process, executes tools on the host, and sends model requests to the
provider configured by the user. The desktop UI uses GPUI; the engine and agent
are Rust. Sessions and application state persist locally.

## 1. Runtime and storage

```text
GPUI desktop UI ── typed RPC ── Engine ── NativeHarness ── Agent loop ── Model provider
                                  │                         │
                            Local persistence          Tools / MCP
                                  │
                       Documents, journals, uploads
```

`apps/rcode` builds one binary:

- `rcode` opens the desktop UI. It connects to an existing local engine when
  available; otherwise it starts an engine in the process and serves its IPC
  interface. Both paths use the same typed RPC protocol.
- `rcode headless` runs the engine without a UI.
- `rcode status` and `rcode sync` inspect the engine and transport state.
- `rcode daemon` manages the headless service through launchd or systemd on
  supported platforms. `rcode update` checks and installs application updates.

The engine uses Tokio. GPUI work crosses the async boundary through
`gpui_tokio`; filesystem, model and transport work belongs outside UI rendering.

### Storage: two roots, one profile

The config root defaults to `~/.rcode`. `RCODE_DATA_DIR` overrides this root for
one launch. Its `location` file can point to a separate, existing data directory;
without a pointer the config and data roots are the same.

- **Config root:** device identity, engine lock, `providers.json`, application
  settings, logs and the `location` pointer.
- **Data root:** profile stores, documents, journals, uploads, cloned
  repositories, scratch directories, caches and worktrees.
  `RCODE_WORKTREES_DIR` can override the worktree directory.

`EngineProfile` resolves an immutable store boundary for the running engine.
Stores use `{data_root}/orgs/{org_id}/{user_id}/`, with attachments in `uploads/`.
The identity fields are storage and protocol keys; the default local identity
comes from `RCODE_ORG_ID` / `RCODE_USER_ID` or `dev-org` / `dev-user`.
Provider settings are loaded from the config root, including when the data root
is on another drive. See [Windows development](docs/reference/windows-development.md).

## 2. Data model and command execution

- **Session documents** in `crates/doc` use Loro for transcript entries,
  structured parts, metadata and the durable command ledger. Text bodies use
  Loro text containers. Tool display payloads are bounded separately from the
  full event data retained in local run journals.
- **Workspace registry** uses `RegistryDoc`: keyed device, space, chat and
  session rows with field clocks. In local mode, writes apply directly to the
  stored rows. This registry is a row table, not a Loro session document.
- **Persistence** in `crates/sync::DocsStore` stores snapshots, progress and
  processed commands in SQLite. The engine coordinates persistence with its
  document cache and session lifecycle.
- **Commands** carry send, steer, interrupt and user-input responses. The owning
  host drains the ledger and records processed commands to prevent duplicate
  execution during replay and recovery.

The code also contains chat2 and registry transport clients and relay framing;
those protocols require a compatible registry endpoint. Direct desktop and
Flutter mobile clients instead use the authenticated RPC listener on a desktop
or headless engine, without a separate registry service. Local desktop operation
does not require a remote transport endpoint.
See [chat2](docs/chat2-sync.md), [registry](docs/registry-sync.md) and
[mobile setup](apps/mobile/README.md) for their implementation boundaries.

## 3. Cargo workspace

The workspace contains `apps/rcode` and these 25 crates. `apps/mobile` is a
Flutter application for Android and iOS. Dart owns its pages and authenticated
RPC client; portable Rust visual and math engines are exposed through a C ABI.
The required Flutter platform runners remain in `apps/mobile/android` and
`apps/mobile/ios`.

| Directory | Responsibility |
| --- | --- |
| `crates/agent` | ReAct loop, tools, compaction, skills, commands, subagents and goals |
| `crates/model` | Provider protocols and streaming HTTP/SSE transport |
| `crates/harness` | NativeHarness, MockHarness, provider settings and engine adaptation |
| `crates/engine` | Sessions, document hosting, recovery, files, Git, terminals and RPC services |
| `crates/proto` | Shared entities, agent events, requests, capabilities and view derivations |
| `crates/doc` | Session schemas, transcript projection, registry rows and command ledger |
| `crates/sync` | SQLite persistence and chat2/registry transport clients |
| `crates/rpc` | Typed request/response/subscription transport and device relay codec |
| `crates/ui` | GPUI desktop shell, composer, transcript, settings, files, browser and terminals |
| `crates/presentation` | Shared localization resources and display formatting |
| `crates/theme` | Theme families, palettes, imports and surface preferences |
| `crates/mcp` | MCP connections and tool integration |
| `crates/plugins` | Plugin contributions, installation and marketplace catalogs |
| `crates/github` | Public GitHub resource access for marketplaces and updates |
| `crates/preview` | Local HTTP service discovery and preview proxy |
| `crates/client` | Client state, subscriptions, session data and offline demo fixtures |
| `crates/mobile` | Portable mobile rendering, layout APIs and Flutter visual/math C ABI |
| `crates/text` | Text measurement and line layout |
| `crates/syntax` | Syntax highlighting |
| `crates/markdown` | Markdown parsing and layout data |
| `crates/veil` | Streaming text opacity transitions |
| `crates/visual` | Sandboxed Luau visuals and native view models |
| `crates/visual-gpu` | GPU rendering support for visuals |
| `crates/zatex` | LaTeX math parsing and layout; package name `zatex` |
| `crates/update` | Application update discovery, download and installation |

`Cargo.toml` is the authority for workspace membership and dependencies.

## 4. Desktop and presentation

The GPUI UI renders engine projections through typed subscriptions. The sidebar
indexes conversations and projects; open tabs and viewport preferences belong
to the local device. Closing a tab and archiving a conversation are separate
operations.

The transcript uses virtualized rows, cached layout and incremental streaming
updates. Markdown, syntax highlighting, math and Luau visuals have dedicated
Rust components. The composer holds drafts, attachments, model/reasoning
selection and interactive questions. Session statistics include aggregate and
per-turn token usage, cache usage and output speed.

Files provide text editing, Markdown and image previews. The terminal uses an
engine-side PTY and a GPUI terminal view. Changes are presented as file/hunk/line
diffs. Browser tabs and project previews use platform-specific browser support;
project preview discovery currently has macOS and Linux implementations.

Themes resolve light/dark variants, semantic palettes, custom imports and an
independent surface preference. English and Simplified Chinese application text
comes from `rcode-presentation`; protocol values remain separate from labels.

See [themes](docs/theme-system.md), [localization](docs/design/i18n-coverage.md),
[session statistics](docs/session-statistics.md),
[preview networking](docs/preview-networking.md) and [Appshots](docs/appshots.md).

## 5. Agent and engine

`NativeHarness` adapts engine requests to `rcode-agent`, which runs the ReAct
loop over `rcode-model`. `MockHarness` supplies canned events for tests.
Provider configuration supports OpenAI Chat Completions, OpenAI Responses and
Anthropic Messages. The Providers settings page edits the same revision-checked
store read by runs.

The native loop streams model output, executes requested tools, appends their
results and continues until an answer, cancellation or failure. It accepts prior
conversation history and supports context compaction. `AskUserQuestion` uses the
engine/UI interaction path; tools execute directly without a permission-tier
selector.

Skills, custom commands, subagent definitions, hooks, plugin contributions and
MCP tools are assembled for a run. Built-in subagents are `general-purpose` and
`Explore`. Explicit goal commands manage continued work and goal state. The
engine owns durable transcripts, run journals, command execution, terminals,
workspace operations, Git worktrees and recovery.

`AuthStatus` carries the local storage identity through the shared `AuthState`
wire shape. The running engine keeps its resolved identity and store boundary.

Details: [harness](crates/harness/README.md),
[main prompt](docs/main-system-prompt.md), [commands](docs/custom-commands.md),
[completion](docs/harness-skill-completion.md),
[subagents](docs/subagent-prompts.md), [goals](docs/goal-mode-and-agent-tools.md)
and [plugins](docs/plugin-marketplaces.md). Native history restoration,
compaction and tool concurrency are described in
[agent history](docs/agent-history-and-compaction.md).

## 6. Development validation

Use [development validation](docs/development-validation.md) for current commands
and `scripts/ci/` for the checks run by CI. Platform browser, GPU, PTY, capture and
iOS behavior require validation on the corresponding operating system. Source
structure and local mock transport tests do not establish remote-service or
real-provider acceptance.
