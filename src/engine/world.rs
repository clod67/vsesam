//! A tiny archetype-free ECS: every entity is a flat record of optional-ish
//! components. Small, cache-friendly enough for a toy, and trivial to drive
//! from nanch native calls.

use std::rc::Rc;

use crate::ai::Behavior;
use crate::nanch::value::Func;

pub type EntityId = usize;

/// How an entity decides what to do each tick.
#[derive(Clone)]
pub enum Brain {
    /// No autonomy — moved only by explicit `动` calls.
    None,
    /// One of the engine's hand-written behaviours.
    Builtin(Behavior),
    /// A nanch function `f(id)` invoked every tick. This is how an "AI agent"
    /// written in nanch is embedded straight into the simulation.
    Script(Rc<Func>),
}

pub struct Entity {
    pub id: EntityId,
    pub alive: bool,
    pub kind: String,
    pub glyph: char,
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub hp: f64,
    pub max_hp: f64,
    pub faction: i32, // 0 = player/allies, 1 = monsters, -1 = neutral (no combat)
    pub damage: f64,
    pub range: f64,
    pub brain: Brain,
    /// Free-form label an agent can stamp on itself from script.
    pub label: String,
}

struct Proto {
    glyph: char,
    hp: f64,
    faction: i32,
    damage: f64,
    range: f64,
}

fn proto(kind: &str) -> Proto {
    match kind {
        "player" | "玩家" => Proto { glyph: '@', hp: 30.0, faction: 0, damage: 3.0, range: 1.5 },
        "agent" | "代理" => Proto { glyph: 'A', hp: 18.0, faction: 0, damage: 2.0, range: 1.5 },
        "goblin" | "哥布林" => Proto { glyph: 'g', hp: 8.0, faction: 1, damage: 2.0, range: 1.3 },
        "orc" | "兽人" => Proto { glyph: 'O', hp: 16.0, faction: 1, damage: 4.0, range: 1.4 },
        "slime" | "史莱姆" => Proto { glyph: 's', hp: 5.0, faction: 1, damage: 1.0, range: 1.2 },
        "wall" | "墙" => Proto { glyph: '#', hp: 1e12, faction: -1, damage: 0.0, range: 0.0 },
        "gold" | "金" => Proto { glyph: '$', hp: 1.0, faction: -1, damage: 0.0, range: 0.0 },
        _ => Proto { glyph: '?', hp: 10.0, faction: 1, damage: 1.0, range: 1.2 },
    }
}

pub struct World {
    pub entities: Vec<Entity>,
    free: Vec<EntityId>,
    pub width: usize,
    pub height: usize,
}

impl World {
    pub fn new(width: usize, height: usize) -> Self {
        World {
            entities: Vec::new(),
            free: Vec::new(),
            width,
            height,
        }
    }

    pub fn spawn(&mut self, kind: &str, x: f64, y: f64) -> EntityId {
        let p = proto(kind);
        let x = x.clamp(0.0, (self.width - 1) as f64);
        let y = y.clamp(0.0, (self.height - 1) as f64);
        let ent = |id| Entity {
            id,
            alive: true,
            kind: kind.to_string(),
            glyph: p.glyph,
            x,
            y,
            vx: 0.0,
            vy: 0.0,
            hp: p.hp,
            max_hp: p.hp,
            faction: p.faction,
            damage: p.damage,
            range: p.range,
            brain: Brain::None,
            label: String::new(),
        };
        if let Some(id) = self.free.pop() {
            self.entities[id] = ent(id);
            id
        } else {
            let id = self.entities.len();
            self.entities.push(ent(id));
            id
        }
    }

    pub fn get(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(id).filter(|e| e.alive)
    }

    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut Entity> {
        self.entities.get_mut(id).filter(|e| e.alive)
    }

    pub fn alive(&self, id: EntityId) -> bool {
        self.entities.get(id).map(|e| e.alive).unwrap_or(false)
    }

