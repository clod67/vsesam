//! Built-in AI behaviours for VSESAM agents.
//!
//! Each behaviour is a pure decision: given a read-only view of the world and
//! a slice of randomness, return this tick's intended move `(dx, dy)`. The
//! interpreter applies the move; combat is resolved separately. This keeps the
//! borrow checker happy (no `&mut World` here) and makes behaviours testable.

use crate::engine::world::{EntityId, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behavior {
    Idle,
    Wander,
    Seek,
    Flee,
    Patrol,
    Hunt,
    /// Utility-driven "smart agent": flees when hurt, holds to fight when an
    /// enemy is in reach, chases when one is visible, wanders otherwise.
    Think,
}

impl Behavior {
    pub fn from_str(s: &str) -> Option<Behavior> {
        Some(match s {
            "idle" | "闲" => Behavior::Idle,
            "wander" | "游" => Behavior::Wander,
            "seek" | "寻" => Behavior::Seek,
            "flee" | "逃" => Behavior::Flee,
            "patrol" | "巡" => Behavior::Patrol,
            "hunt" | "猎" => Behavior::Hunt,
            "think" | "思" => Behavior::Think,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Behavior::Idle => "idle",
            Behavior::Wander => "wander",
            Behavior::Seek => "seek",
            Behavior::Flee => "flee",
            Behavior::Patrol => "patrol",
            Behavior::Hunt => "hunt",
            Behavior::Think => "think",
        }
    }
}

/// Decide a move for entity `id`. `noise` is a fresh random u64 for this call;
/// `tick` is the current frame number.
pub fn decide(world: &World, id: EntityId, beh: Behavior, noise: u64, tick: u64) -> (f64, f64) {
    let me = match world.get(id) {
        Some(e) => e,
        None => return (0.0, 0.0),
    };

    match beh {
        Behavior::Idle => (0.0, 0.0),

        Behavior::Wander => wander(noise),

        Behavior::Seek => match world.nearest_enemy(id) {
            Some(t) => step_toward(world, id, t, 1.0),
            None => wander(noise),
        },

        Behavior::Flee => match world.nearest_enemy(id) {
            Some(t) => step_toward(world, id, t, -1.0),
            None => (0.0, 0.0),
        },

        Behavior::Patrol => {
            // oscillate horizontally; phase offset by id so a line doesn't march in lockstep
            let dir = if ((tick + id as u64) / 4) % 2 == 0 { 1.0 } else { -1.0 };
            (dir, 0.0)
        }

        Behavior::Hunt => match world.nearest_enemy(id) {
            Some(t) => {
                let d = world.distance(id, t).unwrap_or(f64::INFINITY);
                if d <= me.range {
                    (0.0, 0.0) // in range: hold position and let combat resolve
                } else {
                    step_toward(world, id, t, 1.0)
                }
            }
            None => wander(noise),
        },

        Behavior::Think => {
            let hurt = me.max_hp > 0.0 && me.hp / me.max_hp < 0.34;
            match world.nearest_enemy(id) {
                Some(t) => {
                    let d = world.distance(id, t).unwrap_or(f64::INFINITY);
                    if hurt {
                        step_toward(world, id, t, -1.0) // wounded: retreat
                    } else if d <= me.range {
                        (0.0, 0.0) // engaged: stand and fight
                    } else {
                        step_toward(world, id, t, 1.0) // healthy: close in
                    }
                }
                None => wander(noise),
            }
        }
    }
}

/// Unit step toward (sign > 0) or away from (sign < 0) a target, one tile.
fn step_toward(world: &World, id: EntityId, target: EntityId, sign: f64) -> (f64, f64) {
    let (a, b) = match (world.get(id), world.get(target)) {
        (Some(a), Some(b)) => (a, b),
        _ => return (0.0, 0.0),
    };
    let dx = (b.x - a.x).signum() * sign;
    let dy = (b.y - a.y).signum() * sign;
    // avoid freezing when perfectly aligned on one axis
    (clamp_unit(dx), clamp_unit(dy))
}

fn clamp_unit(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// Map a random u64 to one of 9 moves (8 directions + stay).
fn wander(noise: u64) -> (f64, f64) {
    const DIRS: [(f64, f64); 9] = [
        (0.0, 0.0),
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (1.0, 1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
        (-1.0, -1.0),
    ];
    DIRS[(noise % 9) as usize]
}
