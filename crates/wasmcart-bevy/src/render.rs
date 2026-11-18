//! Bevy's renderer on the host's WebGPU device.
//!
//! wgpu reaches the GPU through [`wasmcart_wgpu`], a wgpu `custom` backend over
//! the wasmcart WebGPU imports (the emdawnwebgpu set). The host owns the
//! device and the presentation surface; the cart takes both synchronously in
//! `wc_init` and hands them to Bevy as `RenderCreation::Manual`, so nothing
//! waits on a browser-style async adapter request.
//!
//! The primary window gets a `RawHandleWrapper` naming the host canvas, which
//! is all Bevy's `create_surfaces` needs to configure the surface; the host
//! presents after `wc_render` returns.

use bevy::prelude::*;
use wasmcart_wgpu::wgpu;
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue,
};
use bevy::render::settings::{RenderCreation, RenderResources};
use bevy::render::RenderPlugin;
use bevy::window::{PrimaryWindow, RawHandleWrapper, WindowWrapper};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WebDisplayHandle, WebWindowHandle, WindowHandle,
};

/// The host's canvas, as a window handle. wasmcart has exactly one surface
/// (the `"#canvas"` selector), so the handle carries no data the backend needs.
struct HostCanvas;

impl HasWindowHandle for HostCanvas {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Web(WebWindowHandle::new(1));
        // SAFETY: the handle is a constant id that refers to nothing in the
        // cart's memory; the backend maps every handle to the host canvas.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for HostCanvas {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Web(WebDisplayHandle::new());
        // SAFETY: as above.
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

/// `RenderPlugin` configured with the host's device.
pub fn render_plugin() -> RenderPlugin {
    let (instance, adapter, device, queue) = host_device()
        .expect("wasmcart-bevy: the host gave no WebGPU device (is this a WebGPU host?)");
    let info = adapter.get_info();
    log::info!(
        "wasmcart-bevy: WebGPU adapter {:?} ({:?}), backend {:?}",
        info.name,
        info.device_type,
        info.backend
    );
    RenderPlugin {
        render_creation: RenderCreation::Manual(Box::new(RenderResources(
            RenderDevice::from(device),
            RenderQueue::new(queue),
            RenderAdapterInfo::new(info),
            RenderAdapter::new(adapter),
            RenderInstance::new(instance),
        ))),
        // Pipelines compile on the main thread either way on wasm; ask for it
        // so the first frame that can draw does.
        synchronous_pipeline_compilation: true,
        ..default()
    }
}

/// Gives the primary window its surface handle. Part of
/// [`WasmcartPlugins`](crate::WasmcartPlugins) with the `render` feature.
pub struct WasmcartRenderPlugin;

impl Plugin for WasmcartRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, attach_canvas)
            .add_systems(First, attach_canvas);
    }
}

fn attach_canvas(
    mut commands: Commands,
    windows: Query<Entity, (With<PrimaryWindow>, Without<RawHandleWrapper>)>,
) {
    for e in &windows {
        let wrapper = WindowWrapper::new(HostCanvas);
        match RawHandleWrapper::new(&wrapper) {
            Ok(h) => {
                commands.entity(e).insert(h);
            }
            Err(err) => log::error!("wasmcart-bevy: no surface handle: {err}"),
        }
    }
}

/// The host's device, taken synchronously: the requests below ask for nothing
/// the host device lacks, so they finish on their first poll.
fn host_device() -> Option<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let instance = wasmcart_wgpu::instance();
    let adapter = wasmcart_wgpu::now(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?
        .ok()?;
    // Ask for exactly what the host device has, so Bevy sees its real limits
    // (storage buffers decide clustered lighting, for one) and the request is
    // still met by the host device itself.
    let desc = wgpu::DeviceDescriptor {
        label: Some("wasmcart-bevy"),
        required_features: adapter.features(),
        required_limits: adapter.limits(),
        ..Default::default()
    };
    let (device, queue) = wasmcart_wgpu::now(adapter.request_device(&desc))?.ok()?;
    Some((instance, adapter, device, queue))
}