    pub fn kill(&mut self, id: EntityId) {
        if let Some(e) = self.entities.get_mut(id) {
            if e.alive {
                e.alive = false;
                self.free.push(id);
            }
        }
    }

    pub fn alive_count(&self) -> usize {
        self.entities.iter().filter(|e| e.alive).count()
    }

    pub fn count_faction(&self, faction: i32) -> usize {
        self.entities
            .iter()
            .filter(|e| e.alive && e.faction == faction)
            .count()
    }

    pub fn distance(&self, a: EntityId, b: EntityId) -> Option<f64> {
        let a = self.get(a)?;
        let b = self.get(b)?;
        Some(((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt())
    }

    /// Nearest living entity on a different, non-neutral faction.
    pub fn nearest_enemy(&self, id: EntityId) -> Option<EntityId> {
        let me = self.get(id)?;
        let mut best: Option<(f64, EntityId)> = None;
        for e in self.entities.iter() {
            if !e.alive || e.id == id || e.faction < 0 || e.faction == me.faction {
                continue;
            }
            let d = (e.x - me.x).powi(2) + (e.y - me.y).powi(2);
            if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, e.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// Nearest living entity of any kind (excluding self).
    pub fn nearest_any(&self, id: EntityId) -> Option<EntityId> {
        let me = self.get(id)?;
        let mut best: Option<(f64, EntityId)> = None;
        for e in self.entities.iter() {
            if !e.alive || e.id == id {
                continue;
            }
            let d = (e.x - me.x).powi(2) + (e.y - me.y).powi(2);
            if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, e.id));
            }
        }
        best.map(|(_, id)| id)
    }

    pub fn script_brains(&self) -> Vec<(EntityId, Rc<Func>)> {
        self.entities
            .iter()
            .filter_map(|e| match (e.alive, &e.brain) {
                (true, Brain::Script(f)) => Some((e.id, f.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn builtin_brains(&self) -> Vec<(EntityId, Behavior)> {
        self.entities
            .iter()
            .filter_map(|e| match (e.alive, &e.brain) {
                (true, Brain::Builtin(b)) => Some((e.id, *b)),
                _ => None,
            })
            .collect()
    }

    /// Apply this tick's velocity, clamp to the arena, then consume it so each
    /// brain must restate its intent next tick.
    pub fn integrate(&mut self) {
        let (w, h) = (self.width, self.height);
        for e in self.entities.iter_mut() {
            if !e.alive {
                continue;
            }
            e.x = (e.x + e.vx).clamp(0.0, (w - 1) as f64);
            e.y = (e.y + e.vy).clamp(0.0, (h - 1) as f64);
            e.vx = 0.0;
            e.vy = 0.0;
        }
    }

    /// Each armed combatant strikes its nearest in-range enemy once.
    /// Returns a log of `(attacker_id, victim_id, damage, killed)` events.
    pub fn resolve_combat(&mut self) -> Vec<CombatEvent> {
        let mut hits: Vec<(EntityId, EntityId, f64)> = Vec::new();
        for e in self.entities.iter() {
            if !e.alive || e.damage <= 0.0 || e.faction < 0 {
                continue;
            }
            if let Some(target) = self.nearest_enemy(e.id) {
                if let Some(d) = self.distance(e.id, target) {
                    if d <= e.range {
                        hits.push((e.id, target, e.damage));
                    }
                }
            }
        }
        let mut events = Vec::new();
        for (attacker, victim, dmg) in hits {
            let killed = if let Some(v) = self.entities.get_mut(victim) {
                if !v.alive {
                    continue;
                }
                v.hp -= dmg;
                v.hp <= 0.0
            } else {
                false
            };
            events.push(CombatEvent { attacker, victim, dmg, killed });
            if killed {
                self.kill(victim);
            }
        }
        events
    }
}

pub struct CombatEvent {
    pub attacker: EntityId,
    pub victim: EntityId,
    pub dmg: f64,
    pub killed: bool,
}
