//! The cart side of the wasmcart ABI: the structs the host reads and writes,
//! `wc_get_info`, and the per-frame driver that turns `wc_render` into one
//! `App::update`.
//!
//! Everything here lives in statics because the host reaches it by address.
//! The cart is single-threaded (wasmcart calls every entry point from one
//! thread), so plain `static mut` behind `addr_of_mut!` is sound as long as no
//! two `&mut` to the same static are alive at once, which the accessors below
//! keep to short scopes.

use core::ptr::{addr_of, addr_of_mut};
use core::time::Duration;

use bevy::app::{App, PluginsState};
use bevy::time::TimeUpdateStrategy;
use wasmcart::{WcHostInfo, WcInfo, WcPad, WcPointer, WcTime, WcWheel};

/// Which GPU API the cart renders through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gpu {
    /// No GPU: the cart draws into the 2D framebuffer (or draws nothing).
    Framebuffer,
    /// WebGPU, through the host's emdawnwebgpu import set.
    WebGpu,
}

/// Compile-time description of a cart, read by `wc_get_info` before any Bevy
/// code runs. Build it with [`CartConfig::new`] and the builder methods; they
/// are all `const`, so the config can live in a `const` item.
#[derive(Clone, Copy, Debug)]
pub struct CartConfig {
    /// Reported width in pixels.
    pub width: u32,
    /// Reported height in pixels.
    pub height: u32,
    /// GPU API.
    pub gpu: Gpu,
    /// Audio ring capacity in stereo frames; 0 means the cart is silent.
    pub audio_frames: u32,
    /// Audio sample rate the cart mixes at.
    pub audio_rate: u32,
    /// Extra `WC_FLAG_*` bits (pointer, keyboard).
    pub flags: u32,
    /// Export the named debug table (`WC_FLAG_DEBUG` + `wc_debug_state`).
    pub debug: bool,
}

impl CartConfig {
    /// A `width` x `height` cart with audio at 48 kHz and no GPU.
    pub const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            gpu: Gpu::Framebuffer,
            audio_frames: 8192,
            audio_rate: 48000,
            flags: 0,
            debug: false,
        }
    }
    /// Render with WebGPU (Bevy's renderer).
    pub const fn webgpu(mut self) -> Self {
        self.gpu = Gpu::WebGpu;
        self
    }
    /// Receive mouse and touch input (`WC_FLAG_POINTER`).
    pub const fn pointer(mut self) -> Self {
        self.flags |= wasmcart::WC_FLAG_POINTER;
        self
    }
    /// Receive raw keyboard input (`WC_FLAG_KEYBOARD`).
    pub const fn keyboard(mut self) -> Self {
        self.flags |= wasmcart::WC_FLAG_KEYBOARD;
        self
    }
    /// Emit no audio at all.
    pub const fn silent(mut self) -> Self {
        self.audio_frames = 0;
        self
    }
    /// Expose debug fields by name to a debugging host (see [`crate::debug`]).
    pub const fn debug(mut self) -> Self {
        self.debug = true;
        self
    }
}

#[repr(C)]
struct Shared {
    info: WcInfo,
    pads: [WcPad; 4],
    time: WcTime,
    host: WcHostInfo,
    pointers: [WcPointer; 10],
    keys: [u8; 32],
    wheel: WcWheel,
    audio_cursor: u32,
}

const ZERO_PAD: WcPad = WcPad {
    buttons: 0,
    left_x: 0,
    left_y: 0,
    right_x: 0,
    right_y: 0,
    left_trigger: 0,
    right_trigger: 0,
    connected: 0,
    _pad: [0; 3],
};
const ZERO_POINTER: WcPointer = WcPointer { x: 0, y: 0, buttons: 0, active: 0, _pad: [0; 2] };

