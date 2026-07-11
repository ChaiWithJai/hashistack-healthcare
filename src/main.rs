use rust_proof_service::app;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let addr: SocketAddr = "127.0.0.1:3000".parse()?;
    tracing::info!("listening on {addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app()).await?;
    Ok(())
}
