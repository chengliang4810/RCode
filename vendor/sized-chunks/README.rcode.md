# RCode local sized-chunks fixes

Based on the crates.io `sized-chunks` 0.6.5 source. Upstream API, layout,
capacity rules, and serialization users remain unchanged. Original code is
licensed under MPL-2.0; see `LICENCE.md` and the source headers.

## Local changes

- `Chunk::force_copy` derives both source and destination pointers from the
  same mutable base pointer. This removes the shared-then-exclusive reborrow
  that invalidated the read pointer during `im::OrdSet` insertion.
- `Chunk` and `InlineArray` remove the dropped range from their live metadata
  before invoking element destructors. `RingBuffer` removes each element via
  `pop_front` or `pop_back` before dropping it. Catching a destructor panic
  therefore leaves only live elements in the container; a later drop cannot
  visit an already dropped value. This fixes all seven affected methods in
  [RUSTSEC-2026-0255](https://rustsec.org/advisories/RUSTSEC-2026-0255.html).
- Remove ineffective `#[must_use]` attributes on trait implementation methods
  that current Rust warns will become hard errors.

The aliasing issue is tracked by
[RUSTSEC-2023-0126](https://rustsec.org/advisories/RUSTSEC-2023-0126.html)
and [the original reproducer](https://github.com/bodil/im-rs/issues/207).

## Regression checks

Run `cargo test --manifest-path vendor/sized-chunks/rcode-tests/Cargo.toml`.
The independent consumer crate applies this patch to the real `im` 15.1.0
dependency, including the original `OrdSet<String>` reproducer and overlapping
chunk moves in both directions. Run the same manifest with `cargo miri test`
when a Miri-capable toolchain is available; ordinary tests alone cannot detect
all aliasing violations.

Panic-safety tests deliberately panic from the first, middle, and last removed
element's destructor, catch the panic, inspect surviving values, and then drop
the container. Every original element must be dropped exactly once. Ring
tests cover wrapped and contiguous live ranges. Bounds tests also verify that
invalid indices leave the container untouched.

Verified with Miri from Rust nightly 2026-10-23 (`1.101.0-nightly`, commit
`7189d39d9`): the original upstream `OrdSet` reproducer fails with an invalid
Stacked Borrows read; the original `Chunk::clear` implementation fails with a
double-free after catching a destructor panic. The patched consumer suite
passes all 8 Miri tests. The 57 upstream unit tests also pass on stable Rust.

The upstream version remains 0.6.5 for compatibility. Version-only advisory
tools may continue to report advisories against `im`; the source fix and
regression evidence must remain recorded separately. This does not claim to
resolve the crates' upstream maintenance status.
