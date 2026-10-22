# rcode-harness

The agent interface the engine runs conversations through.

Two drivers implement `Harness`:

- `native.rs` — `NativeHarness` runs the ReAct loop in this process
  (`rcode-agent`) over the configured provider protocols in `rcode-model`.
- `mock.rs` — `MockHarness` replays canned events for the engine's tests.

Supporting modules: `providers.rs` and `provider_store.rs` (endpoint/model
configuration and its persisted revisions), `settings.rs`, `shell_env.rs`
(the login-shell PATH), `hooks.rs` (hook matchers), `native_bridge.rs`
(translation between the loop's message model and `rcode-model`'s), and
`windows_process.rs` (job-object process ownership on Windows).

`compose_login_shell_path` resolves the login-shell PATH used by host tools
such as `gh` and terminals. Agent execution uses the in-process native driver.
