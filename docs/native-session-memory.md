# Native session memory

Status: installed Windows release validated on October 24, 2026; larger-scale
validation and further resource profiling continue.

Completed native agent turns previously kept their runtime, in-memory history and
document writer alive until the existing 30-minute idle reaper ran. Hundreds of
completed conversations could therefore remain resident together.

For native sessions with a data directory and a conversation identity, the harness
now shares twelve warm idle slots across its clones. A completed session without
background work or an active goal keeps a slot if one is available. Otherwise its
runtime ends normally and releases the document writer. The conversation remains
stored; the next message restores its execution history through the existing
native history recorder. Normal document persistence and cache eviction remain
responsible for closing and reopening documents.

The slots do not limit running model tasks. A warm session returns its slot before
starting another user turn, goal continuation or background-notification turn.
Active goals, running tasks, pending background notifications and the runtime's
background-work report protect the session from capacity retirement. Goal-read
errors also retain the runtime. Sessions without durable host context keep their
previous behavior.

The new capacity-retirement path also excludes runtimes that have encountered a
failed preparation, an interrupted turn or an errored turn. This protection is
latched for that runtime, including after a later successful turn: completed
tool results may have survived a storage failure only in memory. A subsequent
successful response does not retroactively persist those missing records.
Explicit host shutdown and the existing lifecycle rules are unchanged. In
particular, the engine's pre-existing error teardown still applies; this guard
does not make errored engine sessions persistent.

This is a capacity bound, not an LRU: the first idle sessions to acquire available
slots remain warm until used again, closed or reaped by the existing idle policy.
A retired session pays the cost of restoring its persisted history on its next
turn. That latency is not yet independently bounded for large histories.

Regression coverage fills all twelve slots before checking excess-runtime
retirement, complete tool-history recovery, immediate follow-up handoffs,
background child completion and goal iterations. Protocol fixtures exercise real
EngineCore and NativeHarness paths locally. The recovery test also waits for a
weak document handle to expire, then verifies that reopening increments the disk
load count and preserves the completed message. This checks actual eviction,
rather than just the disappearance of a running-session status.

The recovery/handoff tests, background-completion test and goal-round test
passed on Windows on October 24, 2026. A direct-runtime fault fixture also
exercises a real Bash tool result whose history append fails, repairs the fixture
storage, then checks two further turns retain that result. Disabling the new
failure guard makes that fixture fail because capacity retirement ends the
runtime. The direct-runtime boundary is intentional: engine error teardown is a
separate, unchanged lifecycle. Queue and attachment regressions passed
33 tests. A separate rich-composer queue-edit test failed while indexing an empty
queue; it uses a RecordingHarness instead of NativeHarness and does not exercise
the new idle-cap policy. That failure is not counted as a passing validation.

## Installed Windows measurement

The installed `D:\dev\RCode\rcode.exe` was replaced normally with the tested
release, SHA-256
`D23E41A764B4B9DB2B3DD99D165BC61879BBB6E03786415540492B168C8B6705`.
The previous installed release was
`A8E56A049E3A1C19B9E4A423E026F01216D407E0A6FE5DC4F6C977C95A6455F5`.
Both versions processed the same local synthetic stream through their actual
native-agent path: 100 tasks, 1,000 reasoning chunks (17,000 characters) and 120
numbered answer lines per task. All 100 answers passed those checks in both runs.

| Private working set | Previous release | New release |
|---|---:|---:|
| Before workload | 73.04 MiB | 45.04 MiB |
| Sampled workload peak | 563.93 MiB | 558.14 MiB |
| Completed, retained chats, idle 120 seconds | 493.84 MiB | 134.39 MiB |
| Deleted fixture chats, idle 120 seconds | 78.88 MiB | 75.76 MiB |

Before chat deletion, idle document writers and open documents decreased from
100 to 12. The raw retained-chat measurement decreased by 359.45 MiB (72.8%).
The increase over each run's own starting point decreased from 420.80 MiB to
89.36 MiB, a 331.44 MiB difference. Starting points differ because the previous
process had already handled earlier workloads; this is one run per version,
not an isolated causal estimate or a cross-machine guarantee. Release
compilation also overlapped the previous run, so these runs do not establish a
CPU-performance improvement. Neither working set was forcibly emptied.

A separate installed-app history fixture filled all twelve warm slots, executed
one real Read-tool turn, then sent ten follow-ups to the excess conversation.
Every follow-up reopened its document from storage, preserved the full tool
result tail and all earlier answers, and returned to twelve idle writers.
Queue-to-first-model-request median was 106.26 ms (maximum 184.35 ms), versus
88.97 ms for the previous warm runtime. These timings include RPC, queue and
model preparation overhead; compilation overlapped the previous run. They do
not isolate restoration cost and do not bound very large histories.

Local evidence is retained under `output/memory-audit/idle-cap-experiment/`,
including manifests, memory samples, answer checks, resource counts, binary
hashes and resume-fixture results. The synthetic workload is distinct from the
real-provider benchmark advertised in the README; it does not replace or update
those published real-model figures.

## Larger local workload

A sequentially submitted 500-task fixture completed all 500 content checks and
returned to twelve idle documents/writers, but did not reach 500 simultaneous
working tasks. Its original concurrency gate failed and remains recorded as
failed; its sampled peak is not advertised as a 500-concurrent result.

A separate local barrier fixture then held requests until all 500 arrived before
starting their fixed streams. Both the application's working count and the
fixture's active streams reached 500; all fixture streams ended without a
disconnect. The original runner's 180-second deadline fired after observing
498 completed tasks, so that runner did not pass. A later independent audit
verified normal completion markers and complete persisted assistant messages
for all 500 tasks, with the full 17,000 reasoning characters and 120 answer lines
in every message and no error parts. The original timeout and cleanup-issued
interrupt commands are retained in the audit rather than erased or relabeled.

After that complete-content audit and at least 120 seconds of observation,
private working set was 234.25 MiB with twelve documents/writers retained.
This is not a matched before/after comparison at 500 tasks. Sampling stopped at
the original deadline, so its 959.45 MiB sampled maximum is not claimed as a
guaranteed entire-run peak. The result validates the idle-document capacity at
larger scale, not the original 180-second timing gate or remote-provider quality.

After deleting the barrier fixture chats and waiting another 120 seconds,
private working set was 163.12 MiB with zero open documents and zero writers.
Temporary providers and all local fixture services were removed/stopped; all
six original conversations remained. The process was not restarted between its
100-task and two 500-task workloads. This post-stress footprint remains above
its 45.04 MiB workload starting point: document-runtime retirement is verified,
but attribution and reduction of the remaining process memory are still open.
The final 30.84-second frontmost, focused-composer idle sample used 0.139% of
the 16-logical-processor machine. This is an observed post-load CPU level, not a
claim of improvement over the differently focused previous-version sample.

## Follow-up: Layout Context Lifetime

Read-only examination of the same installed post-load process identified reusable
GPUI element and Taffy layout storage. A main arena retained sixteen 1 MiB chunks
with zero live elements; its private resident chunk storage was 15.61 MiB. A layout
slot vector retained 18.33 MiB capacity, of which 11.88 MiB was resident. These
are retained capacities, not proof that all those bytes can be reclaimed without
an allocation/CPU tradeoff.

A separate source-level defect was reproduced in the pinned Taffy 0.12.2:
`TaffyTree::clear` and `remove` did not release the removed nodes' measurement
contexts. The local patch now clears/removes that context storage alongside the
nodes, without changing live-node measurement, layout algorithms or pool capacity.
All 96 packaged upstream tests and both five-test lifecycle configurations pass.
A controlled 64-payload probe releases 4 MiB that the original clear retained;
this is not a measurement of the installed application's gain.

The fixed release was installed at `D:\dev\RCode\rcode.exe`, SHA-256
`375B1ED3CB00C333F8346A80D45B68188EA7AFF07147FD0D0AA4936D63C1723E`.
The 804 recorded build inputs remained unchanged during its successful release
build. A further 35 GPUI sidebar regressions passed. Native-window startup,
Chinese text, TODO display, navigation to a blank conversation and editor focus
were checked on the installed executable.

### Matched Installed 500-Task Workload

Both the previous D23E41 release and the new release started in fresh processes,
opened the same original conversation, then switched to a focused blank composer
in the same 1012-by-689 logical-pixel window. Neither workload overlapped a Rust
build. A local fixture held model requests until all 500 arrived, then emitted
the same fixed reasoning and answer chunks. Both runs reached 500 working tasks,
completed all 500 output-length/120-numbered-line checks, and retained twelve idle
documents/writers before fixture chat deletion.

| Private working set | D23E41 | 375B1E |
|---|---:|---:|
| Before workload | 46.59 MiB | 47.21 MiB |
| Sampled workload peak | 826.84 MiB | 735.77 MiB |
| Completed, retained chats, idle 120 seconds | 203.44 MiB | 172.23 MiB |
| Deleted fixture chats, idle 120 seconds | 151.25 MiB | 113.36 MiB |

