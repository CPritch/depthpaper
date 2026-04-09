mod config;
mod cursor;
mod depth;
mod renderer;
mod wayland;

use anyhow::{Context, Result};
use calloop::timer::{TimeoutAction, Timer};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use wayland_client::Connection;
use std::time::Duration;

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
        "outputs ready"
    );

    app.init_cursor(cfg.general.cursor_poll_hz);

    // Render the first frame immediately
    app.render_all(&qh);

    let mut event_loop: calloop::EventLoop<wayland::App> =
        calloop::EventLoop::try_new().context("failed to create calloop event loop")?;
    let loop_handle = event_loop.handle();

    // Wayland fd source — dispatches all SCTK delegate handlers (configure,
    calloop_wayland_source::WaylandSource::new(conn, event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| anyhow::anyhow!("failed to insert Wayland source: {e}"))?;

    // TODO: Implement idle detection and pause.
    let poll_interval = Duration::from_secs_f64(1.0 / cfg.general.cursor_poll_hz as f64);
    let tick_timer = Timer::immediate();
    let qh_tick = qh.clone();

    loop_handle
        .insert_source(tick_timer, move |_deadline, _metadata, app: &mut wayland::App| {
            app.tick(&qh_tick);
            TimeoutAction::ToDuration(poll_interval)
        })
        .map_err(|e| anyhow::anyhow!("failed to insert timer source: {e}"))?;

    info!(hz = cfg.general.cursor_poll_hz, "entering calloop event loop");

    while app.running {
        event_loop
            .dispatch(None, &mut app)
            .context("calloop dispatch error")?;
    }

    Ok(())
}