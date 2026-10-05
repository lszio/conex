//! Test support: admitted targets and a minimal plain-HTTP server.
#![allow(dead_code)]
pub mod tls_fixture;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use conex_core::AllowedTarget;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub fn allowed_target(
    addr: SocketAddr,
    scheme: &str,
    server_name: &str,
    fixed_path: &str,
) -> AllowedTarget {
    AllowedTarget {
        target_id: "catalog".into(),
        audience: server_name.into(),
        scheme: scheme.into(),
        hostname: server_name.into(),
        server_name: server_name.into(),
        port: addr.port(),
        pinned_address: addr,
        fixed_path: fixed_path.into(),
        allow_loopback_http: true,
    }
}

/// Serve one fixed-size response and count every HTTP request actually received.
pub async fn plain_server(body_size: usize) -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut buffer = [0u8; 2048];
                let _ = socket.read(&mut buffer).await;
                counter.fetch_add(1, Ordering::SeqCst);
                let body = vec![b'x'; body_size];
                let header = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.flush().await;
            });
        }
    });
    tokio::time::sleep(Duration::from_millis(5)).await;
    (addr, requests)
}