The raw sampled peak decreased by 91.07 MiB (11.0%), and the post-deletion
measurement decreased by 37.89 MiB (25.0%). The post-deletion increase over each
run's own starting point decreased from 104.66 to 66.15 MiB. This is one paired
measurement, not a cross-machine guarantee or an exact accounting of live closure
bytes. The native allocator's retention and the persistent registry also affect
process memory.

CPU and latency do not establish a uniform speedup. In the observed interval
from the first sample after queue submission to all tasks becoming idle, process
CPU time decreased from 392.00 to 286.45 seconds, while wall time increased from
135.95 to 143.02 seconds. Across the broader baseline-to-idle interval including
chat creation, CPU time was 562.98 versus 514.64 seconds and wall time was 202.92
versus 231.87 seconds. The stored registry accumulated fixture tombstones between
runs, and these intervals are not isolated provider-generation or UI-latency
measurements. End-to-end runner times of 442.84 and 477.30 seconds also include
answer checks, deletion and 240 seconds of fixed idle observation.

Foreground post-load idle CPU observations were about 0.136% for D23E41 and
0.168–0.216% for the new release on the 16-logical-processor machine. Pointer
position differed, so these are observations rather than a causal CPU comparison;
an additional root-focused sample is explicitly excluded. An idle CPU reduction
has not been established and remains part of the optimization work.

All six original chats and the original provider remain. Both temporary providers
and fixture services were removed. The new process remains running after its
500-task workload, with zero open documents/writers and roughly 113–114 MiB
private working set; it was not restarted after the workload and its working set
was not forcibly emptied. Remaining allocator and rendering-pool retention is
still open. Full local evidence is under
`output/memory-audit/residual-audit/matched-500/`. This synthetic comparison does
not update the real-model benchmark advertised in the README.

### Windows Heap Cache Reclamation Candidate

An isolated release-mode process using the same Windows segment-heap signature
tested `HeapSetInformation(NULL, HeapOptimizeResources, ...)`. With 24 allocation
workers still alive but idle and 96 surviving buffers verified, three trials
released 6.12, 4.91 and 5.59 MiB of private commit. The calls took 0.99, 0.44 and
0.48 milliseconds. Three untreated controls released zero over the corresponding
operation interval. These are standalone-probe savings, not additional RCode
savings; the installed application has not called this API.

A separate concurrent-allocation test ran 24 workers and 100 optimization calls
per trial. All three optimized release trials and three controls completed their
payload checks; every operation overlapped live allocation workers. Optimization
call medians were 0.56–0.63 milliseconds, but per-trial 95th-percentile observations
were 5.70–39.23 milliseconds and maxima were 277.68–346.54 milliseconds. This
deliberately aggressive test calls the API roughly every 10 milliseconds plus
validation/call overhead, rather than once per minute. Its wall times include OS
scheduling under CPU pressure; they do not prove the UI would pause for that
duration or establish the behavior of a lower-frequency policy.

An unconditional periodic Windows heap trimmer is therefore not enabled. It has
not yet demonstrated a no-regression tradeoff in RCode. A future candidate needs
idle/transition gating, actual installed-app memory measurements, and allocator,
resume and interaction latency checks. Existing production behavior, the release
executable and the real-model README benchmark remain unchanged by this probe.
Local raw evidence is in `output/memory-audit/residual-audit/heap-cache-*`.

### Render Arena Capacity Decay Candidate

A separate source-owned reclamation candidate changes only GPUI's arena clear
policy. Pools larger than four chunks observe sustained demand at half capacity
or below for 30 seconds, then release excess chunks on an ordinary later clear.
The retained capacity covers the recent observation window's maximum demand,
rounded up geometrically, with at least two chunks. A renewed large frame resets
the grace period. This avoids global heap trimming, additional threads, timers,
forced paging and idle wakeups. With no further frame, no reclamation occurs.

Three original/candidate Windows release probe pairs used the same 16 MiB arena
and a real 31-second low-demand interval. Each candidate released 14 MiB of arena
capacity and 14.02 MiB of private commit on the next frame; each original retained
its allocation.
Returning to the 16 MiB workload regrew the pool successfully. First resumed
burst times were 1.29–2.65 ms for the original and 2.22–2.60 ms for the candidate.
The 2,000-small-frame intervals were 8.15–9.96 ms and 7.97–12.05 ms respectively;
these short wall-time observations do not establish a CPU improvement or absence
of a regression. Reallocation after a quiet period is an explicit tradeoff.

Four original arena regressions and six policy regressions pass against the
actual vendored source. A further identity probe confirms that replacing an
empty Taffy tree with a new tree can make an obsolete node ID valid again;
that shortcut is not used. The Taffy allocation policy remains unchanged.

GPUI is pinned to its existing ZUI revision and locally vendored for the arena
policy. Only `src/arena.rs` differs among its original Rust sources, and the
resolved Windows dependency/feature graph is unchanged apart from the GPUI source
location. The isolated probe measurements are not themselves installed-app
savings. The subsequent installed comparison is recorded below.

### Installed Arena Decay Comparison

The arena policy release was built successfully in 26 minutes 58 seconds and
installed at `D:\dev\RCode\rcode.exe`, SHA-256
`4642D8936AD2E1E86B18E39F56A6BA9FF26BEE5753BE20E536CC9CA45B2140E8`.
All 958 recorded inputs remained unchanged through build and installation, and
the Windows product/display metadata remains `RCode`. The previous 375B1E binary
was backed up before replacement.

Both sides of this comparison used fresh installed processes, the same original
chat for warm-up, a focused blank composer in a 1012-by-689 logical-pixel window,
a 60-second pre-load wait, and the same 500-request local barrier fixture. No
Rust compilation overlapped either workload. Both reached 500 working tasks and
passed all 500 output checks: 17,000 reasoning characters, 120 numbered answer
lines and no error/other parts per task. All 500 fixture streams completed with
zero disconnects. Each stage retained its chats for 120 seconds, then deleted
only its own chats and observed another 120 seconds.

| Private working set | 375B1E | 4642D8 |
|---|---:|---:|
| Before workload | 46.99 MiB | 48.52 MiB |
| Sampled peak | 777.29 MiB | 754.66 MiB |
| Completed, retained chats, idle 120 seconds | 197.48 MiB | 169.78 MiB |
| Deleted fixture chats, idle 120 seconds | 120.71 MiB | 104.82 MiB |

The observed peak decreased by 22.64 MiB, retained-chat usage by 27.71 MiB, and
post-deletion usage by 15.89 MiB (13.16%). Post-deletion increases over each
process's own starting point were 73.71 and 56.30 MiB. This is a new paired run;
do not subtract its results from the earlier Taffy-fix pair or treat every byte
of the difference as an exact accounting of arena chunks.

CPU and elapsed time did not improve in this pair:

| Interval | CPU seconds, before / after | Wall seconds, before / after |
|---|---:|---:|
| Chat creation | 52.02 / 55.67 | 23.61 / 25.90 |
| Queue submission | 156.89 / 163.16 | 60.53 / 61.69 |
| After submission to all tasks idle | 376.42 / 386.81 | 127.95 / 135.57 |
| Total of these intervals | 585.33 / 605.64 | 212.08 / 223.16 |

CPU time increased by about 3.47% and wall time by about 5.22% across those
intervals. The full runners took 451.26 and 466.70 seconds, also including
content auditing, deletion and 240 seconds of fixed observation. These are not
isolated provider-generation or UI-frame timings. Registry tombstones accumulate
between stages, and one pair does not prove the cause of the difference. No CPU
speedup or no-regression claim is made; further profiling/repeats remain open.
Post-deletion 90.54-second idle intervals used about 0.162% and 0.206% of the
16-logical-processor machine, respectively.

Native smoke checks on the installed new process covered Chinese typing without
sending a model request, restoring the draft to empty, reopening the original
long transcript/TODO after decay, maximizing/restoring, minimizing/reactivating,
and returning to a focused blank composer. A screenshot/accessibility roundtrip
is not a measurement of first-frame latency, so it is not reported as one.
All six original chats and the original provider remain, and both temporary
providers/services were removed. The candidate was not restarted after its
500-task workload and no forced working-set operation was used. Foreground
memory comparisons were captured before the later minimize/restore smoke test;
normal OS minimize behavior is not attributed to this source change.

After the additional UI checks and a further two-minute settling period, the
same process measured 109.80 MiB private working set and 135.75 MiB private
commit. One open but unpinned document remained after revisiting the original
chat; view/writer/connection and pending-durable counts were zero. A subsequent
60-second focused-composer sample used about 0.316% of the machine. Its pointer
position and preceding UI workload differ from the paired samples, so it is a
separate observation, not a CPU improvement or a revised paired memory result.

