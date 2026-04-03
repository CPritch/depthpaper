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
    globals::GlobalList,
};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::ffi::c_void;

use crate::config::Config;
use crate::renderer::{OutputRenderState, Renderer};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};

/// Tracks Wayland state for one output.
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
    pub renderer: Option<Renderer>,
    pub render_targets: HashMap<String, OutputRenderState>,
    pub running: bool,
}

impl App {
    pub fn new(config: Config, globals: &GlobalList, qh: &QueueHandle<Self>) -> Result<Self> {
        let registry_state = RegistryState::new(globals);
        let compositor_state =
            CompositorState::bind(globals, qh).context("wl_compositor not available")?;
        let output_state = OutputState::new(globals, qh);
        let layer_shell =
            LayerShell::bind(globals, qh).context("wlr-layer-shell not available")?;
        let shm = Shm::bind(globals, qh).context("wl_shm not available")?;

        Ok(Self {
            config,
            registry_state,
            compositor_state,
            output_state,
            layer_shell,
            shm,
            outputs: Vec::new(),
            renderer: None,
            render_targets: HashMap::new(),
            running: true,
        })
    }

    /// Called from main after roundtrips — creates layer surfaces for any
    /// outputs that were discovered but never got update_output called.
    /// This handles the SCTK race where output info arrives before our
    /// handler is connected.
    pub fn ensure_layer_surfaces(&mut self, qh: &QueueHandle<Self>) {
        for o in &mut self.outputs {
            // Try to fill in info from OutputState if we haven't got it yet
            if o.name.is_empty() {
                if let Some(info) = self.output_state.info(&o.wl_output) {
                    o.name = info.name.clone().unwrap_or_default();
                    if let Some(mode) = info.modes.iter().find(|m| m.current) {
                        o.width = mode.dimensions.0 as u32;
                        o.height = mode.dimensions.1 as u32;
                    }
                    o.scale = info.scale_factor;
                    debug!(
                        name = o.name,
                        w = o.width,
                        h = o.height,
                        scale = o.scale,
                        "filled output info from OutputState"
                    );
                }
            }

            // Create layer surface if we have a name but no surface yet
            if !o.name.is_empty() && o.layer_surface.is_none() {
                let surface = self.compositor_state.create_surface(qh);
                let layer_surface = self.layer_shell.create_layer_surface(
                    qh,
                    surface,
                    Layer::Background,
                    Some("depthpaper"),
                    Some(&o.wl_output),
                );

                layer_surface.set_anchor(Anchor::all());
                layer_surface.set_exclusive_zone(-1);
                layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
                layer_surface.set_size(0, 0);
                layer_surface.commit();

                info!(name = o.name, "created layer surface for output");
                o.layer_surface = Some(layer_surface);
            }
        }
    }

