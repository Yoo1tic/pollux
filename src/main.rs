use mimalloc::MiMalloc;
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let cfg = &gcli_nexus::config::CONFIG;

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(cfg.loglevel.clone()));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_level(true)
                .with_target(false),
        )
        .init();

    info!(
        database_url = %cfg.database_url,
        proxy = %cfg.proxy.as_ref().map(|u| u.as_str()).unwrap_or("<none>"),
        loglevel = %cfg.loglevel,
        nexus_key = %cfg.nexus_key
    );

    let _ = gcli_nexus::config::CONFIG.nexus_key.len();

    let handle = gcli_nexus::service::credentials_actor::spawn().await;

    if let Some(cred_path) = cfg.cred_path.as_ref() {
        match gcli_nexus::service::credential_loader::load_from_dir(cred_path) {
            Ok(files) if !files.is_empty() => {
                info!(
                    path = %cred_path.display(),
                    count = files.len(),
                    "submitting credentials loaded from filesystem"
                );
                handle.submit_credentials(files).await;
            }
            Ok(_) => {
                info!(path = %cred_path.display(), "no credential files discovered");
            }
            Err(e) => {
                warn!(
                    path = %cred_path.display(),
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
