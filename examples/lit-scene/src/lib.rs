//! Lit Scene: Bevy's PBR renderer on the host's WebGPU device.
//!
//! A shadow-casting sun, three coloured point lights circling the scene, metal
//! and dielectric materials across a roughness ramp, a turntable of shapes.
//! Left stick (or d-pad) orbits the camera, the triggers (or L/R) zoom, A drops
//! a ball that bounces on the floor, B pauses the lights.

use bevy::prelude::*;
use wasmcart_bevy::prelude::*;

const CART: CartConfig = CartConfig::new(960, 540).webgpu().debug();

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(WasmcartPlugins)
        .insert_resource(ClearColor(Color::srgb(0.05, 0.06, 0.09)))
        .insert_resource(GlobalAmbientLight { brightness: 120.0, ..default() })
        .init_resource::<Orbit>()
        .add_systems(Startup, setup)
        .add_systems(Update, (orbit, lights, spin, drop_ball, bounce, publish).chain());
    app
}

wasmcart_bevy::cart!(CART, app, debug);

#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    dist: f32,
    lights_paused: bool,
    light_t: f32,
}
impl Default for Orbit {
    fn default() -> Self {
        Self { yaw: 0.9, pitch: 0.5, dist: 13.5, lights_paused: false, light_t: 0.0 }
    }
}

#[derive(Component)]
struct OrbitLight(f32);
#[derive(Component)]
struct Spin(f32);
#[derive(Component)]
struct Ball {
    vel: f32,
}
#[derive(Resource)]
struct Kit {
    ball_mesh: Handle<Mesh>,
    ball_mats: Vec<Handle<StandardMaterial>>,
    plink: AudioClip,
    drop: AudioClip,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((Camera3d::default(), Transform::from_xyz(8.0, 5.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y)));

