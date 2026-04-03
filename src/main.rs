mod config;
mod cursor;
mod depth;
mod renderer;
mod wayland;

use anyhow::{Context, Result};
use tracing::{error, info, warn};
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
    let mut app = wayland::App::new(cfg.clone(), &globals, &qh)?;

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

    // Initialize cursor poller
    let mut cursor = match cursor::CursorPoller::new(cfg.general.cursor_poll_hz) {
        Some(c) => {
            info!(hz = cfg.general.cursor_poll_hz, "cursor polling initialized");
            Some(c)
        }
        None => {
            warn!("failed to initialize cursor poller — parallax disabled");
            None
        }
    };

    app.render_all(&qh);

    while app.running {
        event_queue.blocking_dispatch(&mut app)?;
        app.poll_depth_results();

        // Poll cursor and update uniforms
        if let (Some(cursor), Some(renderer)) = (&mut cursor, &app.renderer) {
            // Use first output's geometry for now.
            // TODO: per-monitor cursor offsets for multi-monitor.
            if let Some(output) = app.outputs.first() {
                let intensity = cfg.intensity_for(&output.name);

                // Hyprland reports in scaled coordinates; output dimensions
                // from the configure event are already in surface coords.
                let moved = cursor.poll(
                    0.0, 0.0, // monitor offset in global coords (single monitor = 0,0)
                    output.width as f32,
                    output.height as f32,
                    0.3, // smoothing factor (0.0 = frozen, 1.0 = instant)
                );

                if moved {
                    renderer.update_uniforms(
                        cursor.offset_x,
                        cursor.offset_y,
                        intensity,
                    );
                }
            }
        }

        app.render_all(&qh);
    }

    Ok(())
}