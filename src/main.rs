use rust_proof_service::app_from_env;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let bind = std::env::var("APP_BIND").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let addr: SocketAddr = bind.parse()?;
    // CONTROL_DB_URL set → durable Postgres control store (#7); unset →
    // the in-memory demo platform, byte-identical to before.
    let router = app_from_env().await?;
    tracing::info!("control plane listening on {addr} — doctor UI at http://{addr}/");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, router).await?;
    Ok(())
}
