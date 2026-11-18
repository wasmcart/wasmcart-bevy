//! Star Catcher: a small 2D game on wasmcart-bevy.
//!
//! Fly the ship with the left stick or the d-pad, catch the falling stars,
//! dodge the red rocks. A boosts. Three hits ends the run; Start plays again.
//!
//! What it exercises: sprites loaded from the cart's own assets through Bevy's
//! asset server, `Mesh2d` shapes, `Text2d` with Bevy's default font, a WAV
//! loaded as an asset plus synthesized sounds, pad input as Bevy `Gamepad`s.

use bevy::prelude::*;
use wasmcart_bevy::prelude::*;

const W: f32 = 640.0;
const H: f32 = 360.0;
const CART: CartConfig = CartConfig::new(W as u32, H as u32).webgpu().debug();

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(WasmcartPlugins)
        .insert_resource(ClearColor(Color::srgb(0.03, 0.03, 0.09)))
        .insert_resource(Rng(0x9E37_79B9))
        .init_resource::<Game>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (fly, spawn, fall, collide, restart, hud, twinkle, publish).chain(),
        );
    app
}

wasmcart_bevy::cart!(CART, app, debug);

#[derive(Resource)]
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32
    }
}

#[derive(Resource)]
struct Game {
    score: u32,
    lives: u32,
    over: bool,
    star_timer: f32,
    rock_timer: f32,
    hurt: f32,
}
impl Default for Game {
    fn default() -> Self {
        Self { score: 0, lives: 3, over: false, star_timer: 0.5, rock_timer: 1.5, hurt: 0.0 }
    }
}

#[derive(Resource)]
struct Art {
    star: Handle<Image>,
    rock_mesh: Handle<Mesh>,
    rock_mat: Handle<ColorMaterial>,
    pickup: Handle<AudioClip>,
    hit: AudioClip,
    over: AudioClip,
}

#[derive(Component)]
struct Ship;
#[derive(Component)]
struct Star {
    speed: f32,
    spin: f32,
}
#[derive(Component)]
struct Rock {
    speed: f32,
    radius: f32,
}
#[derive(Component)]
struct Hud;
#[derive(Component)]
struct Banner;
#[derive(Component)]
struct Twinkle(f32);

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut rng: ResMut<Rng>,
) {
    commands.spawn(Camera2d);
    // backdrop stars: plain coloured sprites, no texture
    for i in 0..90 {
        let x = (rng.next() - 0.5) * W;
        let y = (rng.next() - 0.5) * H;
        let s = 1.0 + rng.next() * 2.0;
        commands.spawn((
            Sprite::from_color(Color::srgb(0.6, 0.7, 1.0), Vec2::splat(s)),
            Transform::from_xyz(x, y, -10.0),
            Twinkle(i as f32 * 0.37),
        ));
    }
    commands.spawn((
        Ship,
        Sprite { custom_size: Some(Vec2::splat(48.0)), ..Sprite::from_image(assets.load("ship.png")) },
        Transform::from_xyz(0.0, -H / 2.0 + 50.0, 1.0),
    ));
    commands.spawn((
        Hud,
        Text2d::new("SCORE 0   LIVES 3"),
        TextFont { font_size: FontSize::Px(20.0), ..default() },
        TextColor(Color::srgb(0.9, 0.95, 1.0)),
        bevy::sprite::Anchor::TOP_LEFT,
        Transform::from_xyz(-W / 2.0 + 12.0, H / 2.0 - 10.0, 5.0),
    ));
    commands.spawn((
        Banner,
        Text2d::new(""),
        TextFont { font_size: FontSize::Px(32.0), ..default() },
        TextColor(Color::srgb(1.0, 0.85, 0.3)),
        Transform::from_xyz(0.0, 20.0, 5.0),
    ));
    commands.insert_resource(Art {
        star: assets.load("star.png"),
        rock_mesh: meshes.add(Circle::new(1.0)),
        rock_mat: materials.add(Color::srgb(0.85, 0.25, 0.2)),
        pickup: assets.load("pickup.wav"),
        hit: AudioClip::sweep(220.0, 55.0, 0.35, Wave::Noise),
        over: AudioClip::sweep(440.0, 110.0, 0.9, Wave::Triangle),
    });
}

fn fly(
    time: Res<Time>,
    game: Res<Game>,
    pads: Query<&Gamepad>,
    mut ship: Query<(&mut Transform, &mut Sprite), With<Ship>>,
) {
    let Ok((mut t, mut sprite)) = ship.single_mut() else { return };
    let mut dir = Vec2::ZERO;
    let mut boost = 1.0;
    if let Some(pad) = pads.iter().next() {
        dir = pad.left_stick();
        if pad.pressed(GamepadButton::DPadLeft) { dir.x -= 1.0; }
        if pad.pressed(GamepadButton::DPadRight) { dir.x += 1.0; }
        if pad.pressed(GamepadButton::DPadUp) { dir.y += 1.0; }
        if pad.pressed(GamepadButton::DPadDown) { dir.y -= 1.0; }
        if pad.pressed(GamepadButton::South) { boost = 1.8; }
    }
    if game.over {
        dir = Vec2::ZERO;
    }
    let dt = time.delta_secs();
    t.translation += (dir.clamp_length_max(1.0) * 260.0 * boost * dt).extend(0.0);
    t.translation.x = t.translation.x.clamp(-W / 2.0 + 24.0, W / 2.0 - 24.0);
    t.translation.y = t.translation.y.clamp(-H / 2.0 + 24.0, H / 2.0 - 60.0);
    // bank into the turn
    t.rotation = Quat::from_rotation_z(-dir.x * 0.25);
    // a hit tints the ship and fades back over half a second (no flashing)
    let k = (game.hurt / 0.5).clamp(0.0, 1.0);
    sprite.color = Color::srgb(1.0, 1.0 - 0.6 * k, 1.0 - 0.6 * k);
}

