mod config;
mod wayland;
// mod renderer; 

use anyhow::Result;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("depthpaper=debug")),
        )
        .init();

    info!("Starting depthpaper daemon...");

    let cfg = match config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load configuration: {:#}", e);
            return Err(e);
        }
    };

    info!(?cfg, "Configuration loaded successfully");

    let mut app = match wayland::App::new(cfg) {
        Ok(a) => a,
        Err(e) => {
            error!("Failed to initialize Wayland application: {:#}", e);
            return Err(e);
        }
    };

    info!("Wayland surfaces initialized. Entering event loop...");

    if let Err(e) = app.run() {
        error!("Application error during run: {:#}", e);
        return Err(e);
    }

    Ok(())
}