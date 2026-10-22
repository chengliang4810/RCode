# Durable session publication

`DocsStore` journals local Loro updates before network admission. Pending batches
and progress persist independently of transport-client lifetime, so document
cache eviction or a disconnected endpoint does not discard accepted work.

## Persistence and replay

- Stable batch IDs make replay idempotent when an acknowledgement is lost.
- Opening a document imports pending updates before attaching writers, including
  updates newer than the last debounced snapshot.
- Local-update subscriptions remain active through agent cleanup. Cold-open
  initialization completes before exposing the writable document handle.
- A failed disk write retains the in-memory copy and prevents clean eviction.
- Bootstrap records saved operation history transactionally without changing
  operation IDs, command IDs, document epochs or download cursors.
- Oversized or rejected batches remain checkpoint obligations. Retry continues
  after failed requests; an obligation retires only when covered by a successful
  snapshot upload. Frontiers are decoded from the captured snapshot bytes.

These are engine/client responsibilities. Using remote publication requires an
endpoint implementing the [chat2 transport](chat2-sync.md). Local persistence
does not depend on that endpoint being available.

## Validation

```sh
cargo test --locked -p rcode-sync --lib
cargo test --locked -p rcode-engine --lib --test restart_resume --test local_profiles --test born_chat2_race --test side_chats --test message_queue --test transcript_salvage
```

The local suites cover outbox persistence, restart/deduplication, failed writes,
LRU eviction, disconnected cleanup and loopback transport. Lost acknowledgements
and failed checkpoint requests are injected by test servers. Private snapshot
fixtures require their explicit inputs. These tests do not establish the
persistence policy or availability of a separately hosted service.