Full evidence is in `output/memory-audit/residual-audit/arena-matched-500/`.
This synthetic installed-client comparison does not change the real-provider
README benchmark. Remaining post-stress memory, CPU attribution and recovery
latency measurements keep the optimization work open.

### CPU Follow-up: Journal Handle Churn

Source inspection identified a separate pre-existing cost in the run journal:
the cache retained at most sixteen open journal files, clearing the cache on
overflow. Interleaving more than sixteen active chats repeatedly reopened their
files and reconstructed the sequence from the tail. The reverse reader starts
with a 64 KiB chunk; it does not reread the entire journal. This behavior existed
on both sides of the arena comparison, so it is not established as the cause of
that pair's 3.47% CPU increase.

A standalone Windows release benchmark interleaved 1,000 reasoning events per
chat, then a terminal event, and verified every replayed sequence and payload.
The timed phase covers reasoning appends; terminal closing, full replay checking
and temporary-fixture cleanup occur outside that interval. Three alternating-order
pairs tested 10, 100 and 500 journals. The candidate
allows up to 512 cached journal handles on Windows and closes a chat's handle
immediately after successfully writing its outer terminal `Done`. Other
platforms retain the previous sixteen-handle bound; nested subagent `Done`
events do not close the parent journal. The bound is a cache budget, not a limit
on the number of running tasks. Overflow still evicts and recovers from disk.

| Interleaved journals | Original append CPU seconds | Candidate append CPU seconds |
|---|---:|---:|
| 10 | 0.016–0.031 | 0.016–0.031 |
| 100 | 8.42–10.61 | 0.234–0.250 |
| 500 | 42.39–57.25 | 1.73–2.11 |

For 500 journals, each trial replay-verified 500,500 events. Active process
handles were 97 for the original versus 581 for the candidate, including the
same 81-handle process baseline: 484 additional active handles. After terminal
events the candidate returned to 81, while the original retained 85. Candidate
private commit at the active checkpoint was only 8–16 KiB above the paired
original, but kernel memory for the additional file objects was not measured.
These component results are not the installed application's overall CPU saving.

A separate short resource-only fixture measured 72–96 KiB more active private
commit for the candidate. Resource differences therefore depend on the workload,
not a fixed per-process allowance. Native paged/nonpaged quota-pool counters
were identical despite the extra 484 handles; those counters do not establish
the total file-object/driver kernel-memory cost, and no zero-kernel-cost claim
is made. This short fixture's timings are excluded because the production
release compiler was active.

The actual engine passes sixteen focused journal regressions, including terminal
statuses, concurrent sequence assignment, 513-chat cache overflow on Windows,
subagent boundaries and tail recovery. Five native integration tests also pass
for background-child completion/wake, durable goal rounds and idle/history
recovery. An independent consumer compiles the actual journal source and passes
its sixteen tests alongside eleven original-source controls.

Fixed-work arena probes also identified avoidable clock sampling on ordinary
clears. The lazy-clock implementation passes all ten actual-source arena tests
and fifteen probe tests; a real 31-second low-demand check still releases
14 MiB of arena capacity and regrows on demand. Four alternating-order CPU pairs
per workload completed, but their microbenchmark values do not establish the
cause of the installed CPU difference or an application-level latency bound.

The release build completed successfully with 958 unchanged recorded inputs.
On October 24 at 18:43 local time, the guarded installer replaced the actual
`D:\dev\RCode\rcode.exe` with candidate 61BE42 and retained the 4642D8 backup.
Both installed 500-task fixtures passed full-output checks and cleanup preserved
the six original chats and provider. In this single sequential pair, CPU from
creation through completion fell from 538.78 to 467.70 seconds (13.19%), and
elapsed time fell from 245.28 to 178.61 seconds (27.18%). However, sampled peak
private working set rose from 833.46 to 1,257.97 MiB, and peak private commit
rose from 892.49 to 1,240.56 MiB. Peak thread counts were 181 and 385; this
correlation does not establish which allocations caused the transient increase.
After deletion plus 120 seconds, private working set was 106.40 versus
108.54 MiB, not a residual-memory improvement. All tasks, documents, writers,
connections and durable pending work were zero after fixture cleanup.

This candidate is installed but is not accepted as an overall resource
improvement. The large transient-memory regression requires repeat measurements
and attribution; registry tombstones also accumulate between sequential stages.
Performance-counter working set and process commit are sampled non-atomically,
so same-row values can cross; the independent commit peak nevertheless confirms
substantial transient allocation growth. No working-set eviction or post-load
restart was used. These local-fixture results do not replace real-provider
README benchmarks. Raw evidence is in
`output/memory-audit/residual-audit/journal-cpu-*` and `arena-*-clock-*`.
The installed paired evidence is in
`output/memory-audit/residual-audit/cpu-matched-500/comparison.json`.

### Repeat Load and Dense Layout Storage Candidate

A second 500-task fixture ran in the same installed 61BE42 process without a
restart. All 500 full outputs passed. Private working set started at 108.29 MiB,
peaked at 1,816.70 MiB and settled to 117.37 MiB after deletion plus 120 seconds;
peak private commit was 1,867.57 MiB. Cleanup preserved the six original chats
and provider, with zero open documents, writers, connections or pending durable
work. This instrumented repeat is not a fresh-start timing comparison.

Read-only thread descriptions identify separate bursts: Tokio workers grew
from 29 to 80, while a later 550-thread sample contained 433 unnamed threads.
At the largest observer commit sample, the process had only 147 threads.
Thread expansion therefore cannot by itself account for the measured peak,
and the unnamed threads have not yet been attributed to a specific caller.
The observer neither suspended the process nor injected code. No compiler ran
during the installed repeat. Evidence is in
`output/memory-audit/residual-audit/cpu-repeat-500/`.

A separate Taffy candidate replaces the three primary slot maps with dense
value storage. After 128 consecutive low-demand clears, it clones the empty
maps to release payload capacity while preserving their small generational
slot tables and free-list state. It does not replace the tree with a new tree,
which could resurrect stale IDs. The threshold counts clears, not seconds;
renewed high demand resets it, and subsequent growth reallocates payloads.

The actual vendored candidate passes 99 unit tests and both five-test context
consumer configurations. A separate alloc-only external consumer also verifies
clear/reclaim/remove/full-regrowth and parent-child consistency. Original and
candidate standalone source snapshots are pinned; the original tree hash is
`152EB83198FB4DB75A2A26B200A0BF5824002EB677D4B732AAC4DC51958AA6C2`.

The 13,000-node component fixture reduces requested live allocations by
18.96 MiB after the low-demand window. In three separate Windows processes,
candidate private commit falls by 18.27 MiB while original controls release
zero over the same phase. Three uncontended alternating-order long fixture
pairs show a 12.02% increase in process CPU; a fourth pair overlapped a compiler
and is excluded. No application memory or CPU gain is inferred from this.
Release819E6B was installed at19:51 on October24 after a guarded backup of
61BE42 and verification of958 unchanged build inputs. Both first-pair stages
verified500full outputs and preserved original chats/provider. Private working
set after deletion plus120seconds fell113.375 ->99.316MiB, with peak1723.063
->1351.523MiB. However, creation-to-completion CPU rose345.563 ->475.359seconds
(37.56%), with wall time120.162 ->179.134seconds (49.08%). Most CPU difference
was submission83.984 ->203.063seconds; creation was64.688 ->65.375seconds.
This single sequential pair is not sufficient for causal attribution, and a
fresh reverse-order dense-then-baseline confirmation is running before adoption.
Native Chinese typing/undo, long-chat/TODO restoration and window resize/restore
passed after the load. Normal minimize testing occurred after frozen resource
comparison and is not counted as a source memory reduction. The candidate is
not accepted as an overall resource improvement. Raw evidence is in
`output/memory-audit/residual-audit/taffy-dense-probe/`.

The reverse-order confirmation also verifies500full outputs on each side.
Baseline61BE42 now uses534.063CPU seconds versus dense484.938seconds, reversing
the whole-run direction (dense9.20%lower). Post-deletion working set remains
lower with dense:111.723 ->101.363MiB. Peak direction also reverses,954.148
->1070.059MiB. Across the two pairs, raw post-deletion savings are10.36–14.06MiB,
but neither a stable peak saving nor a fixed37.56%application CPU regression
is established. The baseline executable alone varies345.563 ->534.063CPU
seconds, mostly in creation/submission; accumulating registry history and
uncontrolled system activity must not be mistaken for a code-only effect.

All four fixtures are cleaned, preserving the original six chats/provider.
The actual installed application is restored to baseline61BE42 for now; dense
remains unaccepted, not silently adopted. Further causal comparisons will use
independent self-created synthetic profiles through the existing per-launch
RCODE_DATA_DIR override, without rewinding the user's registry or copying their
credentials. Those results must remain separate from final normal-profile and
real-provider acceptance. Protocol and raw evidence:
`output/memory-audit/residual-audit/isolated-profiles/plan.md` and
`dense-two-pair-summary.json`.

