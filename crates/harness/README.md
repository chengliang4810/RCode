# rcode-harness

The agent interface the engine runs conversations through.

Two drivers implement `Harness`:

- `native.rs` — `NativeHarness` runs the ReAct loop in this process
  (`rcode-agent`) over the provider protocols in `rcode-model`. There is no
  agent CLI to find, launch, or install.
- `mock.rs` — `MockHarness` replays canned events for the engine's tests.

Supporting modules: `providers.rs` and `provider_store.rs` (endpoint/model
configuration and its persisted revisions), `settings.rs`, `shell_env.rs`
(the login-shell PATH), `hooks.rs` (hook matchers), `native_bridge.rs`
(translation between the loop's message model and `rcode-model`'s), and
`windows_process.rs` (job-object process ownership on Windows).

`compose_login_shell_path` is the one thing here that touches a child process:
it is for the tools RCode launches itself (`gh`, terminals), not for agents.

## History

This crate used to drive independently-installed agent CLIs — Claude Code's
stream-json, Codex's app-server JSON-RPC, Cursor's SDK shim, opencode's HTTP/SSE
server, and several ACP agents — plus the installer, executable discovery,
code-signature verification, JSON-RPC framing, catalog caching and output
redaction that went with them. Those drivers were replaced by `NativeHarness`
(decision record: `docs/research/acp.md`), and their modules were removed along
with the subprocess plumbing. What remains is what the native driver and the
engine still call.
