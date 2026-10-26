# RCode local Taffy lifecycle fix

Based on the crates.io `taffy` 0.12.2 source, upstream commit
`bb351fcc056c93dbb967292acc4242a5d1c46b3c`. The upstream MIT license is
preserved in `LICENSE.md`. Package version, public API, features, layout
algorithms and reusable node-storage policy remain unchanged.

`TaffyTree::clear` now clears node contexts as well as nodes, parents and
children. `TaffyTree::remove` also removes the removed node's context. The
original methods left measurement closures and everything captured by them
alive after the associated node had been removed. GPUI clears its Taffy tree
between frames, so obsolete closures must not remain resident until their
slots happen to be reused or the window is destroyed.

The fix changes resource lifetime, not layout: live contexts remain available
to measurement, and removing a parent still preserves its detached children.
The context-lifetime fix alone does not shrink layout-node storage or GPUI's
element arena.

## Rejected dense-storage experiment

A separate `DenseSlotMap` experiment reclaimed payload capacity after128low
usage clears while preserving generations. It passed layout and stale-key
checks, but installed500-stream acceptance did not establish an overall
resource improvement: deletion-plus-idle improved, retained-chat working set
did not, and CPU/peak observations varied. It is not part of the current
source. The ordinary `SlotMap` implementation and context-lifetime fix remain.
See `docs/native-session-memory.md` for scoped measurements and limitations.

## Regression checks

```sh
cargo test --offline --manifest-path vendor/taffy/Cargo.toml --lib
cargo test --offline --manifest-path vendor/taffy/rcode-tests/Cargo.toml
cargo test --offline --manifest-path vendor/taffy/rcode-tests/Cargo.toml --no-default-features
```

The consumer suite checks immediate context release on clear/remove, frame
slots not reused after a large-to-small workload, detached child lifetime, and
measurement after reuse. Before the source fix, four lifetime tests fail and
the measurement test passes. A separate allocation probe with 64 owned 64 KiB
payloads retains all 4 MiB after the original clear and zero payload bytes
after the fix. This is a controlled reproduction, not a measurement of how
many bytes the installed RCode application's closures retain.
Payload ownership falling to zero is not itself a measured OS working-set reduction.

The 96 packaged upstream unit tests and both five-test consumer configurations
passed on Windows on October 24, 2026. The installed Windows comparison and its
limitations are recorded in `docs/native-session-memory.md` in the repository
root; these tests alone do not establish those resource measurements.
