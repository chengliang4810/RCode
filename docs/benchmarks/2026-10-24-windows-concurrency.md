# Windows real-model concurrency benchmark — October 24, 2026

RCode's native agent submitted real remote model requests at seven task counts. This run measures the desktop client's resource use and task lifecycle under load.

## Environment and workload

- Windows 11 Pro; AMD Ryzen 7 5700G, 16 logical processors, 59.89 GiB usable physical memory.
- Local optimized Windows release executable, SHA-256 `7F54CB248D272B13D1EC4A4D7B743ADCE35964C2DF0BCDC221563508B31FB1C3`. The same executable and process were used throughout all seven stages. This was a local build; results do not identify every downloadable release.
- Model: `deepseek-v4.1-flash` over an OpenAI-compatible Chat Completions endpoint. Model inference ran remotely.
- Each task requested 120 numbered Chinese scenery descriptions, about 20 Chinese characters per line, with no tools, file operations, network browsing, or subagents. Model reasoning and answer lengths varied. The configured model output limit was 4,096 tokens; it was not verified as a hard bound on total reasoning plus answer usage.
- Tasks were submitted through the running application's normal native-agent command path. Automatic chat titles remained enabled and could make auxiliary model calls.
- Each stage finished before the next began. After task termination, the process was observed for 120 seconds; only that stage's test chats were then deleted, followed by another 120-second observation. The app was never restarted and its working set was never forcibly emptied during the ladder.

## Memory results

All memory figures below are MiB. Private working set is resident private process memory; private commit is a separate metric. Peaks are sampled observations, so very short spikes can be missed. Figures exclude measurement scripts, child processes, and remote inference resources.

| Tasks | Stage-start private working set | Peak private working set | Peak increase | Mean increase per requested task | Peak private commit | After deletion + 120 seconds | Main-task errors |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 10 | 59.750 | 83.234 | 23.484 | 2.348 | 165.750 | 61.770 | 0 |
| 20 | 62.070 | 137.547 | 75.477 | 3.774 | 227.527 | 63.441 | 0 |
| 30 | 63.586 | 139.598 | 76.012 | 2.534 | 227.590 | 66.035 | 0 |
| 50 | 65.855 | 325.848 | 259.992 | 5.200 | 396.984 | 116.262 | 0 |
| 100 | 116.641 | 498.680 | 382.039 | 3.820 | 582.812 | 129.609 | 0 |
| 200 | 129.977 | 763.250 | 633.273 | 3.166 | 870.625 | 152.062 | 0 |
| 500 | 152.570 | 1,800.926 | 1,648.355 | 3.297 | 1,972.578 | 215.258 | 2 |

The mean increase uses each stage's warm starting point, and is not a fixed per-chat allocation or a basis for linear extrapolation. Baselines increased across the ladder; cleanup did not return the process to its earlier idle footprint. At the end of the 500-task stage, before deletion, private working set remained 1,752.574 MiB after the 120-second observation. The application's diagnostics still reported 498 open documents with writer leases. After final cleanup, open documents and writer leases were zero, but private working set remained about 215 MiB. Further attribution and optimization of retained memory remained open at the time of this report.

## Task outcomes and responsiveness

| Tasks | Peak application working count | Tasks with observed output | Observed output-window overlap | Final answers with text | Answers with 120 numbered lines | Main-task errors |
|---:|---:|---:|---:|---:|---:|---:|
| 10 | 10 | 10 | 10 | Not audited | Not audited | 0 |
| 20 | 20 | 20 | 20 | Not audited | Not audited | 0 |
| 30 | 30 | 30 | 30 | Not audited | Not audited | 0 |
| 50 | 50 | 50 | 47 | 49 | 39 | 0 |
| 100 | 100 | 100 | 97 | 100 | 84 | 0 |
| 200 | 200 | 200 | 151 | 198 | 162 | 0 |
| 500 | 500 | 499 | 264 | 486 | 403 | 2 |

Application working states include network and provider waiting. Output-window overlap comes from serial message snapshots whose observed start and end times can both be delayed. Neither measure establishes how many requests the provider computed simultaneously.

All main tasks terminated in every stage. The 500-task stage failed the all-output/no-error acceptance check. One task exhausted retries after the provider reported a 120-requests-per-minute limit. Another exhausted retries after an HTTP failure with an empty response body; its cause was not determined. No explicit insufficient-balance response was observed. Auxiliary title-call failures and costs were not fully counted in main-task outcomes.

Content checks counted final text and numbered lines, not semantic correctness or the quality of every description. Some normally terminated tasks produced only reasoning or fewer than 120 lines. The first three stages were deleted before per-answer content checks were added.

At 500 tasks, average RCode CPU use was 6.962% of the 16-logical-processor machine, with a peak sampled interval of 13.460%. Window-message probes observed no timeouts across 423 samples; the longest response was 114.233 ms. These probes measure message-pump responsiveness rather than frame rate or a complete interactive acceptance test. Window probes were not collected for the 10-task stage.

## Scope and provenance

These results describe one machine, one model endpoint, one local executable, and this text-generation workload. Long conversation histories, large attachments, tool subprocesses, other operating systems, and other providers require their own measurements. New lazy-allocation and idle-release graphics changes were not installed during this benchmark and are not included in these figures.

The local audit retained per-stage manifests, timestamped memory samples, task observations, summaries, cleanup samples, and redacted error records under `output/memory-audit/`. The values in this document are transcribed from the completed stage reports. That local audit directory is not part of this published document; no API credentials or private conversation transcripts are included here.
