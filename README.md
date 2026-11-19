# wasmcart-bevy

**Write games in Bevy. Ship them as wasmcart carts.**

[Bevy](https://bevy.org) 0.20 running as a [wasmcart](https://github.com/wasmcart/wasmcart)
cartridge: the ECS, 2D sprites and text, 3D PBR with shadows, gamepads and
sound, in one `.wasc` that runs on any wasmcart host with WebGPU. No browser,
no `wasm-bindgen`, no JavaScript glue: the cart's imports are the wasmcart set
(`env.wc_*`, the WebGPU imports and a few WASI calls) and nothing else.

| Cart | Size | What it shows |
|---|---|---|
| `hello-ecs` | 320x240 | Bevy's ECS, pad input and sound with no GPU at all: squares in the 2D framebuffer |
| `star-catcher` | 640x360 | a 2D game: sprites and a WAV loaded from the cart's assets, `Mesh2d`, `Text2d`, a score |
| `lit-scene` | 960x540 | Bevy's PBR renderer: a shadowed sun, coloured point lights, metal and plastic, a pad-orbited camera |

## How it fits together

```
your Bevy app
  WasmcartPlugins        DefaultPlugins configured for a cart, plus:
    runner               wc_render -> App::update(), Bevy's clock = the host's frame time
    input                wasmcart pads -> Bevy Gamepad entities (same messages bevy_gilrs writes)
                         keyboard and pointer -> KeyboardInput, mouse, touch (when the cart asks)
    audio                a mixer writing the host's audio ring (bevy_audio needs cpal/rodio)
    assets               the default asset source reads the .wasc (wc_load_asset)
    render               RenderCreation::Manual with the host's WebGPU device
  wasmcart-wgpu          wgpu's custom backend over the wasmcart WebGPU imports
  wasmcart host          owns the device, presents the canvas, plays the audio
```

The renderer is Bevy's own, unmodified: wgpu hands every WGSL shader, pipeline
and draw to the host's WebGPU implementation through
[wasmcart-wgpu](https://github.com/wasmcart/wasmcart-rust/tree/main/wasmcart-wgpu).

## A whole cart

```rust
use bevy::prelude::*;
use wasmcart_bevy::prelude::*;

const CART: CartConfig = CartConfig::new(1280, 720).webgpu();

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(WasmcartPlugins)
        .add_systems(Startup, setup)
        .add_systems(Update, play);
    app
}

wasmcart_bevy::cart!(CART, app);

fn setup(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn(Camera2d);
    commands.spawn(Sprite::from_image(assets.load("ship.png")));
}

fn play(pads: Query<&Gamepad>, audio: Res<Audio>, mut beep: Local<Option<AudioClip>>) {
    let beep = beep.get_or_insert_with(|| AudioClip::tone(660.0, 0.1, Wave::Square));
    if pads.iter().any(|p| p.just_pressed(GamepadButton::South)) {
        audio.play(beep);
    }
}
```

`cart!` emits `wc_get_info`, `wc_init` and `wc_render`. You write a function
that builds the `App`; you never call `App::run`. The host calls `wc_render`
once a frame and each call is one `App::update`.

`CartConfig` options: `.webgpu()` (render with Bevy; without it the cart has a
2D framebuffer, `wasmcart_bevy::abi::framebuffer()`), `.pointer()`,
`.keyboard()`, `.silent()`, `.debug()` (named fields for a debugger, below),
`.prefer_core()` (below).

## Core and compatibility-mode WebGPU

A wasmcart host hands a cart a core WebGPU device where the GPU has one, and
otherwise a **compatibility-mode** device (the GLES 3.1 / D3D11 class floor);
a host can also be told to use compatibility mode. Every cart here runs on
both, unchanged. [wasmcart-wgpu](https://github.com/wasmcart/wasmcart-rust/tree/main/wasmcart-wgpu)
describes a compatibility-mode device as it is (no storage buffers in the
vertex stage, no cube-array textures, 128 compute invocations ...), and Bevy
picks its own downlevel paths from that: mesh data in uniform buffers,
clustered lighting on the CPU, no GPU preprocessing. The canvas is an
emulated `Rgba8UnormSrgb` surface, so colours are the same as on core.

`CartConfig::prefer_core()` asks the host for a core device in `wc_init`
(`core-features-and-limits`) and builds the app once the answer arrives, a
frame or two later: a 3D game that wants core's features gets them wherever
the GPU has them, even on a host that defaults to compatibility mode, and
still runs in compatibility mode where it does not. The `gpu.core` debug
field says which device the renderer got.

## Cargo setup

```toml
[package]
name = "my-game"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
wasmcart-bevy = { git = "https://github.com/wasmcart/wasmcart-bevy", features = ["render"] }
bevy = { version = "0.20.1", default-features = false, features = [
    "std", "bevy_asset", "bevy_render", "bevy_core_pipeline",
    "bevy_sprite", "bevy_sprite_render", "bevy_text", "default_font", "png",
    # 3D: "bevy_pbr", "bevy_light", "tonemapping_luts", "ktx2", "zstd_rust"
] }

# Required: see "Why three patched Bevy crates" below.
[patch.crates-io]
bevy_asset = { git = "https://github.com/wasmcart/wasmcart-bevy" }
bevy_render = { git = "https://github.com/wasmcart/wasmcart-bevy" }
bevy_log = { git = "https://github.com/wasmcart/wasmcart-bevy" }

[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Leave these Bevy features **off**; wasmcart-bevy replaces each: `bevy_winit`
(the host owns the window), `bevy_gilrs` (pads), `bevy_audio` (sound),
`bevy_log` (logs go to `wc_log`), `multi_threaded`, `webgl2`, `webgpu`, `web`.
Bevy's `default` features turn all of them on, so always set
`default-features = false`.

Build and pack:

```sh
rustup target add wasm32-wasip1
cargo build --release --target wasm32-wasip1
npx -p wasmcart wasmcart-pack --wasm target/wasm32-wasip1/release/my_game.wasm \
  --assets assets --name my-game --version 0.1.0 --width 1280 --height 720 -o my-game.wasc
npx wasmcart my-game.wasc
```

### Why wasm32-wasip1

`std::time::Instant` panics on `wasm32-unknown-unknown`, and Bevy reads it
everywhere (`Time<Real>`, the render world's clock). On `wasm32-wasip1` it is
WASI's `clock_time_get`, which every wasmcart host provides, and `getrandom`
is WASI's `random_get`. The cart still opens no files: wasmcart hosts give WASI
no filesystem, and assets come through `wc_load_asset`. Bevy's game clock
(`Time`) is not the WASI clock anyway: it advances by the host's `delta_ms`
each frame, so a deterministic host steps a Bevy game exactly.

### Why three patched Bevy crates

`bevy_asset`, `bevy_render` and `bevy_log` turn on Bevy's browser code paths
(the `web` feature of `bevy_platform`, `bevy_app` and `bevy_reflect`) for every
`target_arch = "wasm32"` target, wasip1 included. That swaps the clock for
`performance.now()` through wasm-bindgen and the task executor for
`wasm-bindgen-futures`: imports no wasmcart host provides. `bevy-patches/` holds
those three crates exactly as released on crates.io with that one manifest
entry removed; `scripts/vendor-bevy.sh` regenerates them, and each carries a
`WASMCART-PATCH.txt` saying so. No Bevy source code is changed.

## Input

The four wasmcart pads are Bevy `Gamepad` entities, connected and disconnected
as the host reports them, named by `wc_pad_name`:

| wasmcart | Bevy `GamepadButton` / `GamepadAxis` |
|---|---|
| A B X Y | `South` `East` `West` `North` |
| L R | `LeftTrigger` `RightTrigger` |
| left/right trigger (analog) | `LeftTrigger2` `RightTrigger2` (value 0..1) |
| Start Select Guide | `Start` `Select` `Mode` |
| d-pad | `DPadUp` `DPadDown` `DPadLeft` `DPadRight` |
| L3 R3 | `LeftThumb` `RightThumb` |
| sticks | `LeftStickX/Y` `RightStickX/Y` (Bevy's +y is up; wasmcart's +y down is flipped) |

With `.keyboard()`, the host's key bitmask becomes `KeyboardInput` messages
(`ButtonInput<KeyCode>` works). With `.pointer()`, slot 0 is the mouse
(`MouseButtonInput`, `CursorMoved`, `MouseWheel`, the window's cursor
position) and slots 1-9 are touches (`TouchInput`, `Touches`).

## Audio

`Res<Audio>` plays `AudioClip`s through a small mixer that fills exactly the
samples each frame's `delta_ms` covers:

```rust
audio.play(&clip);
let id = audio.play_with(&music, PlaySettings { volume: 0.5, looped: true, ..default() });
audio.stop(id);
```

Clips come from WAV files (`assets.load::<AudioClip>("hit.wav")`, or
`AudioClip::from_wav(bytes)`: 8/16/24/32-bit PCM or float, any rate, mono or
stereo, resampled to the mix rate) or from `AudioClip::tone` / `sweep` (sine,
square, saw, triangle, noise) for quick effects.

## Debug fields

With `.debug()` and `cart!(CART, app, debug)` the cart exports wasmcart's named
debug table. Systems publish values with `ResMut<CartDebug>`:
`dbg.set("player_x", x)`. A debugging host can also write a field (romdev's
`wasm({op:'write', name, value})`); `dbg.get(name)` then returns the written
value, so a test switch such as an autopilot is a field the cart sets once and
reads each frame. Five fields are always there: `bevy.frame`,
`bevy.entities`, `bevy.ready` (1 once the renderer is up), `host.delta_ms`
and `gpu.core` (1 on a core WebGPU device, 0 in compatibility mode or with
no GPU).
Leave `.debug()` off in a release cart: it then exports no debug surface.

## This repo

```sh
scripts/build.sh            # build every example into dist/*.wasc
scripts/build.sh lit-scene  # one
node test/run.mjs           # the gate: plays each cart through romdev
```

`test/run.mjs` drives the carts through a romdev MCP
server (`ROMDEV_URL`, default `http://127.0.0.1:7331/mcp`): it loads each
cart, plays it with the pad, reads its debug fields, screenshots it and reads
its audio ring, and checks what moved, what drew and what sounded. Every cart
runs on a core device and again on a compatibility-mode device; each run
checks which level it got (`gpu.core`), the compatibility-mode pictures are
compared with the core ones (same colours), and `lit-scene-core` (lit-scene
built with `.prefer_core()`) must get a core device on a compatibility-mode
host. Each check has a control (no input, nothing that should sound, a frame
without sRGB encoding) that must fail it.

The art and sounds in `examples/star-catcher/assets` are drawn by
`examples/star-catcher/tools/make-assets.mjs`; the other carts synthesize
everything they show and play.

## Known gaps

- In compatibility mode a point light can cast shadows only while it is the
  only shadow-casting point light (compat has no cube-array textures, and
  wasmcart-wgpu turns a cube array of one cube into a cube). Use
  `.prefer_core()` for a scene with several.
- Carts are large: 6 MB of wasm for the ECS-only cart, 18-19 MB for the 2D
  and 3D carts (1.5, 4.7 and 5.2 MB as `.wasc`). Most of it is Bevy's
  renderer and shader tooling.
- Bevy's audio API (`AudioPlayer`, `bevy_audio`) is not available; use
  `Res<Audio>`. Ogg/MP3 decoding is not built in.
- No multi-threading: Bevy runs single-threaded (wasmcart threads exist, but
  Bevy's multi-threaded executor is not wired to them yet).
- The cart does not follow host resizes; it renders at its `CartConfig` size.
- Asset directories cannot be listed (`load_folder`), and there are no `.meta`
  files.

## Related

- [wasmcart-rust](https://github.com/wasmcart/wasmcart-rust): the Rust crates
  for wasmcart, including [wasmcart-wgpu](https://github.com/wasmcart/wasmcart-rust/tree/main/wasmcart-wgpu),
  the wgpu backend this crate renders through.
- [wasmcart-macroquad](https://github.com/wasmcart/wasmcart-macroquad):
  macroquad games as wasmcart carts.
- [wasmcart-rust-examples](https://github.com/wasmcart/wasmcart-rust-examples):
  example Rust carts.

## License

MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE), like Bevy. The vendored crates in `bevy-patches/` are Bevy's,
under Bevy's license.