static mut SHARED: Shared = Shared {
    info: WcInfo {
        version: wasmcart::WC_ABI_VERSION,
        width: 0,
        height: 0,
        fb_ptr: 0,
        audio_ptr: 0,
        audio_cap: 0,
        audio_write_ptr: 0,
        input_ptr: 0,
        save_ptr: 0,
        save_size: 0,
        time_ptr: 0,
        host_info_ptr: 0,
        flags: 0,
        audio_sample_rate: 0,
        pointer_ptr: 0,
        keys_ptr: 0,
        gpu_api: 0,
        wheel_ptr: 0,
    },
    pads: [ZERO_PAD; 4],
    time: WcTime { time_ms: 0.0, delta_ms: 0.0, frame: 0 },
    host: WcHostInfo {
        preferred_width: 0,
        preferred_height: 0,
        _reserved0: 0,
        audio_sample_rate: 0,
        flags: 0,
    },
    pointers: [ZERO_POINTER; 10],
    keys: [0; 32],
    wheel: WcWheel { dx: 0, dy: 0 },
    audio_cursor: 0,
};

static mut AUDIO: Vec<f32> = Vec::new();
static mut FB: Vec<u32> = Vec::new();
static mut CONFIG: Option<CartConfig> = None;

struct Runtime {
    app: App,
    ready: bool,
}
static mut RUNTIME: Option<Runtime> = None;

/// Gamepads, written by the host before each frame.
pub fn pads() -> &'static [WcPad; 4] {
    unsafe { &*addr_of!(SHARED.pads) }
}
/// Frame timing, written by the host before each frame.
pub fn time() -> &'static WcTime {
    unsafe { &*addr_of!(SHARED.time) }
}
/// Host preferences, written once before `wc_init`.
pub fn host_info() -> &'static WcHostInfo {
    unsafe { &*addr_of!(SHARED.host) }
}
/// Pointer slots (mouse is 0, touches 1-9). Needs [`CartConfig::pointer`].
pub fn pointers() -> &'static [WcPointer; 10] {
    unsafe { &*addr_of!(SHARED.pointers) }
}
/// Keyboard bitmask (USB HID scancodes). Needs [`CartConfig::keyboard`].
pub fn keys() -> &'static [u8; 32] {
    unsafe { &*addr_of!(SHARED.keys) }
}
/// Scroll wheel delta for this frame. Needs [`CartConfig::pointer`].
pub fn wheel() -> &'static WcWheel {
    unsafe { &*addr_of!(SHARED.wheel) }
}
/// The cart's configuration.
pub fn config() -> CartConfig {
    unsafe { (*addr_of!(CONFIG)).expect("wasmcart-bevy: config read before wc_get_info") }
}
/// Current reported size.
pub fn size() -> (u32, u32) {
    unsafe {
        let i = &*addr_of!(SHARED.info);
        (i.width, i.height)
    }
}

/// The 2D framebuffer (`width * height` pixels, `0xAABBGGRR` in memory, i.e.
/// a `u32` of `0xFFRRGGBB` reads as red in the high byte on little-endian
/// hosts). Empty for a WebGPU cart.
pub fn framebuffer() -> &'static mut [u32] {
    unsafe { &mut *addr_of_mut!(FB) }
}

/// Write `frames` stereo frames into the audio ring from `fill`, which gets a
/// slice of interleaved `[l, r, l, r, ...]` to fill.
pub(crate) fn audio_write(frames: usize, mut fill: impl FnMut(&mut [f32])) {
    unsafe {
        let ring = &mut *addr_of_mut!(AUDIO);
        let cap = ring.len() / 2;
        if cap == 0 {
            return;
        }
        let frames = frames.min(cap - 1);
        let cursor = (*addr_of!(SHARED.audio_cursor)) as usize;
        let first = frames.min(cap - cursor);
        fill(&mut ring[cursor * 2..(cursor + first) * 2]);
        if first < frames {
            fill(&mut ring[0..(frames - first) * 2]);
        }
        *addr_of_mut!(SHARED.audio_cursor) = ((cursor + frames) % cap) as u32;
    }
}

