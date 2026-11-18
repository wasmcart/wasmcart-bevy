//! wasmcart input to Bevy input.
//!
//! - Pads: the four `wc_pad_t` slots become Bevy `Gamepad` entities, fed with
//!   the same raw messages `bevy_gilrs` writes, so `Query<&Gamepad>`,
//!   `gamepad.pressed(GamepadButton::South)` and `gamepad.left_stick()` work
//!   unchanged.
//! - Keyboard (cart declares [`CartConfig::keyboard`](crate::CartConfig::keyboard)):
//!   `KeyboardInput` messages, so `ButtonInput<KeyCode>` works.
//! - Pointer (cart declares [`CartConfig::pointer`](crate::CartConfig::pointer)):
//!   slot 0 is the mouse (`MouseButtonInput`, `CursorMoved`, `MouseWheel` and,
//!   with a window, the window's cursor position); slots 1-9 are touches
//!   (`TouchInput`, so `Touches` works).

use bevy::input::gamepad::{
    GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent,
    RawGamepadAxisChangedEvent, RawGamepadButtonChangedEvent, RawGamepadEvent,
};
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput};
use bevy::input::mouse::{MouseButton, MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::input::{ButtonState, InputSystems};
use bevy::prelude::*;
use wasmcart::*;

use crate::abi;

/// Pad button bits in the order they map to Bevy buttons.
const BUTTONS: [(u32, GamepadButton); 15] = [
    (WC_BTN_A, GamepadButton::South),
    (WC_BTN_B, GamepadButton::East),
    (WC_BTN_X, GamepadButton::West),
    (WC_BTN_Y, GamepadButton::North),
    (WC_BTN_L, GamepadButton::LeftTrigger),
    (WC_BTN_R, GamepadButton::RightTrigger),
    (WC_BTN_START, GamepadButton::Start),
    (WC_BTN_SELECT, GamepadButton::Select),
    (WC_BTN_UP, GamepadButton::DPadUp),
    (WC_BTN_DOWN, GamepadButton::DPadDown),
    (WC_BTN_LEFT, GamepadButton::DPadLeft),
    (WC_BTN_RIGHT, GamepadButton::DPadRight),
    (WC_BTN_L3, GamepadButton::LeftThumb),
    (WC_BTN_R3, GamepadButton::RightThumb),
    (WC_BTN_GUIDE, GamepadButton::Mode),
];

#[derive(Default, Clone, Copy)]
struct PadSnapshot {
    entity: Option<Entity>,
    buttons: u32,
    lt: i16,
    rt: i16,
    axes: [i16; 4],
}

#[derive(Resource, Default)]
struct PadState {
    pads: [PadSnapshot; 4],
}

#[derive(Resource, Default)]
struct KeyState {
    prev: [u8; 32],
}

#[derive(Resource, Default)]
struct PointerState {
    prev: [WcPointer; 10],
}

/// Turns wasmcart input into Bevy input messages. Part of
/// [`WasmcartPlugin`](crate::WasmcartPlugin).
pub struct WasmcartInputPlugin;

impl Plugin for WasmcartInputPlugin {
    fn build(&self, app: &mut App) {
        let cfg = abi::config();
        app.init_resource::<PadState>()
            .add_systems(PreUpdate, pad_system.before(InputSystems));
        if cfg.flags & WC_FLAG_KEYBOARD != 0 {
            app.init_resource::<KeyState>()
                .add_systems(PreUpdate, keyboard_system.before(InputSystems));
        }
        if cfg.flags & WC_FLAG_POINTER != 0 {
            app.init_resource::<PointerState>()
                .add_systems(PreUpdate, pointer_system.before(InputSystems));
        }
    }
}

fn stick(v: i16) -> f32 {
    (v as f32 / 32767.0).clamp(-1.0, 1.0)
}
fn trigger(v: i16) -> f32 {
    (v as f32 / WC_TRIGGER_MAX as f32).clamp(0.0, 1.0)
}

fn pad_system(
    mut commands: Commands,
    mut state: ResMut<PadState>,
    mut raw: MessageWriter<RawGamepadEvent>,
    mut connections: MessageWriter<GamepadConnectionEvent>,
    mut buttons: MessageWriter<RawGamepadButtonChangedEvent>,
    mut axes: MessageWriter<RawGamepadAxisChangedEvent>,
) {
    let pads = abi::pads();
    for (i, pad) in pads.iter().enumerate() {
        let snap = &mut state.pads[i];
        let connected = pad.is_connected();
        match (connected, snap.entity) {
            (true, None) => {
                let e = commands.spawn_empty().id();
                *snap = PadSnapshot { entity: Some(e), ..default() };
                let mut name = [0u8; 64];
                let n = wc_pad_name(i as u32, &mut name);
                let name = if n > 0 {
                    String::from_utf8_lossy(&name[..n]).into_owned()
                } else {
                    format!("wasmcart pad {i}")
                };
                let ev = GamepadConnectionEvent::new(
                    e,
                    GamepadConnection::Connected { name, vendor_id: None, product_id: None },
                );
                raw.write(ev.clone().into());
                connections.write(ev);
            }
            (false, Some(e)) => {
                let ev = GamepadConnectionEvent::new(e, GamepadConnection::Disconnected);
                raw.write(ev.clone().into());
                connections.write(ev);
                *snap = PadSnapshot::default();
                continue;
            }
            (false, None) => continue,
            (true, Some(_)) => {}
        }
        let e = snap.entity.unwrap();
        let changed = pad.buttons ^ snap.buttons;
        for (bit, button) in BUTTONS {
            if changed & bit != 0 {
                let v = if pad.buttons & bit != 0 { 1.0 } else { 0.0 };
                raw.write(RawGamepadButtonChangedEvent::new(e, button, v).into());
                buttons.write(RawGamepadButtonChangedEvent::new(e, button, v));
            }
        }
        for (now, prev, button) in [
            (pad.left_trigger, &mut snap.lt, GamepadButton::LeftTrigger2),
            (pad.right_trigger, &mut snap.rt, GamepadButton::RightTrigger2),
        ] {
            if now != *prev {
                *prev = now;
                let v = trigger(now);
                raw.write(RawGamepadButtonChangedEvent::new(e, button, v).into());
                buttons.write(RawGamepadButtonChangedEvent::new(e, button, v));
            }
        }
        // wasmcart sticks are SDL-style (+y down); Bevy's are +y up.
        let now = [pad.left_x, pad.left_y, pad.right_x, pad.right_y];
        let names = [
            (GamepadAxis::LeftStickX, false),
            (GamepadAxis::LeftStickY, true),
            (GamepadAxis::RightStickX, false),
            (GamepadAxis::RightStickY, true),
        ];
        for k in 0..4 {
            if now[k] != snap.axes[k] {
                snap.axes[k] = now[k];
                let (axis, flip) = names[k];
                let v = if flip { -stick(now[k]) } else { stick(now[k]) };
                raw.write(RawGamepadAxisChangedEvent::new(e, axis, v).into());
                axes.write(RawGamepadAxisChangedEvent::new(e, axis, v));
            }
        }
        snap.buttons = pad.buttons;
    }
}

/// The primary window if there is one, else a placeholder (input messages
/// carry a window entity, but nothing requires it to exist).
fn window_entity(world_windows: &Option<Single<Entity, With<bevy::window::PrimaryWindow>>>) -> Entity {
    world_windows.as_ref().map(|w| **w).unwrap_or(Entity::PLACEHOLDER)
}

fn keyboard_system(
    mut state: ResMut<KeyState>,
    window: Option<Single<Entity, With<bevy::window::PrimaryWindow>>>,
    mut out: MessageWriter<KeyboardInput>,
) {
    let keys = abi::keys();
    let window = window_entity(&window);
    for byte in 0..32 {
        let diff = keys[byte] ^ state.prev[byte];
        if diff == 0 {
            continue;
        }
        for bit in 0..8 {
            if diff & (1 << bit) == 0 {
                continue;
            }
            let code = (byte * 8 + bit) as u8;
            let Some(key_code) = hid_to_keycode(code) else { continue };
            let pressed = keys[byte] & (1 << bit) != 0;
            out.write(KeyboardInput {
                key_code,
                logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
                state: if pressed { ButtonState::Pressed } else { ButtonState::Released },
                text: None,
                repeat: false,
                window,
            });
        }
        state.prev[byte] = keys[byte];
    }
}

fn pointer_system(
    mut state: ResMut<PointerState>,
    window: Option<Single<Entity, With<bevy::window::PrimaryWindow>>>,
    mut windows: Query<&mut Window>,
    mut buttons: MessageWriter<MouseButtonInput>,
    mut moved: MessageWriter<bevy::window::CursorMoved>,
    mut wheel_out: MessageWriter<MouseWheel>,
    mut touches: MessageWriter<TouchInput>,
) {
    let window_e = window_entity(&window);
    let ptrs = abi::pointers();
    // Slot 0: the mouse.
    let m = ptrs[0];
    let pm = state.prev[0];
    let pos = Vec2::new(m.x as f32, m.y as f32);
    if m.is_active() && (m.x != pm.x || m.y != pm.y || !pm.is_active()) {
        let delta = pm.is_active().then(|| pos - Vec2::new(pm.x as f32, pm.y as f32));
        moved.write(bevy::window::CursorMoved { window: window_e, position: pos, delta });
        if let Ok(mut w) = windows.get_mut(window_e) {
            w.set_physical_cursor_position(Some(pos.as_dvec2()));
        }
    }
    let diff = m.buttons ^ pm.buttons;
    for (bit, button) in [(1u8, MouseButton::Left), (2, MouseButton::Right), (4, MouseButton::Middle)] {
        if diff & bit != 0 {
            buttons.write(MouseButtonInput {
                button,
                state: if m.buttons & bit != 0 { ButtonState::Pressed } else { ButtonState::Released },
                window: window_e,
            });
        }
    }
    let wh = abi::wheel();
    if wh.dx != 0 || wh.dy != 0 {
        wheel_out.write(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: wh.dx as f32 / WC_WHEEL_DELTA as f32,
            y: wh.dy as f32 / WC_WHEEL_DELTA as f32,
            window: window_e,
            phase: TouchPhase::Moved,
        });
    }
    // Slots 1-9: touches.
    for i in 1..10 {
        let (now, prev) = (ptrs[i], state.prev[i]);
        let position = Vec2::new(now.x as f32, now.y as f32);
        let phase = match (prev.is_active(), now.is_active()) {
            (false, true) => TouchPhase::Started,
            (true, true) if now.x != prev.x || now.y != prev.y => TouchPhase::Moved,
            (true, false) => TouchPhase::Ended,
            _ => continue,
        };
        let position = if phase == TouchPhase::Ended {
            Vec2::new(prev.x as f32, prev.y as f32)
        } else {
            position
        };
        touches.write(TouchInput { phase, position, window: window_e, force: None, id: i as u64 });
    }
    state.prev = *ptrs;
}

