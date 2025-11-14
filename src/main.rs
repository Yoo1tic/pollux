use std::path::Path;

use mimalloc::MiMalloc;
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env early so Figment/CONFIG sees overrides
    dotenvy::dotenv().ok();
    // Ensure configuration is loaded before logging setup
    let cfg = &gcli_nexus::config::CONFIG;

    // Initialize tracing with loglevel from configuration (env `LOGLEVEL`),
    // falling back to RUST_LOG if present.
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(cfg.loglevel.clone()));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    // Log effective configuration (mask secrets)
    info!(
        database_url = %cfg.database_url,
        proxy = %cfg.proxy.as_ref().map(|u| u.as_str()).unwrap_or("<none>"),
        loglevel = %cfg.loglevel,
        nexus_key = %cfg.nexus_key
    );

    // Initialize and validate configuration early
    let _ = gcli_nexus::config::CONFIG.nexus_key.len();

    // Spawn credentials actor; it will load active credentials from DB automatically
    let handle = gcli_nexus::service::credentials_actor::spawn().await;

    if cfg.load_cred {
        let cred_dir = Path::new("credentials");
        match gcli_nexus::service::credential_loader::load_from_dir(cred_dir) {
            Ok(files) if !files.is_empty() => {
                info!(
                    path = %cred_dir.display(),
                    count = files.len(),
                    "submitting credentials loaded from filesystem"
                );
                handle.submit_credentials(files).await;
            }
            Ok(_) => {
                info!(path = %cred_dir.display(), "no credential files discovered");
            }
            Err(e) => {
                warn!(
                    path = %cred_dir.display(),
                    error = %e,
                    "failed to load credentials from directory"
                );
            }
        }
    }

    // Build axum router and serve
    let state = gcli_nexus::router::NexusState::new(handle.clone());
    let app = gcli_nexus::router::nexus_router(state);

    let addr = "0.0.0.0:8000";
    let listener = TcpListener::bind(addr).await?;
    info!("HTTP server listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}