The independent-profile protocol is now executing rather than merely planned.
The installed baseline generated and fully verified a synthetic long-chat seed
(17,000reasoning characters and120answer lines). After normal exit, its owned
SQLite store passed integrity checking and an explicit WAL checkpoint. Fifteen
closed seed files were copied into two new profiles with every hash verified;
the first clone also reopens with identical complete persisted content. No
normal-profile data, registry or credentials were copied or rewound. This pair
uses the seed's default1320x880window, identical warm/blank state and60-second
preload on both sides. Both500-stream stages now pass output validation with
500normal server completions and zero disconnects. Baseline/dense initial
private working set is37.18/36.78MiB. With conversations retained after120s,
working set is183.93/200.31MiB; after deleting owned conversations and120s,
108.22/86.02MiB. Final private commit is131.53/108.44MiB. Thus this pair saves
22.20MiB after deletion but does not improve retained-chat working set.

Creation-to-completion process CPU is330.094/331.438seconds (+0.41% dense),
wall102.793/107.799seconds (+4.87%); sampled peak private working set is
1704.37/1718.58MiB and peak commit1802.94/1796.73MiB. One sequential pair is
not a stable CPU/peak claim. Both fixtures are cleaned, the user's original
six chats/provider remain untouched, and the installed normal-profile app is
restored to baseline61BE42. Dense remains unadopted as an overall improvement.
Native Chinese typing/undo-empty and long-chat reopening pass after metrics
are frozen. No post-load restart or forced eviction produced these results.
Raw comparison: `output/memory-audit/residual-audit/isolated-profiles/matched-500/comparison.json`.

### Snapshot Duplication Candidate

An isolated executable using the actual session-document source and Windows
SegmentHeap reproduces a distinct snapshot-frequency-dependent cost. Optional
live-allocation backtraces locate large append-only string allocations in
historical change-block decoding during auto-transaction renewal after snapshot
export. Serialization discarded already parsed changes; later access decoded
the same text back into the shared append-only string arena.

Preserving the parsed change Arc alongside encoded bytes avoids that repeated
decode. The candidate library passes332tests with4ignored; the original passes
five existing change-store tests but fails both added arena-growth regressions.
One original export doubles26arena characters to52without changing visible
text. Full text, operation version vectors, snapshot reimport, continued editing
and concurrent-peer merge are verified in100and500synthetic documents.

At the500-document live checkpoint, private commit is1,134.40MiB original
versus140.73MiB candidate; requested live allocations are1,109.00versus125.32MiB.
These are component checkpoints, not installed-app peak or CPU results.
After all documents drop, both have only2,340requested live bytes, but private
commit remains72.61MiB original versus113.66MiB candidate in that pair. Preventing
duplicate application allocations does not automatically return allocator pages.

Separate candidate post-drop tests return62.98–64.28MiB of real private commit
with an explicit Windows heap-optimization call; three10ms untreated controls
return zero. The observed9.31–10.73ms calls are not a UI latency guarantee,
especially given prior concurrent-tail measurements. No production heap trimmer
has been enabled. This Loro candidate is isolated and is not included in the
dense-only819E6B release, whose installed comparisons are documented above. Evidence:
`output/memory-audit/loro-cache-probe/decision.md` and its raw logs.

The snapshot candidate is now integrated as a local `loro-internal`1.13.9 patch.
The unadopted dense-Taffy experiment is removed from production source first,
returning to the exact pre-dense tree hash, so the next installed comparison
isolates the Loro cache transition. Normal application dependency resolution
does not include the separately vendored upstream test helper.

Actual root `rcode-doc` tests pass166cases, including a new Unicode streaming
regression with repeated snapshots, complete version-vector checks, same-peer
continuation after reopen and concurrent-peer merge. Native engine integration
passes five idle-recovery/background-child/goal-round cases. Broader root
engine unit tests pass427cases with two ignored, and sync passes70cases.
The relocated
vendored library passes332tests with four upstream ignored cases. Its initial
standalone test setup encountered the packaged optional wasm lock conflict;
aligning the standalone lock to the previously verified fixture resolves it,
without changing root runtime versions. Release compilation is the next gate;
the installed normal-profile app remains baseline61BE42. No installed savings
or CPU benefit is claimed from passing these correctness checks.

### Installed Snapshot-Fix Comparison

ReleaseA7758B87 builds successfully with1,159frozen inputs unchanged and is
installed at `D:\dev\RCode\rcode.exe`. This release excludes the unadopted
dense-storage experiment and any heap trimming. The before/after stages use
independent verified clones of the same synthetic profile, the actual native
window and500simultaneous fixed-loopback SSE streams. These are not paid-model
inference scores and do not replace the README's real-provider benchmark.

| Metric | Baseline61BE42 | Snapshot fixA7758B87 |
|---|---:|---:|
| Initial private working set MiB |38.05|37.90|
| Sampled peak private working set MiB |1745.27|795.93|
| Peak private commit MiB |1788.52|814.78|
| Retain conversations, idle120s private working set MiB |202.43|170.82|
| Delete owned conversations, idle120s private working set MiB |103.17|101.03|
| Creation-to-completion CPU seconds |316.156|323.484|
| Creation-to-completion wall seconds |105.950|105.719|

This pair reduces sampled peak working set949.34MiB (54.39%) and peak commit
973.74MiB (54.44%). Retained-chat idle working set decreases31.61MiB, but
post-deletion idle decreases only2.14MiB and still does not return below50MiB.
CPU is2.32%higher and wall time0.22%lower in this one sequential pair; neither
is a stable CPU/performance claim. All500outputs per stage match every expected
reasoning/answer character, with500normal server completions and zero
disconnects. No minimize, restart or forced eviction produces these metrics.

Both installed versions also pass a separate ten-followup history-recovery
check after the frozen load measurements: an actual local Read tool, late
full-tool marker, original tool-call identity, all prior answers and eleven
persisted responses. Document loads increase1,002->1,012while idle writers
remain capped at12, exercising eviction/reload rather than only live-cache
reuse. Native Chinese typing/undo-empty and long-chat reopening pass; initial
typing required a refocus retry and is not claimed as a latency measurement.
Library/engine background and goal tests are correctness coverage, not a claim
that every installed native goal/background scenario was manually exercised.

The candidate now opens the normal user profile with the original six chat IDs
and provider, zero tasks and no synthetic provider; original long-chat/TODO UI
is visible. This fresh normal-profile launch is compatibility acceptance, not
a post-load-memory saving. Active-memory improvement is verified in this
controlled pair; residual idle memory, repeatability and final real-provider
acceptance remain open. Raw evidence:
`output/memory-audit/residual-audit/isolated-profiles/loro-matched-500/`.

### Residual Categories And Next CPU Candidate

A subsequent read-only observation of the normally launched candidate separates
private resident pages from commit and shared images. In the same normal-profile
PID, long-chat open/private residency60.07MiB becomes56.69MiB after CtrlN and60s;
private commit135.72->74.04MiB. This is not the closed process's post500result.
Actual stack-bound residency is2.28->2.38MiB across80threads, ruling out stacks
as the explanation for the whole footprint in these samples. The16logical-core
machine and27Tokio-named threads do not justify assuming80scheduler workers.

The noninvasive heap summary reports default-heap49.54MiB commit/44.01MiB busy
entries. Its5.53MiB difference is overhead/slack, not guaranteed reclaimable.
Partial Toolhelp walks are not extrapolated, and packed allocation addresses
are discarded rather than reused as ownership evidence. One unviewed warm
document remains below the existing cache budget; this alone is not a leak.

Independent CPU review reproduces quadratic resident-estimate scans in document
eviction while all documents are pinned. A candidate avoids whole-map scans
for ineligible documents and avoids ID cloning/sorting when already under
budget. Its source-method/mock-host fixture preserves eviction/save order in
108states and reduces500all-pinned/2048call estimate visits512million->zero.
That is an operation-count result, not installed CPU savings. The next patch
retains final pin/Arc checks, budgets and snapshot persistence. Actual engine
regressions and installed measurement remain required before delivery.

The three new actual-host regressions now pass. The complete engine suite
passes430tests with two ignored, and all five native idle/background/goal
integration tests pass with the scan candidate. Its release build is gated on
these exact source hashes and a single DocHost delta from the installed Loro
control. Subsequent installed testing is recorded below; the candidate is not
adopted into the current production source.

The actual installed A775control/7E369D scan candidate each pass500exact-output
and10followup history-recovery checks in independent same-seed profiles.
Creation-to-completion CPU323.75->336.28seconds (+3.87%), wall107.00->129.48s
(+21.01%); retained idle working set158.61->166.25MiB, postdeletion99.20->98.61MiB,
finalcommit124.26->129.59MiB. Peakworking790.34->767.74MiB and peakcommit814.50
->807.82MiB. This single pair does not isolate every causal effect, but does
not justify an overall resource improvement. The source candidate is rejected
and exactA775DocHost restored; rejected source/binary/raw evidence are retained.

