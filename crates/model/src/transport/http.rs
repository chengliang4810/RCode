// Modified for RCode.
use std::pin::Pin;

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::{ModelError, ModelResult, TransportErrorKind};

const MODEL_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Bound connection setup and each idle read, without capping an active stream's total duration.
pub(crate) fn model_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(MODEL_IO_TIMEOUT)
        .read_timeout(MODEL_IO_TIMEOUT)
        .build()
        .expect("model HTTP client initialization failed")
}

pub(crate) type HttpBody = Pin<Box<dyn Stream<Item = ModelResult<Vec<u8>>> + Send>>;

/// 只在 crate 内使用的 HTTP 请求；认证 headers 与 client 不会离开 transport 层。
pub(crate) struct HttpRequest {
    pub(crate) request: reqwest::Request,
}

impl HttpRequest {
    pub(crate) fn new(request: reqwest::Request) -> Self {
        Self { request }
    }
}

/// 已建立连接后的安全响应元数据与可取消 body。
pub(crate) struct HttpResponse {
    pub(crate) status: u16,
    pub(crate) request_id: Option<String>,
    pub(crate) body: HttpBody,
}

impl HttpResponse {
    pub(crate) fn new(
        status: u16,
        request_id: Option<String>,
        body: HttpBody,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            status,
            request_id,
            body: cancellable_body(body, cancellation),
        }
    }
}

/// 可替换的 HTTP 边界，测试可模拟连接、状态、分块 body 与读取失败，且无需网络。
#[async_trait]
pub(crate) trait HttpTransport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
        cancellation: CancellationToken,
    ) -> ModelResult<HttpResponse>;
}

/// 基于 reqwest 的生产 transport。响应 body 在取消时立即丢弃。
pub(crate) struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub(crate) fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn send(
        &self,
        request: HttpRequest,
        cancellation: CancellationToken,
    ) -> ModelResult<HttpResponse> {
        let response = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(ModelError::cancelled()),
            response = self.client.execute(request.request) => response.map_err(map_reqwest_error)?,
        };
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("x-request-id")
            .or_else(|| response.headers().get("request-id"))
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .bytes_stream()
            .map(|chunk| chunk.map(|chunk| chunk.to_vec()).map_err(map_reqwest_error));

        Ok(HttpResponse::new(
            status,
            request_id,
            Box::pin(body),
            cancellation,
        ))
    }
}

fn cancellable_body(body: HttpBody, cancellation: CancellationToken) -> HttpBody {
    Box::pin(futures::stream::unfold(
        (body, cancellation, false),
        |(mut body, cancellation, reported)| async move {
            if reported {
                return None;
            }
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => Some((Err(ModelError::cancelled()), (body, cancellation, true))),
                chunk = body.next() => chunk.map(|chunk| (chunk, (body, cancellation, false))),
            }
        },
    ))
}

pub(super) fn map_reqwest_error(error: reqwest::Error) -> ModelError {
    let kind = if error.is_timeout() {
        TransportErrorKind::Timeout
    } else if error.is_connect() {
        TransportErrorKind::Connection
    } else {
        TransportErrorKind::Other
    };
    ModelError::transport(kind, None::<&str>).with_error(&error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[tokio::test]
    async fn an_idle_body_times_out_after_300_seconds_but_active_reads_reset_it() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (write_chunk, mut chunks) = tokio::sync::mpsc::channel::<()>(1);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            socket.read(&mut bytes).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\na\r\n")
                .await
                .unwrap();
            while chunks.recv().await.is_some() {
                socket.write_all(b"1\r\nb\r\n").await.unwrap();
            }
        });
        let client = model_http_client();
        let request = HttpRequest::new(client.get(format!("http://{address}/")).build().unwrap());
        let mut response = ReqwestTransport::new(client)
            .send(request, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(response.body.next().await.unwrap().unwrap(), b"a");

        // Pause only after the real loopback connection is established. Each
        // read is polled before advancing time, so its deadline is armed.
        tokio::time::pause();
        for _ in 0..2 {
            let mut next = Box::pin(response.body.next());
            assert!(matches!(
                futures::poll!(next.as_mut()),
                std::task::Poll::Pending
            ));
            tokio::time::advance(Duration::from_secs(250)).await;
            assert!(matches!(
                futures::poll!(next.as_mut()),
                std::task::Poll::Pending
            ));
            write_chunk.send(()).await.unwrap();
            // Let real socket I/O progress without the paused clock jumping to
            // the pending read's deadline.
            tokio::time::resume();
            assert_eq!(next.await.unwrap().unwrap(), b"b");
            tokio::time::pause();
        }
        let mut next = Box::pin(response.body.next());
        assert!(matches!(
            futures::poll!(next.as_mut()),
            std::task::Poll::Pending
        ));
        tokio::time::advance(Duration::from_secs(299)).await;
        assert!(matches!(
            futures::poll!(next.as_mut()),
            std::task::Poll::Pending
        ));
        tokio::time::advance(Duration::from_secs(2)).await;
        let error = next.await.unwrap().unwrap_err();
        assert_eq!(error.transport_kind(), Some(TransportErrorKind::Timeout));
        server.abort();
    }
}
