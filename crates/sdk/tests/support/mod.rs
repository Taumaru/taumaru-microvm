#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

const FIXTURE_MARKER: &str = "http://fixture.invalid";

pub struct FixtureServer {
    base_url: String,
    request_count: Arc<AtomicUsize>,
    artifact_request_count: Arc<AtomicUsize>,
    kernel_payload: Arc<Mutex<Vec<u8>>>,
    binary_payload: Arc<Mutex<Option<Vec<u8>>>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl FixtureServer {
    pub async fn start() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Self::start_with_kernel_payload(include_bytes!("../fixtures/kernel-fixture").to_vec()).await
    }

    pub async fn start_with_kernel_payload(
        kernel_payload: Vec<u8>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let base_url = format!("http://{address}/v1/");
        let request_count = Arc::new(AtomicUsize::new(0));
        let requests = Arc::clone(&request_count);
        let artifact_request_count = Arc::new(AtomicUsize::new(0));
        let artifact_requests = Arc::clone(&artifact_request_count);
        let server_base_url = base_url.clone();
        let kernel_payload = Arc::new(Mutex::new(kernel_payload));
        let server_kernel_payload = Arc::clone(&kernel_payload);
        let binary_payload = Arc::new(Mutex::new(None));
        let server_binary_payload = Arc::clone(&binary_payload);
        let (shutdown_sender, mut shutdown_receiver) = oneshot::channel();

        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let Ok((stream, _)) = result else { break };
                        let requests = Arc::clone(&requests);
                        let artifact_requests = Arc::clone(&artifact_requests);
                        let kernel_payload = Arc::clone(&server_kernel_payload);
                        let binary_payload = Arc::clone(&server_binary_payload);
                        let origin = server_base_url.trim_end_matches("/v1/").to_owned();
                        tokio::spawn(async move {
                            let _ = serve_connection(
                                stream,
                                requests,
                                artifact_requests,
                                kernel_payload,
                                binary_payload,
                                origin,
                            )
                            .await;
                        });
                    }
                    _ = &mut shutdown_receiver => break,
                }
            }
        });

        Ok(Self {
            base_url,
            request_count,
            artifact_request_count,
            kernel_payload,
            binary_payload,
            shutdown: Some(shutdown_sender),
            task: Some(task),
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn request_count(&self) -> usize {
        self.request_count.load(Ordering::Relaxed)
    }

    pub fn artifact_request_count(&self) -> usize {
        self.artifact_request_count.load(Ordering::Relaxed)
    }

    pub fn set_kernel_payload(&self, payload: Vec<u8>) -> Result<(), &'static str> {
        let mut current = self
            .kernel_payload
            .lock()
            .map_err(|_| "kernel payload lock is poisoned")?;
        *current = payload;
        Ok(())
    }

    pub async fn start_with_binary_payload(
        payload: Vec<u8>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let server = Self::start().await?;
        server.set_binary_payload(payload)?;
        Ok(server)
    }

    pub fn set_binary_payload(&self, payload: Vec<u8>) -> Result<(), &'static str> {
        let mut current = self
            .binary_payload
            .lock()
            .map_err(|_| "binary payload lock is poisoned")?;
        *current = Some(payload);
        Ok(())
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn serve_connection(
    mut stream: TcpStream,
    request_count: Arc<AtomicUsize>,
    artifact_request_count: Arc<AtomicUsize>,
    kernel_payload: Arc<Mutex<Vec<u8>>>,
    binary_payload: Arc<Mutex<Option<Vec<u8>>>>,
    origin: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buffer = vec![0_u8; 16 * 1024];
    let bytes_read = stream.read(&mut buffer).await?;
    let request = std::str::from_utf8(&buffer[..bytes_read])?;
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_owned();

    request_count.fetch_add(1, Ordering::Relaxed);
    if path != "/v1/" && path != "/v1" {
        artifact_request_count.fetch_add(1, Ordering::Relaxed);
    }
    let kernel_payload = kernel_payload
        .lock()
        .map_err(|_| std::io::Error::other("kernel payload lock is poisoned"))?
        .clone();
    let binary_payload = binary_payload
        .lock()
        .map_err(|_| std::io::Error::other("binary payload lock is poisoned"))?
        .clone();
    let response = fixture_response(&path, &origin, &kernel_payload, binary_payload.as_deref());
    stream.write_all(&response).await?;
    stream.shutdown().await?;
    Ok(())
}

fn fixture_response(
    path: &str,
    origin: &str,
    kernel_payload: &[u8],
    binary_payload: Option<&[u8]>,
) -> Vec<u8> {
    let firecracker_payload = binary_payload
        .unwrap_or_else(|| include_bytes!("../fixtures/firecracker-fixture").as_slice());
    let jailer_payload =
        binary_payload.unwrap_or_else(|| include_bytes!("../fixtures/jailer-fixture").as_slice());
    let mut payloads = HashMap::new();
    payloads.insert("/v1/kernels/linux-test-x86_64/vmlinux", kernel_payload);
    payloads.insert("/v1/kernels/linux-test-aarch64/vmlinux", kernel_payload);
    payloads.insert(
        "/v1/binaries/firecracker-test/1.0.0/x86_64/firecracker",
        firecracker_payload,
    );
    payloads.insert(
        "/v1/binaries/firecracker-test/1.0.0/x86_64/jailer",
        jailer_payload,
    );
    payloads.insert(
        "/v1/binaries/firecracker-test/1.0.0/aarch64/firecracker",
        firecracker_payload,
    );
    payloads.insert(
        "/v1/images/alpine/1.0/alpine-test-minimal.ext4",
        include_bytes!("../fixtures/rootfs-fixture").as_slice(),
    );
    payloads.insert(
        "/v1/images/alpine/1.0/alpine-test-debug.ext4",
        include_bytes!("../fixtures/docker-rootfs-fixture").as_slice(),
    );

    if path == "/v1/" || path == "/v1" {
        let manifest = include_str!("../fixtures/manifest.json").replace(FIXTURE_MARKER, origin);
        return http_response("200 OK", "application/json", manifest.as_bytes());
    }

    match payloads.get(path) {
        Some(payload) => http_response("200 OK", "application/octet-stream", payload),
        None => http_response("404 Not Found", "text/plain", b"not found"),
    }
}

fn http_response(status: &str, content_type: &str, body: &[u8]) -> Vec<u8> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = header.into_bytes();
    response.extend_from_slice(body);
    response
}