The user's physicalEsc stopped Computer Use before binary rollback and normal
profile restoration. At this checkpoint, installed7E369Dstill runs in the
temporary isolated profile, not the user's normal profile. Window operations
remain stopped pending confirmation. This is explicitly unfinished cleanup,
not a successful delivery or a reason to delete/rewind the original six chats.
Evidence: `output/memory-audit/residual-audit/isolated-profiles/eviction-matched-500/`
and `eviction-scan-probe/source-restoration.json`.

### Independent Allocator Screening

An independent synthetic probe compares Windows System/SegmentHeap with the
exact mimalloc0.1.52v2 already locked for macOS, without changing the installed
Windows app. Three alternating-order500document pairs pass full text/snapshot/
version-vector/concurrent-merge checks. Both have2340requested live bytes after
drop, but30seconds later private commit is6.10–7.63MiB System versus256.52–257.39
MiB default mimalloc. Component build CPU is6.64–6.81versus5.69–5.80seconds;
that CPU difference does not justify the default's much higher retained memory.

Child-only immediate-purge pilots reduce mimalloc postdrop commit to8.69–8.70
MiB, with6.92–6.94CPU seconds and full checks passing, but do not demonstrate
better idle footprint than System or native/concurrent latency safety. Main
probe is dev-profile with opt-level2dependencies, not the release-native app.
No explicit heap collection, persistent environment or production allocator
change is made. System's standalone automatic reclamation does not prove all
native residual pages are reclaimable. Default mimalloc is not adopted as a
Windows memory fix. Evidence: `output/memory-audit/allocator-policy-probe/`.

### Current Render-Pool Ownership

A new source-layout proof avoids reusing the stale72byte Arena reader: current
frozen GPUI Arena is96bytes. An isolated exact-Taffy copy validates1096byte
NodeData,1104byte slots and SlotMap field/version layout, without modifying
production. Actual resident-page scans then validate stable descriptors and
complete free lists in the same installed post500PID, with no UI interaction,
restart, target writes, heap trimming or working-set eviction.

The main arena has already decayed to2MiB payload plus0.50MiB drop metadata,
both with zero live entries; a second1MiB arena has only4KiB private residency.
The distinct Taffy-compatible slot vector still reserves18.328125MiB with
14.9375MiB private residency. Capacity17408/initialized14166slots is not live
node count: all14165nonsentinel slots are verified vacant through a complete,
unique version-checked free list. A second scan reproduces the same descriptor
and free list. These are strong structural matches, not debugger type names;
slot-metadata reads can increase residency, so counts precede the metadata walk
and must not be promoted as a new performance saving.

This separates concrete retained layout capacity from model-task/history leaks
and confirms the arena-decay fix is active in this sample. Discarding slot
generations risks stale NodeID revival, and earlier dense-storage CPU/memory
tradeoffs remain unaccepted. Source review also finds eager construction of
all sidebar row elements rather than viewport virtualization, a possible next
direction requiring native grouping/scroll/drag/animation/keyboard acceptance.
No speculative UI or new allocator patch was installed during the Computer Use
stop. Evidence: `output/memory-audit/residual-audit/render-layout-proof/`.

### Normal-Profile Restoration

After explicit human authorization to resume window operations, the isolated
test instance was normally closed with zero running tasks. The installed binary
was restored from rejected7E369D to acceptedA7758B87, with full SHA256 and RCode
product metadata verified. A normal launch at23:00 on October24 restores the
original six chat IDs and provider ID; the original history and TODO panel are
visible, with zero running tasks and no test message sent. No user configuration
was replaced. This restart is cleanup, not evidence of post-load memory recovery
or an additional memory saving. The retained-layout-pool finding still requires
a separately validated solution. Evidence:
`output/memory-audit/residual-audit/normal-profile-restored.json` and
`output/memory-audit/residual-audit/eviction-restoration.json`.

### Viewport-Row Candidate

The next source candidate keeps the existing outer sidebar slots, grouping,
ordering, gaps, fold/reorder animations and drag insertion targets. It defers
inner active-row widget creation and layout until the current frame's actual
content mask intersects the row. Dragging, inline rename and hovered/card-target
rows remain mounted for their existing focus, reveal and reconciliation paths.
No slot generations, document budgets, allocator policy or history are changed.

The initial implementation exposed three drag-width and two inline-rename
placement regressions. An explicit fixed-size flex root restores the original
cross-axis row stretching; the original assertions pass without being weakened.
The final new component tests verify500rows with only six initially constructed
inner rows, exact width/height/gaps, first/last clicks, end scrolling,
collapse/reopen and a forced row under a zero-height mask. Focused sidebar,
rename and hover-card tests pass55cases. The complete UI executable passes1884
tests with5ignored; the focused cases are included, not additional unique tests.

The source delta is two existing files plus the new viewport_row.rs file.
Source-frozen release construction and actual installed matched-load/native
acceptance remain required. Construction counts are not memory or CPU savings,
and the installed accepted control is unchanged at this point. Evidence:
`output/memory-audit/residual-audit/viewport-row-decision.md`,
`viewport-fixed-existing-tests-result.json` and `viewport-all-ui-tests-result.json`.

The release input guard subsequently stops before starting a compiler: concurrent
terminal-module edits differ from the accepted input set. Those later edits are
not covered by the successful UI test executable and are not reverted or silently
included. The tested candidate source is separately archived with verified hashes;
the normal installed accepted version remains running. Native performance
measurement awaits a controlled source baseline, not an uncontrolled mixed build.

That source-control issue is resolved with a private2213file snapshot, leaving
the user's shared worktree alone. The reconstructed control sidebar hashes
exactly match the accepted implementation; both variants keep the same frozen
terminal changes. The isolated control UI suite passes1900tests/5ignored, and
its release succeeds with zero source drift. Restoring only the two candidate
sidebar files produces an exact candidate manifest and passes1902tests/5ignored
under the same frozen inputs. The candidate release is now building. These are
build/regression results, not an installed memory/CPU improvement. The prepared
500stream profiles remain unused; the normal installed A775 version is unchanged.

### Installed Viewport-Row Result

The subsequent actual installed, matched-source500stream pair completes with
all500full outputs verified and500normal server completions/0disconnects on
each side. Both use independently cloned synthetic seed profiles, matching
native geometry/warmup and120second retained/deleted idle intervals. No measured
process restart, minimization, heap trim or working-set eviction occurs.

| Metric | Control | Viewport candidate |
|---|---:|---:|
| Fresh private WS MiB |37.23|37.68|
| Peak private WS MiB |792.86|722.93|
| Peak private commit MiB |826.32|749.05|
| Keep chats, idle120s private WS MiB |168.98|146.74|
| Delete owned chats, idle120s private WS MiB |101.87|81.84|
| Final private commit MiB |127.27|102.38|
| CPU seconds, creation through completion |305.44|249.77|
| Wall seconds, creation through completion |96.72|95.33|
| Peak threads / final threads |623/80|757/80|
| Peak handles / final handles |2069/730|2495/714|

The first pair shows about20.04MiB lower deleted-idle residency,69.93MiB lower
peak residency and18.23%lower CPU time. Peak threads and handles rise, however;
repeat measurements and thread-pool attribution remain required rather than
claiming every resource metric improved. Post-load private residency remains
above50MiB. This is a deterministic loopback stream test, not real-model
inference, and does not replace the README's real-provider results.

Both stages pass a separate native500empty-row scroll/selection/keyboard,
fold/reopen, rename-cancel, drag/return, hover and long-seed acceptance phase
after frozen metrics. Visible Chinese typing/undo leaves an empty draft. Both
also pass the installed Read/history recovery check with10followups,11tool
results,11persisted answers and repeated document reloads under the12writer cap.
No history limit, slot-generation reset or new allocator policy is introduced.

The measured4368C2 candidate is retained after these checks and opens the normal
user profile again: original6chat IDs/provider, history/TODO and zero running
tasks are verified. The older accepted A775 executable is archived for rollback;
the normal-profile restart is not counted as memory recovery. Single-pair gains
remain provisional pending repetition and peak-thread analysis. Evidence:
`output/memory-audit/residual-audit/viewport-row-decision.md` and
`output/memory-audit/residual-audit/isolated-profiles/viewport-matched-500/`.

### User-Requested Installed Rollback

