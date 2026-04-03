use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{Shm, ShmHandler},
};
use tracing::{debug, info, warn};
use wayland_client::{
    globals::GlobalList,
    protocol::{wl_output, wl_surface},
    Connection, QueueHandle,
};

use crate::config::Config;
// use crate::renderer::Renderer;

/// Tracks a single monitor/output and its associated layer surface.
pub struct OutputInfo {
    pub name: String,
    pub wl_output: wl_output::WlOutput,
    pub width: u32,
    pub height: u32,
    pub scale: i32,
    pub layer_surface: Option<LayerSurface>,
    pub configured: bool,
}

pub struct App {
    pub config: Config,
    pub registry_state: RegistryState,
    pub compositor_state: CompositorState,
    pub output_state: OutputState,
    pub layer_shell: LayerShell,
    pub shm: Shm,
    pub outputs: Vec<OutputInfo>,
    // pub renderer: Option<Renderer>,
    pub running: bool,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        let conn = Connection::connect_to_env()
            .context("failed to connect to Wayland display")?;

        let (globals, mut event_queue) =
            wayland_client::globals::registry_queue_init::<App>(&conn)
                .context("failed to init registry")?;

        let qh = event_queue.handle();

        let registry_state = RegistryState::new(&globals);
        let compositor_state =
            CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
        let output_state = OutputState::new(&globals, &qh);
        let layer_shell =
            LayerShell::bind(&globals, &qh).context("wlr-layer-shell not available")?;
        let shm = Shm::bind(&globals, &qh).context("wl_shm not available")?;

        let mut app = Self {
            config,
            registry_state,
            compositor_state,
            output_state,
            layer_shell,
            shm,
            outputs: Vec::new(),
            // renderer: None,
            running: true,
        };

        // Initial roundtrip to discover outputs
        event_queue.roundtrip(&mut app)?;
        info!(count = app.outputs.len(), "discovered outputs");

        Ok(app)
    }

    fn create_layer_surfaces(&mut self, qh: &QueueHandle<App>) -> Result<()> {
        for output in &mut self.outputs {
            if output.layer_surface.is_some() {
                continue;
            }

            let surface = self.compositor_state.create_surface(qh);

            let layer_surface = self.layer_shell.create_layer_surface(
                qh,
                surface,
                Layer::Background,
                Some("depthpaper"),
                Some(&output.wl_output),
            );

            layer_surface.set_anchor(Anchor::all());
            layer_surface.set_exclusive_zone(-1);
            layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer_surface.set_size(output.width, output.height);
            layer_surface.commit();

            debug!(
                name = output.name,
                width = output.width,
                height = output.height,
                "created layer surface"
            );

            output.layer_surface = Some(layer_surface);
        }

        Ok(())
    }

    pub fn run(&mut self) -> Result<()> {
        // TODO: transition to calloop event loop with cursor polling and signal handling.
        // For now, block on the Wayland connection to prove surfaces work.
        let conn = Connection::connect_to_env()?;
        let mut event_queue = wayland_client::globals::registry_queue_init::<App>(&conn)?.1;

        info!("entering main loop");
        while self.running {
            event_queue.blocking_dispatch(self)?;
        }

        Ok(())
    }
}

// --- Smithay delegate implementations ---

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        debug!(scale = new_factor, "surface scale factor changed");
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // TODO: re-render on frame callback
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // We'll get details in update_output; store a placeholder for now
        let info = OutputInfo {
            name: String::new(),
            wl_output: output,
            width: 1920,
            height: 1080,
            scale: 1,
            layer_surface: None,
            configured: false,
        };
        self.outputs.push(info);
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            if let Some(o) = self.outputs.iter_mut().find(|o| o.wl_output == output) {
                o.name = info.name.clone().unwrap_or_default();
                if let Some(mode) = info.modes.iter().find(|m| m.current) {
                    o.width = mode.dimensions.0 as u32;
                    o.height = mode.dimensions.1 as u32;
                }
                o.scale = info.scale_factor;
                
                if o.layer_surface.is_none() {
                    let surface = self.compositor_state.create_surface(qh);
                    let layer_surface = self.layer_shell.create_layer_surface(
                        qh,
                        surface,
                        Layer::Background,
                        Some("depthpaper"),
                        Some(&output),
                    );

                    layer_surface.set_anchor(Anchor::all());
                    layer_surface.set_exclusive_zone(-1);
                    layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
                    // Let the compositor dictate the size via anchors
                    layer_surface.set_size(0, 0); 
                    layer_surface.commit();

                    o.layer_surface = Some(layer_surface);
                    
                    debug!(name = o.name, "created layer surface for output");
                }
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        info!("output removed");
        self.outputs.retain(|o| o.wl_output != output);
    }
}

impl LayerShellHandler for App {
    fn closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
    ) {
        warn!("layer surface closed by compositor");
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = (configure.new_size.0, configure.new_size.1);
        debug!(w, h, "layer surface configured");

        if let Some(output) = self
            .outputs
            .iter_mut()
            .find(|o| o.layer_surface.as_ref() == Some(layer))
        {
            if w > 0 { output.width = w; }
            if h > 0 { output.height = h; }
            output.configured = true;
            layer.wl_surface().commit();
        }

        // TODO: init wgpu renderer here once the first surface is configured
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers!(OutputState);
}

delegate_compositor!(App);
delegate_output!(App);
delegate_layer!(App);
delegate_registry!(App);
delegate_shm!(App);