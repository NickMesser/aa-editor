//! Keeps the server's navmesh heights in step with terrain edits.
//!
//! The AAEmu server reads `game/worlds/<world>/paths/XXX_YYY/{netmission,vertsmission}N.bai` (256 m path blocks,
//! `aaemu-game/src/systems/ai/geodata/bai.rs`). With `GeoDataMode` on, `AiGeoData::get_height` answers from them:
//! the plane of the enclosing navmesh triangle (corners = `vertsmission` records), else the nearest node
//! (`netmission` node = triangle centroid) or corner. A real navmesh bake (CryEngine's AI tools) is out of reach, so
//! this module moves what is already there: every node, corner and link edge centre that sat on the **original**
//! ground (within [`TOLERANCE`]) of an edited cell gets `Z += new - old` ground height at its XY. Points on bridges,
//! roofs and other objects (further than the tolerance from the ground) stay. Links touching a node that is now over
//! a hole are blocked (`max_pass_radius = -1`, the format's "boundary edge"), so A* no longer walks across the hole.
//!
//! Not updated: the triangulation itself. New ground (a raised plateau that used to be a cliff, a valley floor cut
//! below an object) gets no new triangles or links, so it stays as walkable or unwalkable as before; slopes made too
//! steep still have their old links. Only the heights of the existing mesh follow the terrain.
//!
//! Layout (little endian; `bai.rs`): netmission = `i32 version (55 / 54)`, bbox min / max (6 f32), `i32 nodes`,
//! nodes of 60 bytes (`id`, `dir`, `up`, `pos` at +28, ...), `i32 links`, links of 44 bytes (`src`, `dst`,
//! `edge_center` at +8, `max_pass_radius` at +20, ...). vertsmission = `i32 version (2 / 1)`, `i32 count`, records of
//! 32 bytes (`pos` at +0). Positions are block local: world = local + (bx * 256, by * 256, 0). The files are patched
//! in place, so untouched bytes stay identical.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use aa_world::heightmap::CellHeights;
use anyhow::{Context, Result, bail};

/// Metres a navmesh point may be above / below the original ground and still count as "on the ground".
pub const TOLERANCE: f32 = 1.5;
/// Ground changes below this are re-quantisation noise from the heightmap writer (about 1 cm), not edits.
pub const MIN_SHIFT: f32 = 0.05;
/// Size of a path block.
pub const BLOCK: f32 = 256.0;
const CELL: f32 = 1024.0;

/// Ground before and after the edit at a world XY: `(old, new, new_is_hole)`, `None` outside edited cells.
pub type Ground<'a> = dyn Fn(f32, f32) -> Option<(f32, f32, bool)> + 'a;

/// What one file's adjustment changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Changed {
    pub moved: usize,
    pub kept: usize,
    pub blocked_links: usize,
}

