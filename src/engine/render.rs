//! ASCII renderer. Rasterises the world onto a character grid with a box
//! border. Self-contained so VSESAM runs in any terminal (or headless, frame
//! by frame) with no graphics stack.

use super::world::World;

/// Render the world to a bordered ASCII frame (no trailing newline).
pub fn frame(world: &World) -> String {
    let w = world.width;
    let h = world.height;
    let mut grid = vec![vec![' '; w]; h];

    // Draw neutrals first, then monsters, then allies/player on top, so the
    // interesting stuff wins a shared cell.
    let mut order: Vec<&super::world::Entity> = world.entities.iter().filter(|e| e.alive).collect();
    order.sort_by_key(|e| priority(e.faction));
    for e in order {
        let cx = e.x.round() as isize;
        let cy = e.y.round() as isize;
        if cx >= 0 && cy >= 0 && (cx as usize) < w && (cy as usize) < h {
            grid[cy as usize][cx as usize] = e.glyph;
        }
    }

    let mut out = String::new();
    out.push('╔');
    for _ in 0..w {
        out.push('═');
    }
    out.push_str("╗\n");
    for row in grid {
        out.push('║');
        for ch in row {
            out.push(ch);
        }
        out.push_str("║\n");
    }
    out.push('╚');
    for _ in 0..w {
        out.push('═');
    }
    out.push('╝');
    out
}

fn priority(faction: i32) -> i32 {
    match faction {
        -1 => 0, // neutral drawn first (lowest)
        1 => 1,  // monsters
        0 => 2,  // allies / player on top
        _ => 1,
    }
}

/// ANSI clear-screen + home, for animated playback.
pub fn clear() -> &'static str {
    "\x1b[2J\x1b[H"
}
