# Development fixtures

- `resource-stream.jsonl`: sanitized text/reasoning deltas from an 80-section Rust
  ownership response. Account, session, timestamp and usage metadata are omitted.
- `runway-short-stream.jsonl`: a synthetic reply split into 24-character chunks.
- `transcript-selection-stream.jsonl`: a synthetic reply split into six-character
  chunks for streaming-selection cases.
- `preview-server/`: the Vite dependency used by the native preview fixture.
  Install it with `npm ci --prefix scripts/fixtures/preview-server`.

These data files are inputs, not executable validation commands. Current build
and platform checks are listed in
[development validation](../../docs/development-validation.md).