At 2026-10-25 01:04 +08:00, the user explicitly requested resuming window
operations, rolling back and reopening the normal profile. The idle isolated
4368C2 instance was normally closed, and the archived, previously accepted A775
executable was restored to `D:/dev/RCode/rcode.exe`. Its full SHA256 is
`A7758B87E43301684A45E963617E89CB3C7AEAC442E8D5F822DECC5B44AF3305`.
The reopened normal instance (PID63824) preserves all six original chat IDs and
the original provider IDs; original history/TODO is visible and no tasks are
running. This supersedes the installed-candidate adoption described above, not
the frozen measurements. Source changes and candidate archives are preserved.

The reverse-order repeat completed and cleaned its candidate stage, but did not
start its control stage; it is not a completed second comparison. The normal
profile restart is not counted as memory recovery. Evidence:
`output/memory-audit/residual-audit/isolated-profiles/viewport-repeat-500/normal-profile-rollback-restored.json`.

### Windows Notification-Sound Process Audit

An independent native source-prefix probe on 2026-10-25 finds another resource
cost without touching the restored normal RCode instance. The Windows sound
path starts a PowerShell process per cue even when the current session reports
zero WinMM output devices. For30 concurrent silent cues, the observed child
private-commit peak is810.570MiB and captured-child CPU is9.765625 core-seconds;
the sampler misses short-lived processes, so these are bounded observations,
not complete CPU accounting or an RCode private-working-set reduction.

A four-line Windows-only check in `sound.rs` returns before temporary-file and
player creation when no output devices are available. It executes on the
existing background worker and does not cache device availability or disable
the user's sound settings. The same probe binary verifies30/30 and500/500
no-device skips, zero child processes and clean temporary files; one targeted
no-output regression passes. The dependency change enables the existing
`windows-sys` binding's `Win32_Media_Audio` feature.

The installed A775 executable and normal PID63824 are unchanged. Full UI build,
installed-client load measurements, usable-audio-device playback and hot-plug
acceptance are pending. This is not evidence that post-task residency is below
50MiB or that all previously unnamed burst threads belong to sound playback.
Detailed evidence, API references and the unsuccessful direct-waveOut pilot:
`output/memory-audit/sound-process-probe/decision.md`.

