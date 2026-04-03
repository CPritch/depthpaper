mod config;
// mod depth;
mod renderer;
mod wayland;

use anyhow::{Context, Result};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use wayland_client::Connection;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("depthpaper=info")),
        )
        .init();

    info!("starting depthpaper");

    let cfg = config::Config::load()?;
    info!(?cfg, "configuration loaded");

    let conn = Connection::connect_to_env()
        .context("failed to connect to Wayland display")?;

    let (globals, mut event_queue) =
        wayland_client::globals::registry_queue_init::<wayland::App>(&conn)
            .context("failed to init Wayland registry")?;

    let qh = event_queue.handle();
    let mut app = wayland::App::new(cfg, &globals, &qh)?;

    // Discover outputs and create surfaces
    event_queue.roundtrip(&mut app)?;
    event_queue.roundtrip(&mut app)?;
    app.ensure_layer_surfaces(&qh);
    event_queue.roundtrip(&mut app)?;
    event_queue.roundtrip(&mut app)?;

    if app.render_targets.is_empty() {
        error!("no render targets initialized");
        for (i, o) in app.outputs.iter().enumerate() {
            error!(
                idx = i,
                name = o.name,
                has_layer = o.layer_surface.is_some(),
                configured = o.configured,
                "output state"
            );
        }
        anyhow::bail!("failed to initialize any outputs");
    }

    info!(
        outputs = app.outputs.len(),
        render_targets = app.render_targets.len(),
        "entering main loop"
    );

    app.render_all(&qh);

    while app.running {
        event_queue.blocking_dispatch(&mut app)?;
        app.render_all(&qh);
    }

    Ok(())
}