    // floor
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(80.0, 80.0))),
        MeshMaterial3d(mats.add(StandardMaterial {
            base_color: Color::srgb(0.32, 0.34, 0.38),
            perceptual_roughness: 0.85,
            ..default()
        })),
    ));

    // sun with shadows
    commands.spawn((
        DirectionalLight { illuminance: 6000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(4.0, 10.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // three coloured point lights on a circle
    for (i, c) in [Color::srgb(1.0, 0.35, 0.2), Color::srgb(0.2, 0.6, 1.0), Color::srgb(0.4, 1.0, 0.4)]
        .into_iter()
        .enumerate()
    {
        commands
            .spawn((
                OrbitLight(i as f32 * core::f32::consts::TAU / 3.0),
                PointLight { color: c, intensity: 250_000.0, range: 20.0, shadow_maps_enabled: false, ..default() },
                Transform::from_xyz(0.0, 2.0, 0.0),
            ))
            .with_child((
                Mesh3d(meshes.add(Sphere::new(0.12).mesh().uv(16, 8))),
                MeshMaterial3d(mats.add(StandardMaterial {
                    base_color: c,
                    emissive: LinearRgba::from(c) * 8.0,
                    ..default()
                })),
            ));
    }

    // material ramp: metal row in front, plastic row behind, rough to smooth
    let sphere = meshes.add(Sphere::new(0.5).mesh().uv(48, 24));
    for i in 0..5 {
        let r = 0.1 + i as f32 * 0.2;
        let x = -4.0 + i as f32 * 2.0;
        commands.spawn((
            Mesh3d(sphere.clone()),
            MeshMaterial3d(mats.add(StandardMaterial {
                base_color: Color::srgb(0.95, 0.75, 0.35),
                metallic: 1.0,
                perceptual_roughness: r,
                ..default()
            })),
            Transform::from_xyz(x, 0.5, 3.0),
        ));
        commands.spawn((
            Mesh3d(sphere.clone()),
            MeshMaterial3d(mats.add(StandardMaterial {
                base_color: Color::srgb(0.75, 0.1, 0.12),
                metallic: 0.0,
                perceptual_roughness: r,
                ..default()
            })),
            Transform::from_xyz(x, 0.5, 4.6),
        ));
    }

    // turntable of shapes
    let shapes: [(Handle<Mesh>, Color); 4] = [
        (meshes.add(Cuboid::new(1.2, 1.2, 1.2)), Color::srgb(0.2, 0.55, 0.9)),
        (meshes.add(Torus::new(0.35, 0.75)), Color::srgb(0.9, 0.9, 0.92)),
        (meshes.add(Cylinder::new(0.55, 1.4)), Color::srgb(0.35, 0.8, 0.45)),
        (meshes.add(Cone { radius: 0.7, height: 1.5 }), Color::srgb(0.95, 0.6, 0.15)),
    ];
    for (i, (mesh, color)) in shapes.into_iter().enumerate() {
        let a = i as f32 * core::f32::consts::FRAC_PI_2;
        commands.spawn((
            Spin(0.6 + i as f32 * 0.25),
            Mesh3d(mesh),
            MeshMaterial3d(mats.add(StandardMaterial {
                base_color: color,
                metallic: if i == 1 { 1.0 } else { 0.0 },
                perceptual_roughness: if i == 1 { 0.25 } else { 0.5 },
                ..default()
            })),
            Transform::from_xyz(a.cos() * 2.4, 0.9, a.sin() * 2.4 - 1.0),
        ));
    }

    commands.insert_resource(Kit {
        ball_mesh: meshes.add(Sphere::new(0.3).mesh().uv(24, 12)),
        ball_mats: [Color::srgb(1.0, 0.2, 0.6), Color::srgb(0.2, 0.9, 1.0), Color::srgb(1.0, 0.95, 0.2)]
            .into_iter()
            .map(|c| mats.add(StandardMaterial { base_color: c, perceptual_roughness: 0.3, ..default() }))
            .collect(),
        plink: AudioClip::tone(880.0, 0.08, Wave::Sine),
        drop: AudioClip::sweep(300.0, 600.0, 0.15, Wave::Triangle),
    });
}

fn orbit(
    time: Res<Time>,
    pads: Query<&Gamepad>,
    mut o: ResMut<Orbit>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
) {
    let dt = time.delta_secs();
    if let Some(pad) = pads.iter().next() {
        let mut s = pad.left_stick() + pad.right_stick();
        if pad.pressed(GamepadButton::DPadLeft) { s.x -= 1.0; }
        if pad.pressed(GamepadButton::DPadRight) { s.x += 1.0; }
        if pad.pressed(GamepadButton::DPadUp) { s.y += 1.0; }
        if pad.pressed(GamepadButton::DPadDown) { s.y -= 1.0; }
        o.yaw += s.x.clamp(-1.0, 1.0) * 1.6 * dt;
        o.pitch = (o.pitch + s.y.clamp(-1.0, 1.0) * 1.0 * dt).clamp(0.08, 1.35);
        let zoom_in = pad.get(GamepadButton::RightTrigger2).unwrap_or(0.0)
            + if pad.pressed(GamepadButton::RightTrigger) { 1.0 } else { 0.0 };
        let zoom_out = pad.get(GamepadButton::LeftTrigger2).unwrap_or(0.0)
            + if pad.pressed(GamepadButton::LeftTrigger) { 1.0 } else { 0.0 };
        o.dist = (o.dist + (zoom_out - zoom_in) * 6.0 * dt).clamp(4.0, 26.0);
        if pad.just_pressed(GamepadButton::East) {
            o.lights_paused = !o.lights_paused;
        }
    }
    if let Ok(mut t) = cam.single_mut() {
        let p = Vec3::new(o.yaw.cos() * o.pitch.cos(), o.pitch.sin(), o.yaw.sin() * o.pitch.cos()) * o.dist;
        *t = Transform::from_translation(p + Vec3::Y * 0.5).looking_at(Vec3::new(0.0, 0.6, 0.5), Vec3::Y);
    }
}

fn lights(time: Res<Time>, mut o: ResMut<Orbit>, mut q: Query<(&OrbitLight, &mut Transform)>) {
    if !o.lights_paused {
        o.light_t += time.delta_secs();
    }
    let t = o.light_t;
    for (l, mut tr) in &mut q {
        let a = l.0 + t * 0.7;
        tr.translation = Vec3::new(a.cos() * 4.5, 1.6 + 0.6 * (t * 1.3 + l.0).sin(), a.sin() * 4.5);
    }
}

fn spin(time: Res<Time>, mut q: Query<(&Spin, &mut Transform)>) {
    for (s, mut t) in &mut q {
        t.rotate_y(s.0 * time.delta_secs());
        t.rotate_x(s.0 * 0.4 * time.delta_secs());
    }
}

fn drop_ball(
    mut commands: Commands,
    pads: Query<&Gamepad>,
    kit: Res<Kit>,
    audio: Res<Audio>,
    balls: Query<Entity, With<Ball>>,
    mut n: Local<u32>,
) {
    if !pads.iter().any(|p| p.just_pressed(GamepadButton::South)) {
        return;
    }
    if balls.iter().count() >= 24 {
        if let Some(e) = balls.iter().next() {
            commands.entity(e).despawn();
        }
    }
    *n += 1;
    let a = *n as f32 * 2.399;
    commands.spawn((
        Ball { vel: 0.0 },
        Mesh3d(kit.ball_mesh.clone()),
        MeshMaterial3d(kit.ball_mats[*n as usize % kit.ball_mats.len()].clone()),
        Transform::from_xyz(a.cos() * 1.5, 6.0, a.sin() * 1.5 + 1.0),
    ));
    audio.play(&kit.drop);
}

fn bounce(time: Res<Time>, kit: Res<Kit>, audio: Res<Audio>, mut q: Query<(&mut Ball, &mut Transform)>) {
    let dt = time.delta_secs();
    for (mut b, mut t) in &mut q {
        b.vel -= 9.8 * dt;
        t.translation.y += b.vel * dt;
        if t.translation.y < 0.3 {
            t.translation.y = 0.3;
            if b.vel < -1.0 {
                audio.play_with(&kit.plink, PlaySettings { volume: (-b.vel / 10.0).min(1.0), ..default() });
            }
            b.vel = -b.vel * 0.7;
            if b.vel.abs() < 0.4 {
                b.vel = 0.0;
            }
        }
    }
}

fn publish(mut dbg: ResMut<CartDebug>, o: Res<Orbit>, balls: Query<(), With<Ball>>) {
    dbg.set("yaw", o.yaw);
    dbg.set("pitch", o.pitch);
    dbg.set("dist", o.dist);
    dbg.set("balls", balls.iter().count() as f32);
    dbg.set("light_t", o.light_t);
}