fn f32_at(b: &[u8], o: usize) -> Result<f32> {
    Ok(f32::from_le_bytes(b.get(o..o + 4).context("truncated bai")?.try_into()?))
}
fn i32_at(b: &[u8], o: usize) -> Result<i32> {
    Ok(i32::from_le_bytes(b.get(o..o + 4).context("truncated bai")?.try_into()?))
}
fn put_f32(b: &mut [u8], o: usize, v: f32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// Moves one point (local xyz at byte `o`) with the ground. Returns (moved, over a new hole).
fn adjust_point(b: &mut [u8], o: usize, origin: (f32, f32), ground: &Ground, ch: &mut Changed) -> Result<(bool, bool)> {
    let (x, y, z) = (f32_at(b, o)? + origin.0, f32_at(b, o + 4)? + origin.1, f32_at(b, o + 8)?);
    let Some((old, new, hole)) = ground(x, y) else { return Ok((false, false)) };
    if (z - old).abs() > TOLERANCE {
        ch.kept += 1;
        return Ok((false, hole));
    }
    let d = new - old;
    if d.abs() < MIN_SHIFT {
        return Ok((false, hole));
    }
    put_f32(b, o + 8, z + d);
    ch.moved += 1;
    Ok((true, hole))
}

/// Netmission of block `(bx, by)`: nodes, link edge centres and the bbox Z follow the ground. Patches `b` in place.
pub fn adjust_net(b: &mut [u8], block: (u32, u32), ground: &Ground) -> Result<Changed> {
    let version = i32_at(b, 0)?;
    if version != 55 && version != 54 {
        bail!("netmission version {version}");
    }
    let origin = (block.0 as f32 * BLOCK, block.1 as f32 * BLOCK);
    let mut ch = Changed::default();
    let nodes = i32_at(b, 28)?.max(0) as usize;
    let mut zs: Vec<f32> = Vec::new();
    let mut hole_ids = std::collections::HashSet::new();
    for i in 0..nodes {
        let o = 32 + i * 60;
        let (_, hole) = adjust_point(b, o + 28, origin, ground, &mut ch)?;
        if hole {
            hole_ids.insert(i32_at(b, o)? as u32);
        }
        zs.push(f32_at(b, o + 36)?);
    }
    let lo = 32 + nodes * 60;
    let links = i32_at(b, lo)?.max(0) as usize;
    for i in 0..links {
        let o = lo + 4 + i * 44;
        // `edge_center` is read with the offset added (z offset is 0, so the same local -> world rule holds)
        adjust_point(b, o + 8, origin, ground, &mut ch)?;
        let (src, dst) = (i32_at(b, o)? as u32, i32_at(b, o + 4)? as u32);
        if (hole_ids.contains(&src) || hole_ids.contains(&dst)) && f32_at(b, o + 20)? != -1.0 {
            put_f32(b, o + 20, -1.0);
            ch.blocked_links += 1;
        }
    }
    if lo + 4 + links * 44 > b.len() {
        bail!("truncated netmission");
    }
    if ch.moved > 0 && !zs.is_empty() {
        let (min, max) = zs.iter().fold((f32::MAX, f32::MIN), |(a, c), &z| (a.min(z), c.max(z)));
        put_f32(b, 12, f32_at(b, 12)?.min(min));
        put_f32(b, 24, f32_at(b, 24)?.max(max));
    }
    Ok(ch)
}

/// Vertsmission of block `(bx, by)`: the triangle corners follow the ground.
pub fn adjust_verts(b: &mut [u8], block: (u32, u32), ground: &Ground) -> Result<Changed> {
    let version = i32_at(b, 0)?;
    if version != 2 && version != 1 {
        bail!("vertsmission version {version}");
    }
    let origin = (block.0 as f32 * BLOCK, block.1 as f32 * BLOCK);
    let count = i32_at(b, 4)?.max(0) as usize;
    if 8 + count * 32 > b.len() {
        bail!("truncated vertsmission");
    }
    let mut ch = Changed::default();
    for i in 0..count {
        adjust_point(b, 8 + i * 32, origin, ground, &mut ch)?;
    }
    Ok(ch)
}

/// The path blocks of a cell (4 x 4).
pub fn cell_blocks(c: (i32, i32)) -> impl Iterator<Item = (u32, u32)> {
    let (x0, y0) = (c.0.max(0) as u32 * 4, c.1.max(0) as u32 * 4);
    (0..16).map(move |i| (x0 + i % 4, y0 + i / 4))
}

/// Pak folder of a path block (`game/worlds/<world>/paths/XXX_YYY`).
pub fn block_dir(world: &str, b: (u32, u32)) -> String {
    format!("game/worlds/{world}/paths/{:03}_{:03}", b.0, b.1)
}

/// The original pak bytes (the bake the navmesh matches), ignoring the editor's overlay.
pub fn original(path: &str) -> std::io::Result<Vec<u8>> {
    let pak = aa_pak::Pak::global();
    match pak.entry(path) {
        Some(e) if e.size == e.packed => pak.read_packed(path),
        _ => pak.read(path),
    }
}

/// Report of [`export`].
#[derive(Debug, Default)]
pub struct Report {
    pub files: usize,
    pub moved: usize,
    pub kept: usize,
    pub blocked_links: usize,
}

/// Rewrites the navmesh files of every block of the edited cells and writes them under each of `dests` (a client
/// tree root: `<dest>/game/worlds/...`, or with `strip_game` the pak overlay layout `<dest>/worlds/...`).
pub fn export(world: &str, edited: &HashMap<(i32, i32), Arc<CellHeights>>, server_dir: &Path, pak_overlay: Option<&Path>) -> Result<Report> {
    let mut originals: HashMap<(i32, i32), Option<CellHeights>> = HashMap::new();
    for &c in edited.keys() {
        let rel = crate::server_terrain::heightmap_rel(world, c, false);
        originals.insert(c, original(&format!("game/{rel}")).ok().and_then(|b| CellHeights::parse(&b).ok()));
    }
    let ground = |x: f32, y: f32| -> Option<(f32, f32, bool)> {
        let c = ((x / CELL).floor() as i32, (y / CELL).floor() as i32);
        let new = edited.get(&c)?;
        let old = originals.get(&c)?.as_ref()?;
        let (lx, ly) = (x - c.0 as f32 * CELL, y - c.1 as f32 * CELL);
        Some((old.mesh_height(lx, ly), new.mesh_height(lx, ly), new.is_hole(lx, ly) && !old.is_hole(lx, ly)))
    };
    let pak = aa_pak::Pak::global();
    let mut report = Report::default();
    let mut cells: Vec<_> = edited.keys().copied().collect();
    cells.sort();
    for c in cells {
        for b in cell_blocks(c) {
            let dir = block_dir(world, b);
            let names: Vec<String> = pak.list(&format!("{dir}/")).map(str::to_string).collect();
            for name in names {
                let file = name.rsplit('/').next().unwrap_or("").to_string();
                let net = file.starts_with("netmission") && file.ends_with(".bai");
                let verts = file.starts_with("vertsmission") && file.ends_with(".bai");
                if !net && !verts {
                    continue;
                }
                let mut bytes = original(&name).with_context(|| format!("read {name}"))?;
                let ch = if net { adjust_net(&mut bytes, b, &ground) } else { adjust_verts(&mut bytes, b, &ground) };
                let ch = match ch {
                    Ok(ch) => ch,
                    Err(e) => {
                        bevy::log::warn!("navmesh: {name}: {e:#}");
                        continue;
                    }
                };
                report.kept += ch.kept;
                if ch.moved == 0 && ch.blocked_links == 0 {
                    continue;
                }
                report.moved += ch.moved;
                report.blocked_links += ch.blocked_links;
                let lower = name.to_ascii_lowercase();
                let mut dests = vec![server_dir.join(&lower)];
                if let Some(o) = pak_overlay {
                    dests.push(o.join(lower.strip_prefix("game/").unwrap_or(&lower)));
                }
                for path in dests {
                    std::fs::create_dir_all(path.parent().context("no parent")?)?;
                    crate::server_terrain::backup(&path)?;
                    std::fs::write(&path, &bytes).with_context(|| format!("write {}", path.display()))?;
                }
                report.files += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pak_ok() -> bool {
        std::path::Path::new(aa_pak::DEFAULT_PAK).exists() || std::env::var_os("AA_PAK").is_some()
    }

    /// A few real blocks round trip byte for byte when nothing is edited, and their nodes sit on the terrain (the
    /// local -> world rule and the offsets are right).
    #[test]
    fn real_blocks_untouched_and_on_the_ground() {
        if !pak_ok() {
            return;
        }
        // main_world and an instance world (different cell count, same block layout)
        for (world, cell) in [("main_world", (15, 14)), ("instance_prologue", (1, 0)), ("instance_prologue", (0, 1))] {
            let (on, total) = on_the_ground(world, cell);
            println!("{world} {cell:?}: {on} of {total} navmesh points on the ground");
            assert!(total > 100, "{world}: {total} points");
            assert!(on * 2 > total, "{world}: only {on} of {total} navmesh points near the ground: wrong offsets?");
        }
    }

    fn on_the_ground(world: &str, cell: (i32, i32)) -> (usize, usize) {
        let hm = original(&format!("game/{}", crate::server_terrain::heightmap_rel(world, cell, false))).unwrap();
        let heights = CellHeights::parse(&hm).unwrap();
        let (mut on, mut total) = (0, 0);
        for b in cell_blocks(cell) {
            for kind in ["netmission0.bai", "vertsmission0.bai"] {
                let name = format!("{}/{kind}", block_dir(world, b));
                let Ok(orig) = original(&name) else { continue };
                let mut copy = orig.clone();
                let none = |_: f32, _: f32| None;
                if kind.starts_with("net") {
                    adjust_net(&mut copy, b, &none).unwrap();
                } else {
                    adjust_verts(&mut copy, b, &none).unwrap();
                }
                assert_eq!(copy, orig, "{name} unchanged");
                // count the points on the ground through a ground that reports +0 everywhere
                let mut probe = orig.clone();
                let g = |x: f32, y: f32| {
                    let (lx, ly) = (x - cell.0 as f32 * CELL, y - cell.1 as f32 * CELL);
                    let h = heights.mesh_height(lx, ly);
                    Some((h, h + 1.0, false))
                };
                let ch = if kind.starts_with("net") { adjust_net(&mut probe, b, &g).unwrap() } else { adjust_verts(&mut probe, b, &g).unwrap() };
                on += ch.moved;
                total += ch.moved + ch.kept;
            }
        }
        (on, total)
    }

    /// Synthetic block: a node on the ground moves with it, one 5 m above (a bridge) stays, a link to a node over a
    /// new hole is blocked.
    #[test]
    fn edited_copy() {
        let mut b = Vec::new();
        b.extend(55i32.to_le_bytes());
        for v in [0f32, 0., 10., 256., 256., 20.] {
            b.extend(v.to_le_bytes());
        }
        b.extend(3i32.to_le_bytes());
        for (id, x, z) in [(1u32, 10f32, 10.2f32), (2, 20., 15.0), (3, 200., 10.0)] {
            b.extend(id.to_le_bytes());
            for v in [1f32, 0., 0., 0., 0., 1., x, 30., z] {
                b.extend(v.to_le_bytes());
            }
            for v in [0i32, 0, 1, 2] {
                b.extend(v.to_le_bytes());
            }
            b.extend([2u8, 0, 0, 0]);
        }
        b.extend(1i32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        for v in [100f32, 30., 10., 2.0, 0., 0., 0., 0.] {
            b.extend(v.to_le_bytes());
        }
        b.extend([0u8, 1, 1, 0]);
        // block (2, 3): world x = 512 + local; hole for x > 600
        let g = |x: f32, _y: f32| Some((10.0, 13.0, x > 600.0));
        let ch = adjust_net(&mut b, (2, 3), &g).unwrap();
        assert_eq!(ch.moved, 3, "two nodes and the edge centre");
        assert_eq!(ch.kept, 1);
        assert_eq!(ch.blocked_links, 1);
        let node_z = |i: usize| f32_at(&b, 32 + i * 60 + 36).unwrap();
        assert!((node_z(0) - 13.2).abs() < 1e-4);
        assert_eq!(node_z(1), 15.0, "the bridge node stays");
        assert!((node_z(2) - 13.0).abs() < 1e-4);
        let lo = 32 + 3 * 60 + 4;
        assert_eq!(f32_at(&b, lo + 20).unwrap(), -1.0);
        assert!((f32_at(&b, lo + 16).unwrap() - 13.0).abs() < 1e-4);
        assert!((f32_at(&b, 24).unwrap() - 20.0).abs() < 1e-4, "bbox max stays");
    }
}
