//! Named debug fields for a debugging host (wasmcart "Debug state").
//!
//! Opt in with [`CartConfig::debug`](crate::CartConfig::debug) and the
//! `debug` form of [`cart!`](crate::cart). Without it the cart exports no
//! `wc_debug_state` and sets no `WC_FLAG_DEBUG`, as the spec requires.
//!
//! Systems publish values through the [`CartDebug`] resource:
//!
//! ```ignore
//! fn publish(mut dbg: ResMut<CartDebug>, q: Query<&Transform, With<Player>>) {
//!     if let Ok(t) = q.single() { dbg.set("player_x", t.translation.x); }
//! }
//! ```
//!
//! Four fields are always present: `bevy.frame` (frames updated),
//! `bevy.entities` (live entity count), `host.delta_ms` (the host frame time)
//! and `bevy.ready` (1 once every plugin
//! finished, i.e. the renderer is up).

use core::ptr::{addr_of, addr_of_mut};

use bevy::prelude::*;

const MAX_FIELDS: usize = 64;
const BUILTINS: usize = 4;

/// User debug values, copied into the host-visible table after each frame.
#[derive(Resource, Default)]
pub struct CartDebug {
    values: Vec<(&'static str, f32)>,
}

impl CartDebug {
    /// Set (or create) the field `name`. Names are fixed for the life of the
    /// cart; at most 60 user fields.
    pub fn set(&mut self, name: &'static str, value: f32) {
        if let Some(v) = self.values.iter_mut().find(|(n, _)| *n == name) {
            v.1 = value;
        } else if self.values.len() < MAX_FIELDS - BUILTINS {
            self.values.push((name, value));
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Field {
    name_ptr: u32,
    value_ptr: u32,
    ty: u8,
    _pad: [u8; 3],
    len: u32,
}

const WC_DBG_U32: u8 = 4;
const WC_DBG_F32: u8 = 6;

static mut TABLE: Vec<Field> = Vec::new();
static mut NAMES: Vec<Vec<u8>> = Vec::new();
static mut BUILTIN: [u32; 3] = [0; 3];
static mut DELTA_MS: f32 = 0.0;
static mut VALUES: [f32; MAX_FIELDS] = [0.0; MAX_FIELDS];

fn push(name: &str, value_ptr: u32, ty: u8) {
    unsafe {
        let names = &mut *addr_of_mut!(NAMES);
        let mut n = name.as_bytes().to_vec();
        n.push(0);
        names.push(n);
        let table = &mut *addr_of_mut!(TABLE);
        // keep the terminator last
        table.pop();
        table.push(Field {
            name_ptr: names.last().unwrap().as_ptr() as u32,
            value_ptr,
            ty,
            _pad: [0; 3],
            len: 1,
        });
        table.push(Field { name_ptr: 0, value_ptr: 0, ty: 0, _pad: [0; 3], len: 0 });
    }
}

fn ensure_builtins() {
    unsafe {
        if !(*addr_of!(TABLE)).is_empty() {
            return;
        }
        // Fixed capacity: the host may hold the table pointer, so it must never move.
        (*addr_of_mut!(TABLE)).reserve_exact(MAX_FIELDS + 1);
        (*addr_of_mut!(TABLE)).push(Field { name_ptr: 0, value_ptr: 0, ty: 0, _pad: [0; 3], len: 0 });
        let b = addr_of!(BUILTIN) as *const u32;
        push("bevy.frame", b as u32, WC_DBG_U32);
        push("bevy.entities", b.add(1) as u32, WC_DBG_U32);
        push("bevy.ready", b.add(2) as u32, WC_DBG_U32);
        push("host.delta_ms", addr_of!(DELTA_MS) as u32, WC_DBG_F32);
    }
}

pub(crate) fn publish(world: &mut World) {
    if !crate::abi::config().debug {
        return;
    }
    ensure_builtins();
    unsafe {
        let b = &mut *addr_of_mut!(BUILTIN);
        b[0] = b[0].wrapping_add(1);
        b[1] = world.entities().count_spawned() as u32;
        b[2] = 1;
        *addr_of_mut!(DELTA_MS) = crate::abi::time().delta_ms as f32;
        if let Some(dbg) = world.get_resource::<CartDebug>() {
            let user_count = (*addr_of!(TABLE)).len() - 1 - BUILTINS;
            for (i, (name, v)) in dbg.values.iter().enumerate() {
                if i >= user_count {
                    let slot = (addr_of_mut!(VALUES) as *mut f32).add(i);
                    push(name, slot as u32, WC_DBG_F32);
                }
                (*addr_of_mut!(VALUES))[i] = *v;
            }
        }
    }
}

#[doc(hidden)]
pub fn debug_state() -> *const u8 {
    ensure_builtins();
    unsafe { (*addr_of!(TABLE)).as_ptr() as *const u8 }
}
