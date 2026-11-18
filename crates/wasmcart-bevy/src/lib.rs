//! Run a [Bevy](https://bevy.org) app as a [wasmcart](https://github.com/wasmcart/wasmcart) cart.
//!
//! A wasmcart host calls `wc_render` once per frame; this crate turns each call
//! into one `App::update`, with Bevy's clock driven by the host's frame time,
//! the host's pads appearing as Bevy gamepads, and a software mixer feeding
//! the host's audio ring. With the `render` feature, Bevy's renderer draws
//! through the host's WebGPU device.
//!
//! ```ignore
//! use bevy::prelude::*;
//! use wasmcart_bevy::prelude::*;
//!
//! const CART: CartConfig = CartConfig::new(1280, 720).webgpu();
//!
//! fn app() -> App {
//!     let mut app = App::new();
//!     app.add_plugins(WasmcartPlugins).add_systems(Startup, setup);
//!     app
//! }
//!
//! wasmcart_bevy::cart!(CART, app);
//! ```
//!
//! Build for `wasm32-wasip1` (see the README for why, and for the Bevy
//! features to leave off).

pub mod abi;
pub mod audio;
pub mod debug;
pub mod input;
mod logger;
#[cfg(feature = "assets")]
pub mod assets;
#[cfg(feature = "render")]
pub mod render;

use bevy::app::{PluginGroup, PluginGroupBuilder};
use bevy::prelude::*;

pub use abi::{CartConfig, Gpu};

/// The usual imports.
pub mod prelude {
    pub use crate::abi::{CartConfig, Gpu};
    pub use crate::audio::{Audio, AudioClip, PlaySettings, VoiceId, Wave};
    pub use crate::debug::CartDebug;
    pub use crate::{CartInfo, WasmcartPlugin, WasmcartPlugins};
}

/// Cart facts a game may want: size, frame counter and the host clock.
#[derive(Resource, Clone, Copy, Debug)]
pub struct CartInfo {
    /// Reported width in pixels.
    pub width: u32,
    /// Reported height in pixels.
    pub height: u32,
    /// Host frame counter (`wc_time_t.frame`).
    pub frame: u32,
    /// Host clock in milliseconds (`wc_time_t.time_ms`).
    pub time_ms: f64,
    /// Audio mix rate.
    pub sample_rate: u32,
}

/// Wires a Bevy app to the cart: input, audio, debug fields and [`CartInfo`].
/// Add it after Bevy's own plugins (or use [`WasmcartPlugins`], which does).
pub struct WasmcartPlugin;

impl Plugin for WasmcartPlugin {
    fn build(&self, app: &mut App) {
        let cfg = abi::config();
        app.insert_resource(CartInfo {
            width: cfg.width,
            height: cfg.height,
            frame: 0,
            time_ms: 0.0,
            sample_rate: cfg.audio_rate,
        })
        .init_resource::<debug::CartDebug>()
        .add_plugins((input::WasmcartInputPlugin, audio::WasmcartAudioPlugin))
        .add_systems(First, update_cart_info);
    }
}

fn update_cart_info(mut info: ResMut<CartInfo>) {
    let t = abi::time();
    info.frame = t.frame;
    info.time_ms = t.time_ms;
}

/// Bevy's `DefaultPlugins`, configured for a cart, followed by
/// [`WasmcartPlugin`]: the primary window takes the cart's size, assets come
/// from the cart's own bundle (`assets` feature) and the renderer draws
/// through the host's WebGPU device (`render` feature).
///
/// Which of Bevy's plugins are in the group still follows the Bevy features
/// you enable, as with `DefaultPlugins`. Leave `bevy_winit`, `bevy_gilrs`,
/// `bevy_audio` and `bevy_log` off: this crate replaces all four.
pub struct WasmcartPlugins;

impl PluginGroup for WasmcartPlugins {
    fn build(self) -> PluginGroupBuilder {
        let cfg = abi::config();
        let mut group = DefaultPlugins.build();
        group = group.set(bevy::window::WindowPlugin {
            primary_window: Some(Window {
                resolution: bevy::window::WindowResolution::new(cfg.width, cfg.height)
                    .with_scale_factor_override(1.0),
                title: "wasmcart".into(),
                ..default()
            }),
            exit_condition: bevy::window::ExitCondition::DontExit,
            close_when_requested: false,
            ..default()
        });
        #[cfg(feature = "assets")]
        {
            group = group.add_before::<bevy::asset::AssetPlugin>(assets::WasmcartAssetSourcePlugin);
            group = group.set(assets::asset_plugin());
        }
        #[cfg(feature = "render")]
        {
            group = group.set(render::render_plugin());
            group = group.add(render::WasmcartRenderPlugin);
        }
        group.add(WasmcartPlugin)
    }
}

/// Declares the cart: emits `wc_get_info`, `wc_init` and `wc_render`.
///
/// ```ignore
/// wasmcart_bevy::cart!(CART, build_app);         // CART: CartConfig, build_app: fn() -> App
/// wasmcart_bevy::cart!(CART, build_app, debug);  // also export wc_debug_state
/// ```
///
/// With `debug`, `CART` must also say `.debug()`; without it the cart has no
/// debug surface at all, as wasmcart requires of a release cart.
#[macro_export]
macro_rules! cart {
    ($cfg:expr, $build:path) => {
        const __WASMCART_BEVY_CFG: $crate::CartConfig = $cfg;
        #[unsafe(no_mangle)]
        pub extern "C" fn wc_get_info() -> *mut ::core::ffi::c_void {
            $crate::abi::get_info(&__WASMCART_BEVY_CFG) as *mut ::core::ffi::c_void
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn wc_init() {
            $crate::abi::init(&__WASMCART_BEVY_CFG, $build)
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn wc_render() {
            $crate::abi::render()
        }
    };
    ($cfg:expr, $build:path, debug) => {
        $crate::cart!($cfg, $build);
        const _: () = assert!(
            __WASMCART_BEVY_CFG.debug,
            "wasmcart_bevy::cart!(.., debug) needs CartConfig::debug()"
        );
        #[unsafe(no_mangle)]
        pub extern "C" fn wc_debug_state() -> *const u8 {
            $crate::debug::debug_state()
        }
    };
}