/// USB HID usage (keyboard page) to Bevy's physical `KeyCode`.
pub fn hid_to_keycode(code: u8) -> Option<KeyCode> {
    use KeyCode::*;
    const LETTERS: [KeyCode; 26] = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ];
    const DIGITS: [KeyCode; 10] =
        [Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9, Digit0];
    const FKEYS: [KeyCode; 12] = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    Some(match code {
        0x04..=0x1D => LETTERS[(code - 0x04) as usize],
        0x1E..=0x27 => DIGITS[(code - 0x1E) as usize],
        0x28 => Enter,
        0x29 => Escape,
        0x2A => Backspace,
        0x2B => Tab,
        0x2C => Space,
        0x2D => Minus,
        0x2E => Equal,
        0x2F => BracketLeft,
        0x30 => BracketRight,
        0x31 => Backslash,
        0x33 => Semicolon,
        0x34 => Quote,
        0x35 => Backquote,
        0x36 => Comma,
        0x37 => Period,
        0x38 => Slash,
        0x39 => CapsLock,
        0x3A..=0x45 => FKEYS[(code - 0x3A) as usize],
        0x46 => PrintScreen,
        0x47 => ScrollLock,
        0x48 => Pause,
        0x49 => Insert,
        0x4A => Home,
        0x4B => PageUp,
        0x4C => Delete,
        0x4D => End,
        0x4E => PageDown,
        0x4F => ArrowRight,
        0x50 => ArrowLeft,
        0x51 => ArrowDown,
        0x52 => ArrowUp,
        0xE0 => ControlLeft,
        0xE1 => ShiftLeft,
        0xE2 => AltLeft,
        0xE3 => SuperLeft,
        0xE4 => ControlRight,
        0xE5 => ShiftRight,
        0xE6 => AltRight,
        0xE7 => SuperRight,
        _ => return None,
    })
}
