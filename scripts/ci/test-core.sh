#!/usr/bin/env bash
# Core Linux Rust tests in one build and one nextest run, shared by pull-request
# CI and the scheduled release-profile checks.
#
# Coverage:
#   rcode-harness, rcode-preview, rcode-model, rcode-mcp: every test binary
#   rcode-agent: lib (including command cancellation/output regressions)
#   rcode-doc      lib + attachments_roundtrip
#   rcode-sync, rcode-update   lib only
#   rcode-engine   lib + the integration tests named below. Engine has many other
#                  integration binaries (live agents etc.) that no workflow ran, so
#                  they are listed instead of globbed to avoid compiling them.
# nextest does not run doctests; these crates have none (their doc comments
# contain no Rust code blocks).
# Extra arguments are passed to nextest (the nightly job adds `--release`).
set -euo pipefail
cd "$(dirname "$0")/../.."
python3 --version >/dev/null # Required by the local-only MCP transport fixtures.

tests=()
for f in crates/{harness,preview,model,mcp}/tests/*.rs; do
  tests+=(--test "$(basename "$f" .rs)")
done
# Integration targets that CI runs, by name — cargo resolves each against
# whichever of the selected packages has it, so this list deliberately spans
# rcode-engine and rcode-doc. A stale name fails the whole run with "no test
# target named", which is how the removed external-agent targets
# (session_publication, codex_subagents, pi_resume) were caught. Check the
# target exists under SOME selected package before deleting a name here:
# `attachments_roundtrip` is rcode-doc's, not the engine's.
for t in restart_resume local_profiles message_queue attachments_roundtrip; do
  tests+=(--test "$t")
done

exec cargo nextest run --config-file scripts/ci/nextest.toml --locked --no-fail-fast \
  -p rcode-harness -p rcode-engine -p rcode-sync -p rcode-update -p rcode-doc -p rcode-preview \
  -p rcode-agent -p rcode-model -p rcode-mcp \
  --lib --bins "${tests[@]}" "$@"
