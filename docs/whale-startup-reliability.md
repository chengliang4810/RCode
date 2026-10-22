# Large-session persistence and runtime progress

Snapshot export and SQLite writes must not monopolize the engine runtime while
large sessions catch up or stream. The persistence path coordinates snapshot
state, cursor verification and pending local edits.

## Implementation

- Per-chat wake queues coalesce replay changes; acknowledgement and checkpoint
  boundaries request prompt persistence. Snapshot and cursor commit atomically.
- Snapshot jobs await an asynchronous gate before export/write on the blocking
  pool. Synchronous store calls on multithread runtimes use `block_in_place`
  around the mutex wait and operation. Current-thread runtimes retain synchronous
  behavior.
- Desktop supplies a runtime handle to `gpui_tokio`. Large transcript/tail
  materialization and checkpoint preparation use blocking work where needed.
- Verified snapshot cursors avoid unnecessary replay. Missing dependencies and
  actual endpoint resets still invoke recovery.
- Dial, response-header and stalled-body deadlines bound network waits while
  allowing progressing downloads to finish.
- Shutdown cancels transport work and flushes open documents. Persisted cursors
  and pending batches provide the restart boundary.

## Validation

```sh
cargo test --locked -p rcode-sync -p rcode-rpc --features rcode-sync/mock-server
cargo test --locked -p rcode-engine --lib --test restart_resume --test local_profiles --test side_chats
cargo test --locked -p rcode-update
cargo test --locked -p rcode-ui --lib -- --test-threads=1
```

Local tests cover snapshot coalescing, cursor capture, mutex contention, timer
and socket progress, request cancellation and restart behavior. Private snapshot
fixtures require their explicit inputs. These checks do not validate a hosted
checkpoint service or establish production throughput.
