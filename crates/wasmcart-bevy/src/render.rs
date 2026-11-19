//! Bevy's renderer on the host's WebGPU device.
//!
//! wgpu reaches the GPU through [`wasmcart_wgpu`], a wgpu `custom` backend over
//! the wasmcart WebGPU imports (the emdawnwebgpu set). The host owns the
//! device and the presentation surface; the cart takes both synchronously in
//! `wc_init` and hands them to Bevy as `RenderCreation::Manual`, so nothing
//! waits on a browser-style async adapter request.
//!
//! On a compatibility-mode device (the wasmcart tier's floor) wasmcart-wgpu
//! reports the device's real limits and downlevel flags, so Bevy takes its
//! downlevel paths by itself: no storage buffers in the vertex stage (mesh
//! data in uniforms), clustering on the CPU, no GPU preprocessing. The canvas
//! is an emulated `Rgba8UnormSrgb` surface, which Bevy prefers, so colours
//! match a core device. A cart built with [`CartConfig::prefer_core`](crate::CartConfig::prefer_core)
//! asks for a core device first (see [`start_core_request`]).
//!
//! The primary window gets a `RawHandleWrapper` naming the host canvas, which
//! is all Bevy's `create_surfaces` needs to configure the surface; the host
//! presents after `wc_render` returns.

use core::future::Future;
use core::pin::Pin;
use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicU32, Ordering};
use core::task::Poll;

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

/// `RenderPlugin` configured with the host's device (or the core device a
/// [`CartConfig::prefer_core`](crate::CartConfig::prefer_core) cart got).
pub fn render_plugin() -> RenderPlugin {
    let chosen = unsafe { (*addr_of_mut!(CORE_DEVICE)).take() };
    let (instance, adapter, device, queue) = chosen
        .or_else(host_device)
        .expect("wasmcart-bevy: the host gave no WebGPU device (is this a WebGPU host?)");
    GPU_CORE.store(wasmcart_wgpu::is_core(&device) as u32, Ordering::Relaxed);
    NO_STORAGE.store(bevy::render::storage_buffers_are_unsupported(&device.limits()), Ordering::Relaxed);
    let info = adapter.get_info();
    log::info!(
        "wasmcart-bevy: WebGPU adapter {:?} ({:?}), {} device",
        info.name,
        info.device_type,
        if wasmcart_wgpu::is_core(&device) { "core" } else { "compatibility-mode" }
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
        if NO_STORAGE.load(Ordering::Relaxed) {
            no_gpu_preprocessing(app);
        }
    }
}

/// Without storage buffers in every stage (a compatibility-mode device,
/// which has none in the vertex stage) Bevy's meshes read their data from
/// uniform buffers, but Bevy still turns GPU preprocessing on wherever there
/// are compute shaders, and that writes the mesh data into storage buffers
/// the uniform bindings then reject. Turn it off, as on WebGL 2: Bevy's own
/// plugins read the resource when they finish (after this build), and its
/// render-startup system that re-derives it is followed by one that turns it
/// off again.
fn no_gpu_preprocessing(app: &mut App) {
    use bevy::render::batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport};
    use bevy::render::{RenderApp, RenderStartup};
    fn none(mut commands: Commands) {
        commands.insert_resource(GpuPreprocessingSupport { max_supported_mode: GpuPreprocessingMode::None });
    }
    log::info!("wasmcart-bevy: no storage buffers in the vertex stage; GPU preprocessing off");
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app
            .insert_resource(GpuPreprocessingSupport { max_supported_mode: GpuPreprocessingMode::None })
            .add_systems(
                RenderStartup,
                none.after(bevy::render::init_gpu_resource::<GpuPreprocessingSupport>),
            );
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

/// 1 when the renderer runs on a core WebGPU device, 0 in compatibility mode
/// (published as the `gpu.core` debug field).
pub(crate) static GPU_CORE: AtomicU32 = AtomicU32::new(0);
/// The device has no storage buffers in some stage (see no_gpu_preprocessing).
static NO_STORAGE: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

type Parts = (wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue);
type CoreFuture = Pin<Box<dyn Future<Output = Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError>>>>;

// The cart is single-threaded (every entry point runs on one thread).
static mut CORE_REQUEST: Option<(wgpu::Instance, CoreFuture)> = None;
static mut CORE_DEVICE: Option<Parts> = None;

/// For a [`CartConfig::prefer_core`](crate::CartConfig::prefer_core) cart:
/// ask for a core device (the host's own answers at once if it is core)
/// (`core-features-and-limits`, wasmcart SPEC.md "Feature level") with
/// WebGPU's core default limits. Returns true while that request is out; the
/// answer arrives between frames ([`poll_core_request`]).
pub(crate) fn start_core_request() -> bool {
    let instance = wasmcart_wgpu::instance();
    let Some(Ok(adapter)) = wasmcart_wgpu::now(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        return false;
    };
    let desc = wgpu::DeviceDescriptor {
        label: Some("wasmcart-bevy core"),
        required_features: adapter.features(),
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    };
    let fut: CoreFuture = Box::pin(wasmcart_wgpu::request_core(&adapter, &desc));
    unsafe { *addr_of_mut!(CORE_REQUEST) = Some((instance, fut)) };
    true
}

/// Poll the core request: true once it is answered. A core device is kept for
/// [`render_plugin`]; a refusal (no core on this GPU) leaves the host's
/// compatibility-mode device to it.
pub(crate) fn poll_core_request() -> bool {
    let slot = unsafe { &mut *addr_of_mut!(CORE_REQUEST) };
    let Some((_, fut)) = slot.as_mut() else { return true };
    match wasmcart_wgpu::poll_once(fut.as_mut()) {
        Poll::Pending => false,
        Poll::Ready(r) => {
            let (instance, _) = slot.take().unwrap();
            match r {
                Ok((device, queue)) => {
                    let adapter = wasmcart_wgpu::adapter_for(&device);
                    unsafe { *addr_of_mut!(CORE_DEVICE) = Some((instance, adapter, device, queue)) };
                }
                Err(e) => log::info!("wasmcart-bevy: no core WebGPU device ({e}); using the compatibility-mode one"),
            }
            true
        }
    }
}
