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
    protocol::{wl_output, wl_surface},
    Connection, QueueHandle, Proxy,
};
use std::ptr::NonNull;
use std::ffi::c_void;

use crate::config::Config;
use crate::renderer::Renderer;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};

pub struct OutputInfo {
    pub name: String,
    pub wl_output: wl_output::WlOutput,
    pub width: u32,
    pub height: u32,
    pub scale: i32,
    pub layer_surface: Option<LayerSurface>,
    pub configured: bool,
    pub wgpu_surface: Option<wgpu::Surface<'static>>,
    pub wgpu_config: Option<wgpu::SurfaceConfiguration>,
}

pub struct App {
    pub config: Config,
    pub registry_state: RegistryState,
    pub compositor_state: CompositorState,
    pub output_state: OutputState,
    pub layer_shell: LayerShell,
    pub shm: Shm,
    pub outputs: Vec<OutputInfo>,
    pub renderer: Option<Renderer>,
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
            renderer: None,
            running: true,
        };

        event_queue.roundtrip(&mut app)?;
        info!(count = app.outputs.len(), "discovered outputs");

        Ok(app)
    }

    pub fn run(&mut self) -> Result<()> {
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
    ) {}

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {}

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {}

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {}
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
        let info = OutputInfo {
            name: String::new(),
            wl_output: output,
            width: 1920,
            height: 1080,
            scale: 1,
            layer_surface: None,
            configured: false,
            wgpu_surface: None,
            wgpu_config: None, 
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

        if self.renderer.is_none() {
            info!("Initializing wgpu renderer...");
            self.renderer = Some(pollster::block_on(Renderer::new()).expect("Failed to init wgpu"));
        }

        if let Some(output) = self
            .outputs
            .iter_mut()
            .find(|o| o.layer_surface.as_ref() == Some(layer))
        {
            if w > 0 { output.width = w; }
            if h > 0 { output.height = h; }
            output.configured = true;

            layer.wl_surface().commit();

            let renderer = self.renderer.as_ref().unwrap();
            
            if output.wgpu_surface.is_none() {
                let display_ptr = _conn.backend().display_ptr() as *mut c_void;
                let surface_ptr = layer.wl_surface().id().as_ptr() as *mut c_void;

                let display_handle = WaylandDisplayHandle::new(
                    NonNull::new(display_ptr).expect("Wayland display pointer was null")
                );
                
                let window_handle = WaylandWindowHandle::new(
                    NonNull::new(surface_ptr).expect("Wayland surface pointer was null")
                );

                let target = wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: RawDisplayHandle::Wayland(display_handle),
                    raw_window_handle: RawWindowHandle::Wayland(window_handle),
                };

                let surface = unsafe { renderer.instance.create_surface_unsafe(target) }
                    .expect("Failed to create wgpu surface");
                
                output.wgpu_surface = Some(surface);
            }

            if let Some(surface) = &output.wgpu_surface {
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    width: output.width,
                    height: output.height,
                    present_mode: wgpu::PresentMode::Fifo,
                    alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied, 
                    view_formats: vec![],
                    desired_maximum_frame_latency: 2,
                };
                
                surface.configure(&renderer.device, &config);
                output.wgpu_config = Some(config);
                
                info!(name = output.name, "Configured wgpu swapchain");
            }
        }
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