The device-count condition has a material qualification: Microsoft's
[API definition](https://learn.microsoft.com/en-us/windows/win32/api/mmeapi/nf-mmeapi-waveoutgetnumdevs)
also allows zero on an enumeration error. The current session's direct playback
failure corroborates the local observation, but does not prove general audio
equivalence. A transient query error or a device connecting after the check may
skip a cue that the former delayed child playback could have played. Keep that
impact explicit; the candidate is not automatically accepted on component or UI
tests alone. A bounded child-observer calibration captures10/10 with0.5625 CPU
seconds, rather than4.6875 for continuous20ms polling; this is observer evidence,
not a new application benchmark. The transient WMI registration attempt was
canceled and yields no passing event-driven evidence.

The complete isolated UI suite subsequently passes1882 tests,0 failures and
5 ignored, with no filtered tests or drift across2216 source files. An outer
PowerShell single-summary-string indexing bug initially reports a false gate
failure; the original test result is retained, independently checked against
the test binary/log/source hashes, and the parser fix passes9 regression cases.
`full-ui-gate-validation.json` is the authoritative corrected gate, not a rerun
or an overridden failing test. Release compilation has started without changing
the installed A775 instance; native performance and audio-equivalence acceptance
are still pending. A2216-file matching control source is also frozen; it differs
from the candidate only in the guard and its Cargo feature declaration.

Read-only Windows SDK extraction of the actual installed A775 manifest confirms
that it already requests SegmentHeap and PerMonitorV2. This is not a runtime heap
signature or a new optimization. Candidate resources must preserve these existing
declarations; absence from the top-level resource script alone is not evidence
of absence from the linked executable.

The sound-guard candidate release subsequently builds successfully with all2216
inputs unchanged. Archive SHA256:
`6B895051492027E00605B09B7A29D790A915A0FC5492FB927C6B47651ADDDCC0`.
Its RCode product name and normalized linked manifest match the installed A775
baseline. It is not installed; matched-control release compilation has started.
Two fresh15-file synthetic profiles and identically instrumented500-task runners
are prepared, without starting model requests. Child accounting is sampled only
at existing parent checkpoints and remains a lower bound, not complete system
resource accounting. Native memory/performance/audio acceptance is still pending.

### Matched Sound-Guard Native Pair Completed (2026-10-25 03:12)

This supersedes the preceding pending-build/native-measurement progress. Both
2216-file matched releases finish successfully with no source drift and identical
linked manifests. They differ only in the sound guard and its Windows API feature.
The real installed `D:/dev/RCode/rcode.exe` is replaced for each isolated run; both
achieve500 concurrent working sessions,500 verified full outputs and0 disconnects.
These are deterministic loopback streams, not real-provider model inference.

| Metric | Matched Control | Sound Guard |
| --- | ---: | ---: |
| Warm blank private working set | 37.79MiB | 37.44MiB |
| Sampled parent private working-set peak | 795.87MiB | 772.88MiB |
| Retained500 chats, idle120 seconds | 184.02MiB | 190.31MiB |
| Deleted500 chats, idle120 seconds | 100.77MiB | 100.16MiB |
| Sampled parent peak threads | 587 | 116 |
| Parent creation-to-completion CPU | 297.30 core-seconds | 314.53 core-seconds |
| Captured PowerShell children | 176 | 0 |
| Captured child full-stage CPU | 97.83 core-seconds | 0 |
| Sampled child private-commit sum peak | 3038.43MiB | 0 |

Child observations are sampled lower bounds, not complete process-start tracing.
Child commit is not private resident memory; working-set sums duplicate shared
pages. Child CPU spans the entire stage, including idle/deletion, and must not be
added to the shorter parent CPU interval. This single sequential pair does not
establish stable small differences: parent CPU increases5.80%, retained residency
increases6.29MiB and deleted residency differs by only0.62MiB. The guard addresses
a real child-process/thread burst, not the remaining post-load100MiB residency.
Post-task residency below50MiB is still unachieved.

After metrics freeze, both pass10 history-reload followups,11 Read-tool results,
11 persisted answers, visible Chinese input, undo-to-empty and long-chat reopen.
The500-row sidebar interaction set is not repeated in this pair. Positive-device
audio/hot-plug equivalence remains unverified, including the documented zero-on-
error behavior; this candidate is not automatically adopted.

Both temporary fixture/provider sets are cleaned. A775 and the normal profile are
restored again: normal PID43796, original6 chat IDs and provider IDs preserved,
provider revision unchanged, original history/TODO visible, empty draft and no
working sessions. The restart is not a memory-recovery result. No README marketing
claim or commit is produced. Full evidence and impact are recorded in
`output/memory-audit/sound-process-probe/decision.md` and
`output/memory-audit/residual-audit/isolated-profiles/sound-guard-matched-500/`.

### Native Live-Allocation Diagnosis (2026-10-25)

An isolated diagnostic image instruments the existing System-backed Rust global
allocator, not the user's installed executable. Actual500 fixed streams reach
500 working sessions, verify all full outputs and finish with0 disconnects.
The normal A775 PID43796 keeps its original start/hash throughout. Diagnostic
sound disabling, atomic counters, native stack capture, a fixed1.69MiB static
table and a two-second request poll make this attribution evidence, not a new
production memory/CPU score.

Requested live bytes through that allocator are16.75MiB after warmup,80.27MiB
after retaining500 chats,52.06MiB after deleting them and waiting120 seconds,
and52.04MiB after removing the owned fixture space/provider/server too. All four
large-allocation snapshots have0 dropped records; deleted/cleaned docs, views,
writers, connections and pending durable work are zero. About35.3MiB additional
requested payload remains allocated relative to warmup. It is not correct to
treat the entire increase as already-freed payload awaiting Windows reclamation.
This does not prove a leak or equate requested bytes with private resident pages.

Large surviving allocations include18.328125MiB, two4MiB blocks and3.125MiB.
Their sizes alone do not identify their owners. The matching release PDB GUID
and age are verified, but its stripped public symbols mostly yield distant
nearest labels, not real Rust callsite names. The symbolizer now keeps those
labels separate and only assigns a function name when its nonzero function
extent contains the PC; invalid nearest-label reports are preserved and explicitly
superseded. A richer-symbol build is running to obtain source-owned attribution,
with a fresh native capture required before relying on changed-image addresses.

The owned diagnostic process closes normally, its fixture server is stopped,
and the normal app remains running without replacement or restart. This round
does not achieve post-load residency below50MiB or deliver an adopted product
optimization. Detailed raw evidence, scope and next gates:
`output/memory-audit/allocation-live-probe/decision.md`.

### Source-Owned Residual Allocations (2026-10-25)

The richer-symbol diagnostic and its fresh native500 run have now completed.
All500 full outputs verify, with zero disconnects and zero dropped allocation
traces. Local PDB GUID/age, exact image hash and containing function extents are
verified before assigning source owners. Requested-live payload is16.75MiB warm
and52.10MiB after deleting the500 chats and cleaning owned fixtures. Session/doc/
view/writer/connection registries are empty; the synthetic seed chat is retained.

Source-confirmed remaining allocations include19.91MiB in Taffy layout buffers
(0.29MiB warm), and8MiB across two dispatch-node vectors (0.125MiB warm).
These explain about27.50MiB of the35.34MiB requested-live increase. A3.89MiB
model-catalog allocation already exists at baseline and is not post-load growth.
Generic Rust symbol names may be identical-code-folding aliases; verified
non-generic callers plus owning source fields establish these families.

The dispatch vectors are cleared for frame reuse without reducing their peak
capacity. A source candidate reclaims oversized, persistently underused buffers
after30 seconds at a subsequent ordinary clear, without adding a background timer.
Seven focused source regressions and the complete UI suite (1882 passed, zero
failed, five ignored, zero filtered) pass on2217 unchanged frozen inputs.
The production-profile candidate is building; native CPU, recovery and functional
acceptance are still pending. Re-entering a large workload may need new
allocations. Taffy generational IDs are not reset by this candidate.

Requested allocations are not resident pages or promised reclaimable MiB. This
instrumented run is attribution, not a production performance score. The normal
installed A775 executable and original PID43796 remain unchanged, and the
diagnostic process/server are closed. No new production saving or post-load
below50MiB achievement is claimed. Evidence and decision gates are recorded in
`output/memory-audit/allocation-live-probe/private-owner-family-summary.json` and
`output/memory-audit/dispatch-capacity-decay/decision.md`.

### Direct Slot Payload: Production Native Pair, Held

On2026-10-25 the pure2237-input candidate finishes the unchanged production
release build,1680.18seconds, zero source drift. Actual compiled fork/Taffy paths
and RCode display metadata validate. Archive SHA256 is
`0B30D24F10CBD7502786351F97C03CADFFDBB96777A7424C887ED90A905EFF36`.
Complete UI correctness tests pass1882/5ignored/0failed/0filtered. The normal
installed A775 executable and PID43796/birth03:12:44 remain unchanged.

Both fresh native profiles use the same1322x881 visible seed, actual editor
focus, full-option sound and fixed loopback500 barrier. Both verify500 concurrent
working sessions,500 exact full outputs, zero disconnects,120-second retained
and deleted waits, and owned cleanup. This is not real-provider inference,
installed acceptance or a cold blank-app comparison.

Original -> candidate private WS MiB: warm38.67 ->63.90; sampled peak809.45 ->
835.71; retained154.82 ->188.31; deleted107.07 ->119.14; quiet-cleaned108.79 ->
119.29; after two ordinary Tab navigations103.43 ->91.37; after actual unsent
input and seed-history reopen106.25 ->93.17. Parent creation-to-completion CPU
325.15625 ->334.25core-seconds is2.80% worse; elapsed108.1613 ->106.0195seconds
is1.98% better. Warm baselines differ25.23MiB and captured direct PowerShell
activity varies408 ->206, so one sequential pair does not establish causality
or stable improvement. The observed12.06MiB frame-reuse saving is not a quiet
idle win and is not a below50MiB result. Child CPU has a different full-stage
sampling interval and is not silently added to the parent interval.

First-stage actual Chinese/English input, focus, unsent draft preservation,
blank-page navigation and seed-history return are observed on both sides.
The control then closes normally. The candidate remains the same PID for a
second500 run, which again passes500 full outputs/concurrency/zero disconnects
and owned cleanup. Second-run post-delete WS is101.38MiB and a later pre-UI
checkpoint is100.77MiB. The next screenshot unexpectedly shows General
Settings, not the pre-run seed view. No agent UI action occurs during the run;
cause/timing is unknown. Further window inputs stop pending the user's answer.
This second memory checkpoint is therefore not claimed as a matched seed-view
plateau, nor as proof of a navigation bug or any settings change.

Decision: **HOLD, not adopted**.64 low-demand clears remain frame-dependent;
background quiet recovery and native CPU reduction are not demonstrated.
No forced working-set trim, minimization or post-load restart is counted as
recovery, and no README marketing claim is added. Evidence and full limitations:
`output/memory-audit/slot-payload-probe/decision.md`,
`output/memory-audit/slot-payload-probe/native-pair/comparison.json` and
`output/memory-audit/slot-payload-probe/native-pair/after/reentry/`.

The human subsequently confirms operating the isolated window and authorizes
continuation. The agent returns from settings without changing controls; seed
history, preserved draft, editor focus, two Tab navigations and new literal
unsent input all verify. Full persisted seed content remains exact. After
return to the seed and ordinary focus/navigation, private WS is102.70MiB, not
a repeat of the first91.37MiB checkpoint. Human view changes remain a recorded
confound, not a reason to rewrite the second result. All diagnostic services
are cleaned and the candidate closes normally after measurement. At10:59:05
owned27680 is closed, no compiler is active, and normal A775/PID43796/birth is
unchanged. No UI clarification remains pending. The candidate is still held;
post-load below50MiB and stable native CPU reduction remain unachieved.

### Layout Payload Prototype: Component Evidence Only

An isolated source prototype keeps small generational SlotMap metadata but
separates the large NodeData payload vector. Empty payload reclamation preserves
stale-key rejection without changing public LayoutId or resetting generations.
Root production Taffy and the installed normal RCode are unchanged.

The first prototype passes100 actual upstream/new unit tests and seven external
original/candidate lifecycle/layout tests. Four alternating-order native component
pairs each run512 large-tree rounds and13,824 stale-key/regrowth checks. Process
CPU averages19.84 ->17.56core-seconds (11.50% lower); requested-live payload drops
by17.12MiB, with17.17MiB mean observed private-commit reduction. All140 recorded
inputs remain unchanged. This is instrumented component evidence, **not** private
resident desktop memory, an installed product saving or post-load below50MiB.

A later destructor-unwind robustness change is separately versioned and still
under tests/rebenchmark. The prototype's64-low-clear trigger is not an accepted
production idle policy. Full desktop integration, native performance and feature
acceptance remain outstanding; nothing is installed or promoted in README.
Evidence: `output/memory-audit/taffy-sparse-probe/decision.md`.

The final v2 robustness change now passes101 unit tests, seven std consumers and
seven alloc-only consumers. A separate four-pair v2 rerun verifies all140 source
hashes and13,824 stale-key/regrowth checks per process. Final component CPU is
19.64 ->17.84core-seconds (9.17% lower), with17.12MiB payload reclamation and
17.17MiB observed commit reduction. This supersedes the older v1 CPU percentage
for current-source evaluation; neither is a desktop WS/product score.
A pure2217-input desktop integration snapshot changes only the three Taffy
files, without the held dispatch/sound/viewport experiments. Complete UI
regression is running; native adoption and post-load below50MiB remain unproved.

The complete sparse integration UI suite now passes1882 tests (zero failures,
five ignored, zero filtered), with2217 unchanged inputs. Independent log/result/
binary hash validation passes. The unchanged-profile production release is
building; real native500/re-entry/CPU/private-WS acceptance remains outstanding.
Normal A775/PID43796/start remain unchanged and no candidate is installed.

### Sparse Layout: First Native Pair Is Held

The pure sparse production release completes with2217 unchanged inputs. Actual
full-option native runs on fresh profiles each verify500 working sessions,500
full fixed-stream outputs and zero disconnects. No allocator instrumentation or
sound override is included. Control -> sparse private WS MiB: warm39.88 ->64.26,
deleted108.59 ->118.89, quiet-cleaned108.53 ->118.44, after ordinary frame/focus
actions104.05 ->90.33. The post-interaction difference favors the candidate by
13.72MiB, but quiet/deleted memory is worse, not improved.

Parent load CPU321.31 ->346.69core-seconds (7.90% worse), wall106.48 ->111.80seconds
(4.99% worse). Captured direct PowerShell children213 ->459 and initial residency
differ materially; one sequential pair does not isolate causal overhead. Neither
the favorable component CPU/17MiB result nor baseline-adjustment overrides these
unfavorable actual observations. Candidate is held, not installed/adopted.

A second500 load in the same candidate process again verifies all500 outputs/
working sessions/zero disconnects, but final residency is127.13MiB after deletion,
114.27MiB quiet,114.23MiB after frame reuse and102.79MiB after real history reopen.
It does not return to the first90.33MiB checkpoint or below50MiB. Full persisted
seed content survives, navigation to a new unsent draft and back verifies, and
owned fixtures/clients close normally. Normal43796/A775/start remain unchanged.
Reverse-order/noise isolation and actual clear-trigger policy investigation
remain necessary. Evidence: `output/memory-audit/taffy-sparse-probe/decision.md`.

The subsequent reverse-order diagnostic runs the sparse candidate first and
control second, with sound disabled only in both isolated child environments.
Both verify500 working sessions/full fixed-stream outputs/zero disconnects,
matched warm baselines40.07 ->38.46MiB, and zero sampled PowerShell children.
Control -> candidate quiet private working set110.57 ->92.58MiB saves17.99MiB;
after ordinary frame actions105.21 ->88.50MiB saves16.70MiB. Parent load CPU
308.58 ->326.09core-seconds is5.68% worse and wall99.25 ->104.18s is4.96% worse.
This is a single sequential diagnostic pair, not real-provider inference or
proof of causal overhead. The memory benefit does not justify automatic
adoption with this CPU result. Candidate remains held; original full-option
and same-process regrowth observations remain applicable. Both diagnostic
clients close normally, fixtures stop, and the normal installation/configuration
remain unchanged. Evidence: `output/memory-audit/taffy-sparse-probe/native-pair-quiet/comparison.json`.

### Direct Slot Payload: Component Gate Passes, Native Pending

An isolated slotmap1.1.1 fork exposes an explicit slot-index accessor and
empty-only low-first free-list ordering without changing key versions. Primary
unit metadata validates full keys; a directly indexed payload Vec can release
large allocations. Parent/child secondary maps share explicit keys. The storage
and fork API introduce no new unsafe block or opaque FFI-bit decoding.

A reproduced regression (next small slot13000 instead of1) reveals unnecessary
sparse-span refills before reclamation. Final code orders empty metadata on
every successful clear and appends live values directly.28 fork tests,105 Taffy
unit tests and17 std/17 alloc-only consumer tests pass, including100,000 metadata
operations, null/stale keys, version wrap, cloning, drops/panic and layout equality.

24 native component trials complete with233 unchanged inputs and matching
layout fingerprints. Actual large-workload CPU original -> candidate is
5.74609 ->5.36719core-seconds/default (6.59% lower) and1.26563 ->1.07422/mixed
(15.12% lower). Whole-child CPU including regrowth/stale-key audits also falls
5.15%/14.86%; it is reported separately, not silently excluded. Recovery-frame
CPU falls below counter resolution, so zero readings do not imply zero work.

Requested payload reclaimed18.19322MiB and observed private-commit drop about
18.24MiB are **component**, not RCode desktop private-working-set, results.
Normal installation/configuration remain unchanged. Proceed to complete desktop
UI/build and native paired/re-entry tests; do not install or advertise whole-app
gains yet. The64-clear recovery policy remains frame dependent, not a background
idle guarantee. Full-build disk reserves must be restored without weakening
existing gates. Evidence: `output/memory-audit/slot-payload-probe/decision.md`
and `output/memory-audit/slot-payload-probe/benchmark-results.json`.

Full integration subsequently freezes2237 actual inputs from the pure control,
not the dirty worktree. Only tested storage/fork sources, explicit local binding
and necessary slotmap lock references change; production package/profile and
the existing registry slotmap identity/checksum remain unchanged. Independent
source/path validation passes with zero drift. Full desktop UI compilation
actually starts09:26:19 and is still advancing at09:37:33; tests/native memory
acceptance are not yet complete. Normal installed A775/PID43796 stays untouched.

To restore existing build reserves,1850 idle flat cache artifacts are removed
with path/hash/process guards, without touching incremental directories,
source/user data or archived executables/private symbols. Actual free space
increases4.124GiB on C and1.292GiB on D. Old mutable cache test images/PDBs must
be rebuilt; historical results are not a current executable gate. This disk
maintenance is not application memory saving. Evidence:
`output/memory-audit/slot-payload-probe/build-reserve-cleanup.json` and
`output/memory-audit/slot-payload-probe/full-ui-build-start.json`.

That full UI build finishes successfully in33m55s; a post-build driver selector
mistakenly includes the package's custom-build target. The raw compiler evidence
and exact fresh library/image paths are verified, then the already built image
runs once without recompilation:1882passed/0failed/5ignored/0filtered in10.59s.
All2237 inputs remain unchanged and the normal instance is preserved. The
production-profile candidate build starts10:00:53 and is still live at10:03:57;
neither a completed release nor desktop memory/CPU improvement is claimed yet.
Fresh isolated native seeds and port27680 ownership/refusal guards are prepared,
with no app/server/request launch. Evidence:
`output/memory-audit/slot-payload-probe/full-ui-result.json` and
`output/memory-audit/slot-payload-probe/release-build-start.json`.

### Cache Representation: Three Component Gates, Not Adopted

Three isolated candidates preserve all nine slots and validate std/serde,
alloc-only, NaN/signed-zero, clone/equality/debug/clear and computed layouts.
Each passes98 release unit tests plus10 std and10 alloc-only consumer tests.
The original alloc+serde feature combination already fails on an unrelated
missing String import; it is not fixed or claimed as passing here.

Each candidate measures Cache368 ->296 bytes and1.1953125MiB less requested
retained payload. Each completes24 native component trials with142 unchanged
inputs and matching layout/cache fingerprints. CPU changes relative to its
own interleaved original control (positive means slower):

| Candidate | Default layouts | Mixed layouts | Cache hot path |
| --- | ---: | ---: | ---: |
| Occupancy bitmap, dynamic bit scan |+1.51%|-1.71%|+7.18%|
| Occupancy bitmap, fixed slots |+2.45%|+3.02%|-2.34%|
| Safe NonZeroU64 key niche, original Option slots |+3.34%|+6.77%|+2.39%|

All three fail the intended CPU/memory tradeoff and remain unadopted; no full
desktop candidate is built or installed from these variants. These72 trials
are instrumented component evidence, not RCode private-working-set or real-
provider scores. A compact representation or an improved microbenchmark alone
does not prove a faster application. Normal app/configuration remain unchanged.
Evidence: `output/memory-audit/cache-occupancy-probe/benchmark-results.json`,
`output/memory-audit/cache-occupancy-loop-probe/benchmark-results.json`, and
`output/memory-audit/cache-niche-probe/benchmark-results.json`.

### Direct Secondary Payload: Rejected At Component Gate

A separate generation-safe prototype removes the extra key-to-payload lookup,
using a primary unit SlotMap and explicit SecondaryMap keys for payload,
parents and children. Root production sources and normal installation are not
changed. Actual102 unit/7 std consumer/7 alloc-only consumer tests pass, including
stale IDs, context drops, layout equality, destructor panic and bounded small
regrowth after13,000 nodes with holes.

A three-way instrumented component benchmark completes24 processes, four
rotated trials per implementation/workload,210 inputs with zero drift and
matching computed-layout fingerprints. Mean CPU original -> prior sparse ->
new direct: default6.336 ->6.012 ->6.750 core-seconds; heterogeneous flex-wrap
1.332 ->1.352 ->1.457. New direct is6.54% and9.38% worse than original and
reclaims only1.20MiB more requested payload than prior sparse. It is rejected
before expensive full-desktop integration. This is not a whole-app RAM score
or evidence of causal instruction-level overhead. Prior favorable component
CPU does not generalize to the mixed workload or override native regressions.
No installed adoption or post-load below50MiB claim follows.
Evidence: `output/memory-audit/taffy-direct-probe/benchmark-results.json`.

### Dispatch Capacity Decay: First Native Pair, Not Adopted

The pure production-profile candidate builds successfully with2217 unchanged
inputs. Both native500 runs verify500 simultaneous working sessions,500 full
fixed-stream outputs and zero disconnects. Independent fresh synthetic profiles
and the same1322x881 visible long-history window are used; this is not a blank
cold-start baseline, installed acceptance or real-provider inference.

Control -> candidate private WS MiB: warm63.83 ->63.86, deleted132.69 ->126.33,
quiet-cleaned134.35 ->126.83, after ordinary frame/focus actions106.53 ->100.37.
The observed final differences are about6–8MiB. Retained500 memory is worse,
189.86 ->206.24MiB. Parent baseline-to-completion CPU is318.63 ->344.47core-seconds
(8.11% worse), with wall time102.54 ->111.44seconds (8.68% worse).

Captured direct PowerShell child activity varies130 ->346 and its sampled CPU
75.47 ->151.27seconds over a different/full-stage interval. One sequential pair
does not assign causal CPU regression to the policy, but worse observations
cannot be discarded. Both binaries also fall about26–28MiB during later UI
interaction, which must not be attributed wholesale to the capacity change.
The candidate is held, **not adopted or installed**. A sound-disabled-on-both-
sides diagnostic repeat and large-workload re-entry/latency remain outstanding.

Actual focus and final draft/undo states are observed; initial automated text
captures lag later key/IME interaction, so immediate input/IME latency acceptance
is not claimed. Both clients close normally, fixture providers/servers/chats are
cleaned, seed history survives, and normal A775/PID43796/start remain unchanged.
No post-load below50MiB achievement or new README performance claim is made.
Raw matched results and decision:
`output/memory-audit/dispatch-capacity-decay/native-pair/comparison.json` and
`output/memory-audit/dispatch-capacity-decay/decision.md`.