#[doc(hidden)]
pub fn get_info(cfg: &CartConfig) -> *mut WcInfo {
    unsafe {
        if (*addr_of!(CONFIG)).is_none() {
            *addr_of_mut!(CONFIG) = Some(*cfg);
            if cfg.gpu == Gpu::Framebuffer {
                *addr_of_mut!(FB) = vec![0xFF00_0000; (cfg.width * cfg.height) as usize];
            }
            if cfg.audio_frames > 0 {
                *addr_of_mut!(AUDIO) = vec![0.0; cfg.audio_frames as usize * 2];
            }
        }
        let s = &mut *addr_of_mut!(SHARED);
        let i = &mut s.info;
        i.version = wasmcart::WC_ABI_VERSION;
        i.width = cfg.width;
        i.height = cfg.height;
        i.fb_ptr = (*addr_of!(FB)).as_ptr() as u32;
        if cfg.gpu != Gpu::Framebuffer {
            i.fb_ptr = 0;
        }
        let audio = &*addr_of!(AUDIO);
        i.audio_ptr = if audio.is_empty() { 0 } else { audio.as_ptr() as u32 };
        i.audio_cap = (audio.len() / 2) as u32;
        i.audio_write_ptr = addr_of!(s.audio_cursor) as u32;
        i.input_ptr = addr_of!(s.pads) as u32;
        i.time_ptr = addr_of!(s.time) as u32;
        i.host_info_ptr = addr_of!(s.host) as u32;
        i.flags = cfg.flags
            | wasmcart::WC_FLAG_AUDIO_F32
            | if cfg.debug { wasmcart::WC_FLAG_DEBUG } else { 0 };
        i.audio_sample_rate = cfg.audio_rate;
        i.pointer_ptr = if cfg.flags & wasmcart::WC_FLAG_POINTER != 0 {
            addr_of!(s.pointers) as u32
        } else {
            0
        };
        i.wheel_ptr = if cfg.flags & wasmcart::WC_FLAG_POINTER != 0 {
            addr_of!(s.wheel) as u32
        } else {
            0
        };
        i.keys_ptr = if cfg.flags & wasmcart::WC_FLAG_KEYBOARD != 0 {
            addr_of!(s.keys) as u32
        } else {
            0
        };
        i.gpu_api = match cfg.gpu {
            Gpu::Framebuffer => wasmcart::WC_GPU_API_NONE,
            Gpu::WebGpu => wasmcart::WC_GPU_API_WEBGPU,
        };
        i as *mut WcInfo
    }
}

#[doc(hidden)]
pub fn init(cfg: &CartConfig, build: fn() -> App) {
    get_info(cfg);
    crate::logger::install();
    let app = build();
    unsafe {
        *addr_of_mut!(RUNTIME) = Some(Runtime { app, ready: false });
    }
    // Plugins that finish synchronously (everything but an async renderer
    // init) are ready now, so the first frame can run in the first wc_render.
    step_lifecycle();
}

/// Moves the app through finish/cleanup once every plugin is ready. Never
/// blocks: a plugin still waiting on an async result (WebGPU device creation)
/// is retried next frame, after the host has run its event loop.
fn step_lifecycle() -> bool {
    let rt = unsafe { (*addr_of_mut!(RUNTIME)).as_mut().expect("wc_init not called") };
    if rt.ready {
        return true;
    }
    match rt.app.plugins_state() {
        PluginsState::Adding => {
            bevy::tasks::tick_global_task_pools_on_main_thread();
            false
        }
        PluginsState::Ready => {
            rt.app.finish();
            rt.app.cleanup();
            rt.ready = true;
            true
        }
        PluginsState::Finished => {
            rt.app.cleanup();
            rt.ready = true;
            true
        }
        PluginsState::Cleaned => {
            rt.ready = true;
            true
        }
    }
}

#[doc(hidden)]
pub fn render() {
    if !step_lifecycle() {
        return;
    }
    let rt = unsafe { (*addr_of_mut!(RUNTIME)).as_mut().unwrap() };
    let t = time();
    let dt = if t.delta_ms.is_finite() && t.delta_ms > 0.0 { t.delta_ms } else { 0.0 };
    rt.app
        .world_mut()
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(dt / 1000.0)));
    rt.app.update();
    crate::debug::publish(rt.app.world_mut());
}
