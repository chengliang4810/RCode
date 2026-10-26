# Windows native streaming stress test — October 25, 2026

An experimental RCode desktop candidate completed two consecutive rounds of
500 concurrent fixed-stream tasks in one process. All 1,000 responses matched
the complete expected content, and all 1,000 streams completed without a
disconnect. These are local client stress-test results, not real-model
inference results or performance guarantees for a downloadable release.

## Environment and workload

- Windows 11 Pro, build 26300; AMD Ryzen 7 5700G, 8 cores/16 logical processors.
- Unmodified production release profile: fat LTO, one codegen unit, optimized
  release code, without allocation counters or diagnostic allocator changes.
- Candidate SHA-256:
  `0B30D24F10CBD7502786351F97C03CADFFDBB96777A7424C887ED90A905EFF36`.
  The change isolates direct-indexed Taffy node payload storage, with a local
  slotmap fork preserving generational keys. It is not installed or adopted
  for release. Complete UI tests pass 1,882, with 5 ignored and no failures.
- Independent synthetic profile with one persisted long-history seed, a
  1,322×881 native window, and default completion sound enabled. The user's
  normal installed application remains separately running and unchanged.
- A local SSE fixture holds all 500 requests at a barrier before releasing
  their streams. Each response contains exactly 17,000 reasoning characters
  and 120 numbered Chinese answer lines, with no tools or subagents.
- Each round verifies 500 simultaneously working application tasks and every
  complete response. Chats are retained for 120 seconds after auditing, then
  the round's 500 test chats are deleted and observed for another 120 seconds.
- No compiler runs during measurement. No working-set trimming, minimization
  or post-load restart is used as a memory-recovery shortcut.

## Candidate results

All memory figures are MiB of the desktop process's private working set.
Peaks are sampled observations, not guarantees that every transient spike was
captured. Fixture servers, measurement scripts, child processes and remote
inference are excluded. Private commit is a different metric.

| Metric | First round | Second round, same process |
|---|---:|---:|
| Concurrent working tasks | 500 | 500 |
| Complete, exact responses | 500/500 | 500/500 |
| Completed streams / disconnects | 500 / 0 | 500 / 0 |
| Stage-start private working set | 63.90 | 93.58 |
| Peak private working set | 835.71 | 807.53 |
| Retained chats after 120 seconds | 188.31 | 193.88 |
| Deleted chats after 120 seconds | 119.14 | 101.38 |

Both rounds use the same candidate executable and process. The second-round
baseline is a warm post-workload state, not a fresh start. Memory is not a
fixed per-task allocation and these values must not be extrapolated linearly.
The 1,000 responses are the total across two 500-task rounds, not 1,000 concurrent
tasks. Deterministic content matching does not measure model quality.

## Memory recovery and comparison

A sequential control/candidate pair uses fresh synthetic profiles, matching
visible seed geometry and editor focus, default sound, and the same fixture.
The control SHA-256 is
`802439FC9FC6EE61BFD54E0DB8F69B3FE729FD8B902709C64D9B8284D244FC53`.

| First-round metric | Original control | Experimental candidate |
|---|---:|---:|
| Warm private working set, MiB | 38.67 | 63.90 |
| Private working set after owned cleanup, before UI actions, MiB | 108.79 | 119.29 |
| After two ordinary Tab navigations, MiB | 103.43 | 91.37 |
| After unsent input and seed-history reopen, MiB | 106.25 | 93.17 |
| Parent creation-to-completion CPU, core-seconds | 325.16 | 334.25 |
| Parent creation-to-completion elapsed time, seconds | 108.16 | 106.02 |

The candidate's observed post-navigation memory is 12.06 MiB lower, but its
quiet memory is 10.50 MiB higher and parent CPU is 2.80% higher. Warm baselines
already differ 25.23 MiB. This single sequential pair does not establish a
stable overall improvement or isolate causal CPU overhead. Completion-sound
PowerShell activity also varies substantially; captured child counters are
sampled lower bounds over a different interval and are not added to parent CPU.

The user browses settings during the second candidate round. After returning
to the seed and ordinary focus/navigation, memory is 102.70 MiB, not a repeat of
the first 91.37 MiB checkpoint. This interaction remains a recorded confound;
the second run is not a matched repeated idle benchmark. Actual input,
preserved draft, seed-history return and full persisted content checks pass.

The candidate remains experimental and is not adopted: quiet recovery and
native CPU reduction have not been demonstrated. There is no claim of a
stable post-load footprint below 50 MiB. All owned fixture services, providers
and test chats are cleaned, and diagnostic applications close normally only
after their measurements. Normal application files and user data are unchanged.

## Provenance

The 2,237 frozen build inputs have zero hash drift. Local timestamped build
receipts, raw Cargo output, per-round manifests, memory/CPU samples, complete
response audits and cleanup receipts are retained under
`output/memory-audit/slot-payload-probe/`. That local audit directory is not
included in the published report. No credentials or private chat transcripts
are included here. The separate
[October 24 real-model benchmark](2026-10-24-windows-concurrency.md) uses a
different executable and workload; its numbers are not replaced by this test.
