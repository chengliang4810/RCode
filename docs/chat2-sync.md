# chat2 session transport

Session documents persist locally in Loro. The Rust chat2 components implement
checkpoint and row transport when an endpoint is configured; the repository
contains client and test infrastructure, not a deployable session service.

## Implementation

| Source | Responsibility |
| --- | --- |
| `crates/sync/src/chat_frames.rs` | Binary frame codec and typed headers |
| `crates/sync/src/chat_client.rs` | Connection, catch-up, row push/ack and reconnection |
| `crates/sync/src/store.rs` | SQLite snapshots, cursors, batches and progress |
| `crates/engine/src/chat2_host.rs` | Engine adapters for checkpoint and transport operations |
| `crates/engine/src/doc_host.rs` | Document admission, persistence, publication and lifecycle |
| `crates/doc/src/rebuild.rs` | Thin-document rebuild and document epoch handling |
| `crates/doc/src/parts.rs` | Bounded document representation of tool payloads |

## Frames and progress

Binary WebSocket frames have this layout:

```text
[type: u8][header length: u32 little endian][JSON header][payload bytes]
```

The header is an object of at most 4096 bytes. The codec defines `HELLO`, `STATE`,
`ROWS_REQ`, `ROW`, `ROWS_DONE`, `PUSH`, `ACK`, `PRESENCE`, `PROBE`, `PROBE_OK` and
`ERROR`. Unknown kinds can be skipped by the client. Payloads carry opaque Loro
updates, checkpoint data or ephemeral presence data.

The client tracks sequence progress and local batch IDs. Acknowledgements allow
persisted pending batches to drain; reconnect and catch-up use the stored cursor.
Checkpoint fetches, document epochs and thin rebuilds are coordinated with the
engine's persistence path. The command ledger and run journals remain local
execution responsibilities.

## Validation

The source codec tests and local mock transport tests define the implemented
contract. They do not verify an externally hosted endpoint.

```sh
cargo test --locked -p rcode-sync --lib
cargo test --locked -p rcode-doc --lib
cargo test --locked -p rcode-engine --test born_chat2_race --test restart_resume
```

See [registry transport](registry-sync.md),
[synchronization resource bounds](sync-resource-resilience.md) and
[current development validation](development-validation.md).
