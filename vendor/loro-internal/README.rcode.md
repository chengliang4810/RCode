# RCode snapshot allocation fix

Based on crates.io `loro-internal`1.13.9, upstream commit
`1e529a6841b35572b05c3cb1258c0862b3aaef40`. The upstream MIT license is
included in `LICENSE`. Package version, public API, snapshot encoding and
operation history are unchanged.

`ChangesBlock::to_bytes` keeps parsed changes alongside their encoded bytes.
Previously serialization discarded the parsed changes, so transaction renewal
or the next append decoded existing text again into the append-only string
arena. Keeping both representations avoids duplicating already allocated text.
`changes_mut` still invalidates encoded bytes before mutation; subsequent
exports therefore serialize the latest operations rather than stale bytes.

Two internal regressions verify unchanged arena length during repeated exports
and continued Unicode editing with per-step export/import. The original fails
both regressions; the candidate passes. Application-level history, streaming,
reopening and concurrent-peer behavior are tested in `rcode-doc`.

The test-only `dev-utils` helper is vendored separately as `../loro-dev-utils`
from the same upstream commit, without stubs. It is not an application runtime
dependency. The standalone workspace permits running the upstream library
tests without adding its test-only dependencies to the application's workspace.

```sh
cargo test --manifest-path vendor/loro-internal/Cargo.toml --lib
cargo test -p rcode-doc --lib
```

The isolated500-document fixture substantially reduces active allocation, but
allocator-retained pages remain after documents drop. Installed application
CPU, memory and native recovery acceptance are still required before adoption.
No heap trimming, history truncation or snapshot-format migration is enabled.
