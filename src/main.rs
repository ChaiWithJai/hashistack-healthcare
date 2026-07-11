use rust_proof_service::app;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let bind = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let addr: SocketAddr = bind.parse()?;
    tracing::info!("control plane listening on {addr} — doctor UI at http://{addr}/");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app()).await?;
    Ok(())
}
