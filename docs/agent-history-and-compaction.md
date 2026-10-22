# Native agent history, compaction and tool scheduling

The native agent has no default iteration ceiling. It continues until the model
answers without tools, the user cancels, or execution fails. Tool concurrency
does not change this policy.

## History

The chat document contains presentation summaries. It is not the authoritative
source of complete model/tool messages.

For new native turns, the harness records completed assistant messages and tool
results locally under the configured data root's `model-history/` directory.
Each assistant entry group has one append-only file, named by a hash of the
group id. Versioned records retain exact tool arguments, full results and
reasoning signatures. The native `SessionStarted` id anchors the corresponding
display entry and its continuation segments.

On restore, a wholly visible entry group uses these structured records once.
Results are paired with their calls; a missing result after interruption is
represented as an explicit error, never by executing the historical call.
Complete corrupt records fail visibly; only a torn final line is ignored.
Reasoning from a different configured model is omitted, while observable work
is retained. Recording failures stop subsequent work; completed tool results
remain available to a live in-process resume.

Excluded or partially compacted groups are not restored wholesale. Legacy
history uses display text with full tool outputs fetched from the local run
journal, when present. Hydration runs off the async engine thread and does not
expand or rewrite the shared chat document. Remote-only history without either
local source retains its available text; it cannot reconstruct missing data.

## Compaction

Compaction cuts at complete exchanges, including assistant/tool cycles within
one user task. Calls and their results stay together. Full compaction retains
recent exchanges and, when cutting inside a task, its latest user instruction.

Micro compaction only shortens old successful tool outputs. Recent exchanges
and error diagnostics remain verbatim. Old results retain their head and tail
with an omission marker inside the character budget, so repeated compaction
does not progressively destroy the retained text. The durable records retain
the original output.

## Tools

Tools are serial by default. `Read`, `Grep`, `Glob`, `FolderOperations`,
`WebFetch` and `WebSearch` explicitly allow concurrent invocations. Consecutive
safe calls run with at most eight in flight. Writes, edits, shell commands,
questions and unclassified tools form ordering barriers. Configured tool hooks
also keep execution serial because their commands may have side effects.

Results reach the model in call order even if tools finish out of order.
Cancellation drains started calls and prevents later side-effecting batches.

Shell transport, pipeline status, timeout semantics and line-ending-aware exact
edits are described in [Shell execution and exact file edits](agent-tool-execution.md).

## Regression checks

- `cargo test --locked --offline -p rcode-agent --lib`
- `cargo test --locked --offline -p rcode-harness --lib native_history`
- `cargo test --locked --offline -p rcode-engine --lib sessions::history_tests`

These cover local persistence/reload, interrupted pairs, full legacy outputs,
single-task compaction, recent/error preservation, concurrency barriers and
cancellation. They do not establish real-provider latency or task quality.
