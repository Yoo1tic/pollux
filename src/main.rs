use mimalloc::MiMalloc;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::{net::TcpListener, signal};
use tracing::info;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The server binary requires a real config file with a non-empty pollux_key.
    // (Library code uses `config::CONFIG` which is best-effort and does not validate.)
    let cfg = pollux::config::Config::from_toml();

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(cfg.basic.loglevel.clone()));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                // .compact()
                .with_level(true)
                .with_target(false),
        )
        .init();

    let db = pollux::db::spawn(cfg.basic.database_url.as_str()).await;
    let providers = pollux::providers::Providers::spawn(db.clone(), &cfg).await;
    // Build axum router and serve
    let pollux_key: Arc<str> = Arc::from(cfg.basic.pollux_key.clone());
    let state =
        pollux::server::router::PolluxState::new(providers, pollux_key, cfg.basic.insecure_cookie);
    let app = pollux::server::router::pollux_router(state);

    let addr = SocketAddr::from((cfg.basic.listen_addr, cfg.basic.listen_port));
    let listener = TcpListener::bind(addr).await?;
    info!("HTTP server listening on {}", addr);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    info!("Server has shut down gracefully.");
    Ok(())
}
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { /* ... */ },
        _ = terminate => { /* ... */ },
    }
}
