# Workspace registry

The workspace index is a local row table for devices, spaces, chats and session
status. `RegistryDoc` supplies the typed API; `DocsStore` persists its snapshot.
In local mode writes fold directly into authoritative rows and do not accumulate
a pending network queue.

## Sources

- `crates/doc/src/registry.rs`: rows, field clocks, pending operations and merge rules.
- `crates/engine/src/workspace_host.rs`: engine mutations, subscriptions and persistence.
- `crates/sync/src/registry.rs`: optional WebSocket and HTTP transport clients.
- `crates/sync/src/registry/mock_server.rs`: in-process test server, enabled by
  `mock-server`; this is test infrastructure.

## Row semantics

Rows are keyed by kind and ID. Field-level hybrid logical clocks order updates;
strict clock comparisons make repeated operations idempotent. Deletions use
tombstones. An update cannot create a missing row; an upsert can create a row or
revive it when its clock permits.

For a connected replica, acknowledged rows and pending local operations are kept
separately. Reads apply the pending overlay. Repeated unsent operations can be
coalesced when row state and notifications are preserved. Snapshot state includes
the cursor and pending queue so a restart can resume transport work.

## Transport client

The client sends JSON `hello`, `push`, `presence` and `probe` frames. It accepts
`state`, `rows`, `ack`, `presence` and `probe-ok` responses. `state` supports full
and cursor-based replies. The client applies merged rows through `RegistryDoc`,
persists progress, and reconnects with backoff when protocol probes time out.
The HTTP seam supports row fetch and batch push with the same merge semantics.

An endpoint must implement these protocols before remote clients can use it.
This repository does not provide a deployable registry service. Direct desktop
and Flutter clients use the engine RPC listener and do not need that separate
registry endpoint. Mobile pairing requirements are described in the [mobile README](../apps/mobile/README.md).

## Validation

```sh
cargo test --locked -p rcode-doc --lib registry
cargo test --locked -p rcode-sync --lib registry
cargo test --locked -p rcode-engine --test workspace_sync --test local_profiles
```

Local mock-server tests exercise row merge, persistence and transport behavior.
See [current development validation](development-validation.md) for platform checks.
