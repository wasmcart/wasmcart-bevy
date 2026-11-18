//! Bevy's ECS with no renderer: entities are coloured squares drawn into the
//! cart's 2D framebuffer by a plain system. Pad 1 steers the white square,
//! A chirps and spawns a bouncing square, B clears them. Arrow keys also steer,
//! and a click or a touch puts the white square where it lands.

use bevy::prelude::*;
use wasmcart_bevy::prelude::*;

const W: u32 = 320;
const H: u32 = 240;
const CART: CartConfig = CartConfig::new(W, H).keyboard().pointer().debug();

#[derive(Component)]
struct Player;

#[derive(Component)]
struct Velocity(Vec2);

#[derive(Component)]
struct Pos(Vec2);

#[derive(Component)]
struct Tint(u32);

#[derive(Resource)]
struct Sounds {
    spawn: AudioClip,
    clear: AudioClip,
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(WasmcartPlugins)
        .add_systems(Startup, setup)
        .add_systems(Update, (steer, keys, click, spawn_or_clear, bounce, publish).chain())
        .add_systems(PostUpdate, draw);
    app
}

wasmcart_bevy::cart!(CART, app, debug);

fn setup(mut commands: Commands) {
    commands.spawn((Player, Pos(Vec2::new(W as f32 / 2.0, H as f32 / 2.0)), Tint(0xFFFF_FFFF)));
    commands.insert_resource(Sounds {
        spawn: AudioClip::sweep(440.0, 880.0, 0.12, Wave::Square),
        clear: AudioClip::sweep(300.0, 80.0, 0.25, Wave::Noise),
    });
}

fn steer(time: Res<Time>, pads: Query<&Gamepad>, mut q: Query<&mut Pos, With<Player>>) {
    let Some(pad) = pads.iter().next() else { return };
    let mut dir = pad.left_stick();
    if pad.pressed(GamepadButton::DPadLeft) { dir.x -= 1.0; }
    if pad.pressed(GamepadButton::DPadRight) { dir.x += 1.0; }
    if pad.pressed(GamepadButton::DPadUp) { dir.y += 1.0; }
    if pad.pressed(GamepadButton::DPadDown) { dir.y -= 1.0; }
    for mut p in &mut q {
        // screen y grows downward; Bevy's stick y grows upward
        p.0 += Vec2::new(dir.x, -dir.y).clamp_length_max(1.0) * 120.0 * time.delta_secs();
        p.0 = p.0.clamp(Vec2::ZERO, Vec2::new(W as f32 - 8.0, H as f32 - 8.0));
    }
}

/// Arrow keys steer too (the cart declares the keyboard).
fn keys(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut q: Query<&mut Pos, With<Player>>) {
    let mut d = Vec2::ZERO;
    if keys.pressed(KeyCode::ArrowLeft) { d.x -= 1.0; }
    if keys.pressed(KeyCode::ArrowRight) { d.x += 1.0; }
    if keys.pressed(KeyCode::ArrowUp) { d.y -= 1.0; }
    if keys.pressed(KeyCode::ArrowDown) { d.y += 1.0; }
    for mut p in &mut q {
        p.0 = (p.0 + d * 120.0 * time.delta_secs()).clamp(Vec2::ZERO, Vec2::new(W as f32 - 8.0, H as f32 - 8.0));
    }
}

/// A mouse click or a touch puts the player there (the cart declares the pointer).
fn click(
    buttons: Res<ButtonInput<MouseButton>>,
    window: Query<&Window>,
    touches: Res<Touches>,
    mut q: Query<&mut Pos, With<Player>>,
) {
    let mut at = None;
    if buttons.just_pressed(MouseButton::Left) {
        at = window.iter().next().and_then(|w| w.physical_cursor_position());
    }
    if let Some(t) = touches.iter_just_pressed().next() {
        at = Some(t.position());
    }
    if let Some(at) = at {
        for mut p in &mut q {
            p.0 = (at - Vec2::splat(4.0)).clamp(Vec2::ZERO, Vec2::new(W as f32 - 8.0, H as f32 - 8.0));
        }
    }
}

fn spawn_or_clear(
    mut commands: Commands,
    pads: Query<&Gamepad>,
    player: Query<&Pos, With<Player>>,
    balls: Query<Entity, With<Velocity>>,
    audio: Res<Audio>,
    sounds: Res<Sounds>,
    mut n: Local<u32>,
) {
    let Some(pad) = pads.iter().next() else { return };
    let Ok(p) = player.single() else { return };
    if pad.just_pressed(GamepadButton::South) {
        *n += 1;
        let a = *n as f32 * 2.399;
        let colors = [0xFF40_80FF, 0xFFFF_8040, 0xFF40_FF80, 0xFFFF_40C0];
        commands.spawn((
            Pos(p.0),
            Velocity(Vec2::new(a.cos(), a.sin()) * 90.0),
            Tint(colors[*n as usize % 4]),
        ));
        audio.play(&sounds.spawn);
    }
    if pad.just_pressed(GamepadButton::East) {
        for e in &balls {
            commands.entity(e).despawn();
        }
        audio.play(&sounds.clear);
    }
}

fn bounce(time: Res<Time>, mut q: Query<(&mut Pos, &mut Velocity)>) {
    for (mut p, mut v) in &mut q {
        p.0 += v.0 * time.delta_secs();
        if p.0.x < 0.0 || p.0.x > W as f32 - 8.0 { v.0.x = -v.0.x; }
        if p.0.y < 0.0 || p.0.y > H as f32 - 8.0 { v.0.y = -v.0.y; }
        p.0 = p.0.clamp(Vec2::ZERO, Vec2::new(W as f32 - 8.0, H as f32 - 8.0));
    }
}

fn publish(mut dbg: ResMut<CartDebug>, player: Query<&Pos, With<Player>>, balls: Query<(), With<Velocity>>) {
    if let Ok(p) = player.single() {
        dbg.set("player_x", p.0.x);
        dbg.set("player_y", p.0.y);
    }
    dbg.set("balls", balls.iter().count() as f32);
}

fn draw(q: Query<(&Pos, &Tint)>, info: Res<CartInfo>) {
    let fb = wasmcart_bevy::abi::framebuffer();
    // a slowly shifting backdrop so motion is visible even with no input
    let shade = ((info.time_ms / 40.0) as u32 % 64) as u32;
    fb.fill(0xFF00_0000 | (0x20 + shade) << 16 | 0x18 << 8 | 0x30);
    for (p, t) in &q {
        let (x0, y0) = (p.0.x as i32, p.0.y as i32);
        for y in y0..y0 + 8 {
            for x in x0..x0 + 8 {
                if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) {
                    fb[(y as u32 * W + x as u32) as usize] = t.0;
                }
            }
        }
    }
}
