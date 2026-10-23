# RCode local sized-chunks fixes

Based on the crates.io `sized-chunks` 0.6.5 source. Upstream API, layout,
capacity rules, and serialization users remain unchanged. Original code is
licensed under MPL-2.0; see `LICENCE.md` and the source headers.

## Local changes

- `Chunk::force_copy` derives both source and destination pointers from the
  same mutable base pointer. This removes the shared-then-exclusive reborrow
  that invalidated the read pointer during `im::OrdSet` insertion.

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

The upstream version remains 0.6.5 for compatibility. Version-only advisory
tools may continue to report advisories against `im`; the source fix and
regression evidence must remain recorded separately. This does not claim to
resolve the crates' upstream maintenance status.
