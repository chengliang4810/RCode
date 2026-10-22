//! Terminal panel (ARCHITECTURE.md): an `alacritty_terminal`-backed
//! emulator fed by the engine's PTY stream over the generic RPC client.
//!
//! - [`emulator`] — pure Term + vte state machine (bytes in, grid out);
//! - [`view`] — cell palette, keystroke→bytes encoding, input coalescing, and
//!   the custom grid-painting element;
//! - [`panel`] — session-scoped tabs, subscriptions with reconnect backoff,
//!   drag-reorder, and the Cmd/Ctrl+J toggle action.
//!
//! Method names come from `rcode_rpc::methods` and wire types from
//! `rcode_proto` (`TerminalSession`, `TerminalEvent`) — the same contract the
//! engine serves (ARCHITECTURE.md).

pub(crate) mod dock;
pub mod emulator;
pub mod panel;
pub mod view;
