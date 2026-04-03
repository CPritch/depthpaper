mod config;
// Stub these for now
// mod wayland;
// mod renderer;

use anyhow::Result;
use tracing::{info, error};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("depthpaper=debug")),
        )
        .init();

    info!("Starting depthpaper daemon...");

    // Load configuration
    let cfg = match config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load configuration: {:#}", e);
            return Err(e);
        }
    };

    info!(?cfg, "Configuration loaded successfully");

    // let mut app = wayland::App::new(cfg)?;
    // app.run()

    Ok(())
}