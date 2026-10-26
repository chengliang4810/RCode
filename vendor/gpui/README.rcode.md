# RCode local GPUI arena capacity policy

Based on the unchanged GPUI 0.2.2 sources at Zeron ZUI commit
`361ff20b919fdfb80199ee6d9aed725300cb7d54`. The original Apache license is
preserved in `LICENSE-APACHE`. The standalone manifest resolves inherited
workspace dependencies to their original declarations and pins sibling crates
to the same Git revision.

The only upstream Rust source change is in `src/arena.rs`. After clearing every
element and invalidating its handles, an arena larger than four chunks can shed
excess chunks when frame demand stays at half its capacity or below for at least
30 seconds. It retains the recent window's maximum demand rounded up to a power
of two, with a minimum of two chunks. Renewed high demand resets the observation
window, and ordinary allocation regrows the pool when needed. Element metadata
and the small chunk-descriptor vector keep their existing capacities.

The monotonic clock is read only for oversized pools whose current demand is
eligible for decay. Ordinary small/high-demand clears do not sample it. The
same grace-period, destruction and capacity-retention regressions cover this
lazy-clock path.

The policy does not add a timer, thread, wakeup, working-set eviction or global
heap-trimming call. It runs on a later ordinary clear; an inactive/minimized
window without another frame does not release its arena merely because time
passed. Live objects are not moved or released. Existing alignment, destructor
and invalid-handle behavior is preserved.

An isolated Windows release probe using the same segment heap and a 16 MiB
arena demonstrated 14 MiB of commit release after a real 31-second low-demand
interval and the next frame. Returning to a large frame requires reallocating
discarded chunks, so it can increase the first resumed frame's allocation cost.
Probe results are not installed-app savings or UI-latency acceptance. Actual
RCode release delivery and workload verification are still required.

## Focused regressions

```sh
cargo test --offline --release --manifest-path vendor/gpui/rcode-tests/Cargo.toml
```

The suite compiles the actual patched arena source directly. Four original arena
tests and six decay-policy tests cover alignment, growth, invalid handles,
destructor order, the grace period, renewed demand, recent-peak retention and
regrowth. These ten tests pass; they do not replace installed-app acceptance.