    pub fn render_all(&self, qh: &QueueHandle<Self>) {
        let renderer = match &self.renderer {
            Some(r) => r,
            None => return,
        };

        for output in &self.outputs {
            if !output.configured { continue; }

            if let Some(render_state) = self.render_targets.get(&output.name) {
                if let Some(layer) = &output.layer_surface {
                    layer.wl_surface().frame(qh, layer.wl_surface().clone());
                }
                renderer.render_frame(render_state);
            }
        }
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
        debug!("new_output: output added, waiting for info");
        self.outputs.push(OutputInfo {
            name: String::new(),
            wl_output: output,
            width: 1920,
            height: 1080,
            scale: 1,
            layer_surface: None,
            configured: false,
        });
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let info = match self.output_state.info(&output) {
            Some(i) => i,
            None => {
                debug!("update_output: no info available yet");
                return;
            }
        };

        let o = match self.outputs.iter_mut().find(|o| o.wl_output == output) {
            Some(o) => o,
            None => {
                warn!("update_output: unknown output");
                return;
            }
        };

        o.name = info.name.clone().unwrap_or_default();
        if let Some(mode) = info.modes.iter().find(|m| m.current) {
            o.width = mode.dimensions.0 as u32;
            o.height = mode.dimensions.1 as u32;
        }
        o.scale = info.scale_factor;

        debug!(
            name = o.name,
            w = o.width,
            h = o.height,
            scale = o.scale,
            "update_output"
        );

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
            info!(name = o.name, "created layer surface for output");
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(o) = self.outputs.iter().find(|o| o.wl_output == output) {
            info!(name = o.name, "output removed");
            self.render_targets.remove(&o.name);
        }
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
        conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = (configure.new_size.0, configure.new_size.1);
        debug!(w, h, "layer surface configured");

        if self.renderer.is_none() {
            info!("initializing wgpu renderer...");
            match pollster::block_on(Renderer::new()) {
                Ok(r) => self.renderer = Some(r),
                Err(e) => {
                    warn!("failed to init renderer: {e:#}");
                    return;
                }
            }
        }

        let output_idx = match self.outputs.iter().position(|o| {
            o.layer_surface.as_ref() == Some(layer)
        }) {
            Some(i) => i,
            None => {
                warn!("configure: no matching output for layer surface");
                return;
            }
        };

        if w > 0 { self.outputs[output_idx].width = w; }
        if h > 0 { self.outputs[output_idx].height = h; }
        self.outputs[output_idx].configured = true;

        let output_name = self.outputs[output_idx].name.clone();
        let output_w = self.outputs[output_idx].width;
        let output_h = self.outputs[output_idx].height;

        debug!(name = output_name, w = output_w, h = output_h, "configuring output");

        let renderer = self.renderer.as_ref().unwrap();

        if !self.render_targets.contains_key(&output_name) {
            let display_ptr = conn.backend().display_ptr() as *mut c_void;
            let wl_surface = layer.wl_surface();
            let surface_ptr = wl_surface.id().as_ptr() as *mut c_void;

            debug!(?display_ptr, ?surface_ptr, "raw wayland handles");

            let display_handle = WaylandDisplayHandle::new(
                NonNull::new(display_ptr).expect("null display ptr"),
            );
            let window_handle = WaylandWindowHandle::new(
                NonNull::new(surface_ptr).expect("null surface ptr"),
            );

            let target = wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: RawDisplayHandle::Wayland(display_handle),
                raw_window_handle: RawWindowHandle::Wayland(window_handle),
            };

            let surface = match unsafe { renderer.instance.create_surface_unsafe(target) } {
                Ok(s) => s,
                Err(e) => {
                    warn!(name = output_name, "failed to create wgpu surface: {e}");
                    return;
                }
            };

            let surface_caps = surface.get_capabilities(&renderer.adapter);
            let alpha_mode = if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
                wgpu::CompositeAlphaMode::PreMultiplied
            } else {
                surface_caps.alpha_modes[0]
            };

            debug!(
                name = output_name,
                ?alpha_mode,
                formats = ?surface_caps.formats,
                "surface capabilities"
            );

            let surface_config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: wgpu::TextureFormat::Bgra8UnormSrgb,
                width: output_w,
                height: output_h,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            };

            surface.configure(&renderer.device, &surface_config);

            // Load wallpaper and optionally run depth estimation
            let wallpaper_path = self.config.wallpaper_for(&output_name).to_path_buf();

            // Load color texture
            let color_view = match renderer.load_wallpaper_texture(&wallpaper_path) {
                Ok(v) => v,
                Err(e) => {
                    warn!("failed to load wallpaper: {e:#}");
                    return;
                }
            };

            // Try to produce a depth map if a model is configured
            let depth_view_owned;
            let depth_view_ref = if let Some(model_path) = &self.config.general.model_path {
                match crate::depth::get_depth_map(&wallpaper_path, model_path) {
                    Ok(depth) => {
                        let (_tex, view) = renderer.upload_depth_map(&depth);
                        depth_view_owned = view;
                        &depth_view_owned
                    }
                    Err(e) => {
                        warn!("depth estimation failed, using flat wallpaper: {e:#}");
                        &renderer.depth_view
                    }
                }
            } else {
                debug!("no model_path configured, skipping depth estimation");
                &renderer.depth_view
            };

            let bind_group = renderer.create_bind_group(&color_view, depth_view_ref);

            let render_state = OutputRenderState {
                surface,
                config: surface_config,
                bind_group,
            };

            self.render_targets.insert(output_name.clone(), render_state);
            info!(name = output_name, w = output_w, h = output_h, "output fully initialized");

            layer.wl_surface().frame(qh, layer.wl_surface().clone());
            layer.wl_surface().commit();
        } else {
            if let Some(rt) = self.render_targets.get_mut(&output_name) {
                rt.config.width = output_w;
                rt.config.height = output_h;
                rt.surface.configure(&renderer.device, &rt.config);
                info!(name = output_name, w = output_w, h = output_h, "reconfigured swapchain");
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