fn spawn(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    mut rng: ResMut<Rng>,
    art: Res<Art>,
) {
    if game.over {
        return;
    }
    let dt = time.delta_secs();
    game.star_timer -= dt;
    game.rock_timer -= dt;
    if game.star_timer <= 0.0 {
        game.star_timer = 0.45 + rng.next() * 0.4;
        let x = (rng.next() - 0.5) * (W - 60.0);
        commands.spawn((
            Star { speed: 90.0 + rng.next() * 80.0, spin: (rng.next() - 0.5) * 4.0 },
            Sprite { custom_size: Some(Vec2::splat(32.0)), ..Sprite::from_image(art.star.clone()) },
            Transform::from_xyz(x, H / 2.0 + 20.0, 0.5),
        ));
    }
    if game.rock_timer <= 0.0 {
        let level = 1.0 + game.score as f32 / 15.0;
        game.rock_timer = (1.6 + rng.next()) / level;
        let x = (rng.next() - 0.5) * (W - 60.0);
        let radius = 10.0 + rng.next() * 14.0;
        commands.spawn((
            Rock { speed: (110.0 + rng.next() * 90.0) * level.sqrt(), radius },
            Mesh2d(art.rock_mesh.clone()),
            MeshMaterial2d(art.rock_mat.clone()),
            Transform::from_xyz(x, H / 2.0 + 30.0, 0.6).with_scale(Vec3::splat(radius)),
        ));
    }
}

fn fall(
    mut commands: Commands,
    time: Res<Time>,
    mut stars: Query<(Entity, &Star, &mut Transform), Without<Rock>>,
    mut rocks: Query<(Entity, &Rock, &mut Transform), Without<Star>>,
) {
    let dt = time.delta_secs();
    for (e, s, mut t) in &mut stars {
        t.translation.y -= s.speed * dt;
        t.rotate_z(s.spin * dt);
        if t.translation.y < -H / 2.0 - 30.0 {
            commands.entity(e).despawn();
        }
    }
    for (e, r, mut t) in &mut rocks {
        t.translation.y -= r.speed * dt;
        if t.translation.y < -H / 2.0 - 40.0 {
            commands.entity(e).despawn();
        }
    }
}

fn collide(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    audio: Res<Audio>,
    art: Res<Art>,
    clips: Res<Assets<AudioClip>>,
    ship: Query<&Transform, With<Ship>>,
    stars: Query<(Entity, &Transform), With<Star>>,
    rocks: Query<(Entity, &Rock, &Transform)>,
) {
    game.hurt = (game.hurt - time.delta_secs()).max(0.0);
    let Ok(ship) = ship.single() else { return };
    let p = ship.translation.truncate();
    for (e, t) in &stars {
        if t.translation.truncate().distance(p) < 30.0 {
            commands.entity(e).despawn();
            if !game.over {
                game.score += 1;
                if let Some(clip) = clips.get(&art.pickup) {
                    audio.play(clip);
                }
            }
        }
    }
    if game.over {
        return;
    }
    for (e, r, t) in &rocks {
        if t.translation.truncate().distance(p) < r.radius + 16.0 {
            commands.entity(e).despawn();
            game.lives = game.lives.saturating_sub(1);
            game.hurt = 0.5;
            audio.play(&art.hit);
            if game.lives == 0 {
                game.over = true;
                audio.play(&art.over);
            }
        }
    }
}

fn restart(
    mut commands: Commands,
    mut game: ResMut<Game>,
    pads: Query<&Gamepad>,
    things: Query<Entity, Or<(With<Star>, With<Rock>)>>,
) {
    if !game.over {
        return;
    }
    if pads.iter().any(|p| p.just_pressed(GamepadButton::Start)) {
        for e in &things {
            commands.entity(e).despawn();
        }
        *game = Game::default();
    }
}

fn hud(
    game: Res<Game>,
    mut hud: Query<&mut Text2d, (With<Hud>, Without<Banner>)>,
    mut banner: Query<&mut Text2d, (With<Banner>, Without<Hud>)>,
) {
    if !game.is_changed() {
        return;
    }
    if let Ok(mut t) = hud.single_mut() {
        t.0 = format!("SCORE {}   LIVES {}", game.score, game.lives);
    }
    if let Ok(mut t) = banner.single_mut() {
        t.0 = if game.over { format!("GAME OVER  {}\npress START", game.score) } else { String::new() };
    }
}

fn twinkle(time: Res<Time>, mut q: Query<(&Twinkle, &mut Sprite)>) {
    let t = time.elapsed_secs();
    for (tw, mut s) in &mut q {
        let a = 0.45 + 0.35 * (t * 1.3 + tw.0).sin();
        s.color = Color::srgba(0.6, 0.7, 1.0, a);
    }
}

fn publish(
    mut dbg: ResMut<CartDebug>,
    game: Res<Game>,
    ship: Query<&Transform, With<Ship>>,
    stars: Query<(), With<Star>>,
    rocks: Query<(), With<Rock>>,
) {
    if let Ok(t) = ship.single() {
        dbg.set("ship_x", t.translation.x);
        dbg.set("ship_y", t.translation.y);
    }
    dbg.set("score", game.score as f32);
    dbg.set("lives", game.lives as f32);
    dbg.set("over", game.over as u32 as f32);
    dbg.set("stars", stars.iter().count() as f32);
    dbg.set("rocks", rocks.iter().count() as f32);
}
