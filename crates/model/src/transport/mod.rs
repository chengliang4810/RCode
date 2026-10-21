// Modified for RCode.
//! 与 Provider 无关的 HTTP 与 SSE 传输基础设施。
#![allow(dead_code, unused_imports)]

//!
//! HTTP seam 仅供 crate 内 adapter 使用；公共协议 API 不暴露 client、headers 或原始请求。

// Ported adjustment: The original project's `transport/cloudflare.rs` (emscripten/Cloudflare
// backend) is intentionally not ported — the local-first target has no such seam.
pub(crate) mod http;
pub(crate) mod sse;

pub(crate) use http::{HttpBody, HttpRequest, HttpResponse, HttpTransport, ReqwestTransport};
pub(crate) use sse::{SseEvent, SseParser};
