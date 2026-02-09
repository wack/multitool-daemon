use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::net::TcpListener;
use tracing::{error, info};

/// Shared state for health probes.
pub struct HealthState {
    /// True once the controller is fully started and watching resources.
    ready: AtomicBool,
}

impl HealthState {
    pub fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
        }
    }

    /// Mark the controller as ready to serve traffic.
    pub fn set_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Check whether the controller is ready.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }
}

impl Default for HealthState {
    fn default() -> Self {
        Self::new()
    }
}

/// Start a minimal HTTP server that exposes `/healthz` and `/readyz` endpoints.
///
/// - `/healthz` always returns 200 (the process is alive).
/// - `/readyz` returns 200 only when `state.is_ready()` is true, otherwise 503.
pub async fn serve_health(addr: SocketAddr, state: Arc<HealthState>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!(%addr, "health server listening");

    loop {
        let (stream, _peer) = listener.accept().await?;

        let state = Arc::clone(&state);
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let (mut reader, mut writer) = stream.into_split();

            // Read the request (we only need the first line to determine the path).
            let mut buf = [0u8; 1024];
            let n = match reader.read(&mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    error!(%e, "failed to read from health probe connection");
                    return;
                }
            };

            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");

            let (status, body) = match path {
                "/healthz" => ("200 OK", "ok"),
                "/readyz" => {
                    if state.is_ready() {
                        ("200 OK", "ready")
                    } else {
                        ("503 Service Unavailable", "not ready")
                    }
                }
                _ => ("404 Not Found", "not found"),
            };

            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\r\n{body}",
                body.len(),
            );

            if let Err(e) = writer.write_all(response.as_bytes()).await {
                error!(%e, "failed to write health probe response");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_state_defaults_not_ready() {
        let state = HealthState::new();
        assert!(!state.is_ready());
    }

    #[test]
    fn health_state_set_ready() {
        let state = HealthState::new();
        state.set_ready();
        assert!(state.is_ready());
    }

    #[tokio::test]
    async fn health_server_responds() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        let state = Arc::new(HealthState::new());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let server_state = Arc::clone(&state);
        let handle = tokio::spawn(async move {
            serve_health(addr, server_state).await.unwrap();
        });

        // Give the server a moment to bind.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Test /healthz -> always 200
        {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream
                .write_all(b"GET /healthz HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut buf = vec![0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let resp = String::from_utf8_lossy(&buf[..n]);
            assert!(resp.contains("200 OK"), "healthz should return 200");
        }

        // Test /readyz -> 503 when not ready
        {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream
                .write_all(b"GET /readyz HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut buf = vec![0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let resp = String::from_utf8_lossy(&buf[..n]);
            assert!(
                resp.contains("503"),
                "readyz should return 503 when not ready"
            );
        }

        // Mark ready and test again
        state.set_ready();
        {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream
                .write_all(b"GET /readyz HTTP/1.1\r\n\r\n")
                .await
                .unwrap();
            let mut buf = vec![0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let resp = String::from_utf8_lossy(&buf[..n]);
            assert!(
                resp.contains("200 OK"),
                "readyz should return 200 when ready"
            );
        }

        handle.abort();
    }

    use tokio::time::Duration;
}
