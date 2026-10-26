# Thinking preview

Streaming reasoning stays on the existing thought row. Its collapsed header updates with the latest nonblank source line on every text update, without waiting for a newline. Completed reasoning previews its first nonblank line. Opening the detail hides the inline preview; thought details never open automatically, and completion preserves a reader's manual expansion.

The expanded thought detail retains every formatted line. Its viewport grows with short content and stops at 24 display lines (432 px of text plus the existing body padding). The mouse wheel scrolls inside this viewport; horizontal scrolling also preserves access to wide lines. The virtual list only shapes visible rows, keeps its scroll position across renders, and leaves the surrounding conversation in place while scrolling inside the thought.

Ordinary tool-output limits and folded narration previews keep their existing behavior. The outer tool-process group retains its existing folding rules.

Focused verification:

```powershell
cargo test --locked --offline -p rcode-ui --lib thought
cargo run --locked --offline -p rcode-ui --example thinking-preview-fixture --features appshots-fixture
```

The native fixture uses synthetic reasoning and temporary settings; it sends no model requests. It is a temporary development run, not an installed build. Local installation deliveries use the repository's release profile.
