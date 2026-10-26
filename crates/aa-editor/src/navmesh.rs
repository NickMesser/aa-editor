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
//! Walls: a link between two nodes on the ground is blocked as well when the edit raised the ground between them into a
//! wall ([`WALL`] above the line between the nodes, where it was not before) or made it steeper than [`MAX_SLOPE`]
//! (where it was not), so NPCs no longer path across sculpted walls and cliffs. The other way round, a blocked link
//! between two nodes on the ground is **reopened** when the ground between them was a wall or too steep before the
//! edit and can be walked now (a cliff smoothed into a ramp): its `max_pass_radius` becomes half the shared edge's
//! length, as the bake writes it. Links blocked on walkable ground (obstacles, forbidden areas) stay blocked.
//!
//! The bake triangulates all the ground (walkability is in the links), so a sculpted hill or valley is inside some
//! triangle whose plane no longer follows it: [`crate::navmesh_add::refine`] splits those triangles (after the
//! adjustment above) so the server's heights and links follow the new shape.
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
/// Metres the edited ground may rise above the line between two linked nodes before the link counts as walled off.
pub const WALL: f32 = 1.5;
/// Steepest walkable ground (rise over run, about 50°) along a link after an edit.
pub const MAX_SLOPE: f32 = 1.2;
const CELL: f32 = 1024.0;

/// Ground before and after the edit at a world XY: `(old, new, new_is_hole)`, `None` outside edited cells.
pub type Ground<'a> = dyn Fn(f32, f32) -> Option<(f32, f32, bool)> + 'a;

/// What one file's adjustment changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Changed {
    pub moved: usize,
    pub kept: usize,
    /// Links blocked (holes and walls).
    pub blocked_links: usize,
    /// Of those, links blocked by a new wall or slope.
    pub walls: usize,
    /// Blocked links reopened because the edit made the ground between them walkable.
    pub opened: usize,
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

/// Netmission of block `(bx, by)`: nodes, link edge centres and the bbox Z follow the ground, links over new holes and
/// walls are blocked and links over ground the edit made walkable reopened (their radius from the triangle corners in
/// `verts`, the paired vertsmission, else 1 m). Patches `b` in place.
pub fn adjust_net(b: &mut [u8], block: (u32, u32), ground: &Ground, verts: Option<&[u8]>) -> Result<Changed> {
    let version = i32_at(b, 0)?;
    if version != 55 && version != 54 {
        bail!("netmission version {version}");
    }
    let origin = (block.0 as f32 * BLOCK, block.1 as f32 * BLOCK);
    let mut ch = Changed::default();
    let nodes = i32_at(b, 28)?.max(0) as usize;
    let mut zs: Vec<f32> = Vec::new();
    let mut hole_ids = std::collections::HashSet::new();
    let mut corners_of: HashMap<u32, [i32; 3]> = HashMap::new();
    // node id -> (world x, y, z before, z after) of the nodes that sat on the original ground
    let mut on_ground: HashMap<u32, (f32, f32, f32, f32)> = HashMap::new();
    for i in 0..nodes {
        let o = 32 + i * 60;
        let before = f32_at(b, o + 36)?;
        let (_, hole) = adjust_point(b, o + 28, origin, ground, &mut ch)?;
        let id = i32_at(b, o)? as u32;
        corners_of.insert(id, [i32_at(b, o + 44)?, i32_at(b, o + 48)?, i32_at(b, o + 52)?]);
        if hole {
            hole_ids.insert(id);
        }
        let (x, y) = (f32_at(b, o + 28)? + origin.0, f32_at(b, o + 32)? + origin.1);
        if ground(x, y).is_some_and(|(old, _, _)| (before - old).abs() <= TOLERANCE) {
            on_ground.insert(id, (x, y, before, f32_at(b, o + 36)?));
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
        if f32_at(b, o + 20)? == -1.0 {
            if let (Some(a), Some(c)) = (on_ground.get(&src), on_ground.get(&dst))
                && !hole_ids.contains(&src)
                && !hole_ids.contains(&dst)
                && rough(*a, *c, ground, false)
                && !rough(*a, *c, ground, true)
            {
                let edge = (b[o + 40] as usize % 3, b[o + 41] as usize % 3);
                let radius = verts
                    .zip(corners_of.get(&src))
                    .and_then(|(v, c)| Some((vert_xy(v, c[edge.0])?, vert_xy(v, c[edge.1])?)))
                    .map_or(1.0, |(p, q)| (((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt() / 2.0 * 100.0).round() / 100.0);
                put_f32(b, o + 20, radius);
                ch.opened += 1;
            }
            continue;
        }
        let walled = match (on_ground.get(&src), on_ground.get(&dst)) {
            (Some(a), Some(c)) => walled(*a, *c, ground),
            _ => false,
        };
        if hole_ids.contains(&src) || hole_ids.contains(&dst) || walled {
            put_f32(b, o + 20, -1.0);
            ch.blocked_links += 1;
            ch.walls += walled as usize;
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

/// XY of record `k` of a vertsmission.
fn vert_xy(v: &[u8], k: i32) -> Option<(f32, f32)> {
    let o = 8 + usize::try_from(k).ok()? * 32;
    (o + 8 <= v.len()).then(|| (f32_at(v, o).unwrap_or(0.0), f32_at(v, o + 4).unwrap_or(0.0)))
}

/// Whether the ground (before the edit, or after with `new`) between two nodes on it (`(x, y, z before, z after)`)
/// rises more than [`WALL`] above the line between them, is steeper than [`MAX_SLOPE`] or (after) has a hole. Ground
/// outside the edited cells counts as rough (nothing to judge by).
fn rough(a: (f32, f32, f32, f32), c: (f32, f32, f32, f32), ground: &Ground, new: bool) -> bool {
    let run = ((c.0 - a.0).powi(2) + (c.1 - a.1).powi(2)).sqrt();
    let n = (run.ceil() as usize).clamp(2, 512);
    let step = run / n as f32;
    let mut prev: Option<f32> = None;
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let Some((old, now, hole)) = ground(a.0 + (c.0 - a.0) * t, a.1 + (c.1 - a.1) * t) else { return true };
        let (z, line) = if new { (now, a.3 + (c.3 - a.3) * t) } else { (old, a.2 + (c.2 - a.2) * t) };
        if (new && hole) || z - line > WALL {
            return true;
        }
        if let Some(p) = prev
            && step > 0.0
            && (z - p).abs() / step > MAX_SLOPE
        {
            return true;
        }
        prev = Some(z);
    }
    false
}

/// Whether the edit put a wall or a too steep slope between two linked nodes on the ground (`(x, y, z before, z after)`).
fn walled(a: (f32, f32, f32, f32), c: (f32, f32, f32, f32), ground: &Ground) -> bool {
    let run = ((c.0 - a.0).powi(2) + (c.1 - a.1).powi(2)).sqrt();
    let n = (run.ceil() as usize).clamp(2, 512);
    let step = run / n as f32;
    let mut prev: Option<(f32, f32)> = None;
    for k in 0..=n {
        let t = k as f32 / n as f32;
        let (x, y) = (a.0 + (c.0 - a.0) * t, a.1 + (c.1 - a.1) * t);
        let Some((old, new, _)) = ground(x, y) else {
            prev = None;
            continue;
        };
        let line_old = a.2 + (c.2 - a.2) * t;
        let line_new = a.3 + (c.3 - a.3) * t;
        if new - line_new > WALL && old - line_old <= WALL {
            return true;
        }
        if let Some((p_old, p_new)) = prev
            && step > 0.0
            && (new - p_new).abs() / step > MAX_SLOPE
            && (old - p_old).abs() / step <= MAX_SLOPE
        {
            return true;
        }
        prev = Some((old, new));
    }
    false
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
    pub walls: usize,
    pub opened: usize,
    /// Triangles split ([`crate::navmesh_add::refine`]) and the triangles they became.
    pub split: usize,
    pub children: usize,
}

/// Rewrites the navmesh files of every block of the edited cells (and the ring of blocks around them, which repeat the
/// border triangles) and writes them under `server_dir` (`<dir>/game/worlds/...`) and the pak overlay
/// (`<overlay>/worlds/...`).
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
    let mut blocks: Vec<(u32, u32)> = edited
        .keys()
        .flat_map(|&c| cell_blocks(c))
        .flat_map(|b| (-1i64..=1).flat_map(move |dx| (-1i64..=1).map(move |dy| (b.0 as i64 + dx, b.1 as i64 + dy))))
        .filter(|&(x, y)| x >= 0 && y >= 0)
        .map(|(x, y)| (x as u32, y as u32))
        .collect();
    blocks.sort();
    blocks.dedup();
    for b in blocks {
        let dir = block_dir(world, b);
        let names: Vec<String> = pak.list(&format!("{dir}/")).map(str::to_string).collect();
        // netmissionN.bai with its vertsmissionN.bai
        let missions: Vec<String> = names
            .iter()
            .filter_map(|n| n.rsplit('/').next()?.strip_prefix("netmission")?.strip_suffix(".bai").map(str::to_string))
            .collect();
        for m in missions {
            let (net_name, verts_name) = (format!("{dir}/netmission{m}.bai"), format!("{dir}/vertsmission{m}.bai"));
            let mut net = original(&net_name).with_context(|| format!("read {net_name}"))?;
            let mut verts = original(&verts_name).ok();
            let mut ch = match adjust_net(&mut net, b, &ground, verts.as_deref()) {
                Ok(ch) => ch,
                Err(e) => {
                    bevy::log::warn!("navmesh: {net_name}: {e:#}");
                    continue;
                }
            };
            let mut verts_changed = false;
            if let Some(v) = verts.as_mut() {
                match adjust_verts(v, b, &ground) {
                    Ok(c) => {
                        verts_changed = c.moved > 0;
                        ch.moved += c.moved;
                        ch.kept += c.kept;
                    }
                    Err(e) => bevy::log::warn!("navmesh: {verts_name}: {e:#}"),
                }
            }
            let mut refined = crate::navmesh_add::Refined::default();
            if let Some(v) = verts.as_mut() {
                match crate::navmesh_add::refine(&mut net, v, b, &ground) {
                    Ok(r) => refined = r,
                    Err(e) => bevy::log::warn!("navmesh: {net_name}: cannot split triangles: {e:#}"),
                }
            }
            report.kept += ch.kept;
            report.moved += ch.moved;
            report.blocked_links += ch.blocked_links;
            report.walls += ch.walls;
            report.opened += ch.opened;
            report.split += refined.split;
            report.children += refined.children;
            let net_changed = ch.moved > 0 || ch.blocked_links > 0 || ch.opened > 0 || refined.split > 0;
            let mut files: Vec<(&str, &[u8])> = Vec::new();
            if net_changed {
                files.push((&net_name, &net));
            }
            if let Some(v) = verts.as_deref()
                && (verts_changed || refined.split > 0)
            {
                files.push((&verts_name, v));
            }
            for (name, bytes) in files {
                let lower = name.to_ascii_lowercase();
                let mut dests = vec![server_dir.join(&lower)];
                if let Some(o) = pak_overlay {
                    dests.push(o.join(lower.strip_prefix("game/").unwrap_or(&lower)));
                }
                for path in dests {
                    std::fs::create_dir_all(path.parent().context("no parent")?)?;
                    crate::server_terrain::backup(&path)?;
                    std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
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
                    adjust_net(&mut copy, b, &none, None).unwrap();
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
                let ch = if kind.starts_with("net") { adjust_net(&mut probe, b, &g, None).unwrap() } else { adjust_verts(&mut probe, b, &g).unwrap() };
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
        let ch = adjust_net(&mut b, (2, 3), &g, None).unwrap();
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

    /// Two nodes 40 m apart on flat ground, linked: a wall sculpted between them blocks the link, a gentle raise of
    /// everything does not, nor does a wall that was already there.
    #[test]
    fn sculpted_walls_block_links() {
        let block = || {
            let mut b = Vec::new();
            b.extend(55i32.to_le_bytes());
            for v in [0f32, 0., 0., 256., 256., 20.] {
                b.extend(v.to_le_bytes());
            }
            b.extend(2i32.to_le_bytes());
            for (id, x) in [(1u32, 10f32), (2, 50.)] {
                b.extend(id.to_le_bytes());
                for v in [1f32, 0., 0., 0., 0., 1., x, 30., 10.] {
                    b.extend(v.to_le_bytes());
                }
                for v in [0i32, 0, 1, 2] {
                    b.extend(v.to_le_bytes());
                }
                b.extend([2u8, 0, 0, 0]);
            }
            b.extend(1i32.to_le_bytes());
            b.extend(1u32.to_le_bytes());
            b.extend(2u32.to_le_bytes());
            for v in [30f32, 30., 10., 2.0, 0., 0., 0., 0.] {
                b.extend(v.to_le_bytes());
            }
            b.extend([0u8, 1, 1, 0]);
            b
        };
        let radius = |b: &[u8]| f32_at(b, 32 + 2 * 60 + 4 + 20).unwrap();
        // a 6 m wall across x = 28..32 (block (0, 0): world = local)
        let mut b = block();
        let wall = |x: f32, _y: f32| Some((10.0, if (28.0..32.0).contains(&x) { 16.0 } else { 10.0 }, false));
        let ch = adjust_net(&mut b, (0, 0), &wall, None).unwrap();
        assert_eq!((ch.blocked_links, ch.walls), (1, 1));
        assert_eq!(radius(&b), -1.0);
        // everything 2 m higher: the nodes follow, the link stays open
        let mut b = block();
        let raise = |_x: f32, _y: f32| Some((10.0, 12.0, false));
        let ch = adjust_net(&mut b, (0, 0), &raise, None).unwrap();
        assert_eq!(ch.blocked_links, 0);
        assert_eq!(radius(&b), 2.0);
        // the wall was there before: not ours to block
        let mut b = block();
        let old_wall = |x: f32, _y: f32| {
            let h = if (28.0..32.0).contains(&x) { 16.0 } else { 10.0 };
            Some((h, h + 0.5, false))
        };
        assert_eq!(adjust_net(&mut b, (0, 0), &old_wall, None).unwrap().blocked_links, 0);
    }

    /// The same two nodes with their link blocked: smoothing a wall that stood between them reopens it, a block on
    /// flat ground (an obstacle) stays.
    #[test]
    fn smoothed_walls_reopen_links() {
        let block = |radius: f32| {
            let mut b = Vec::new();
            b.extend(55i32.to_le_bytes());
            for v in [0f32, 0., 0., 256., 256., 20.] {
                b.extend(v.to_le_bytes());
            }
            b.extend(2i32.to_le_bytes());
            for (id, x) in [(1u32, 10f32), (2, 50.)] {
                b.extend(id.to_le_bytes());
                for v in [0f32, 0., 0., 0., 0., 0., x, 30., 10.] {
                    b.extend(v.to_le_bytes());
                }
                for v in [0i32, 0, 1, 2] {
                    b.extend(v.to_le_bytes());
                }
                b.extend([2u8, 0, 0, 0]);
            }
            b.extend(1i32.to_le_bytes());
            b.extend(1u32.to_le_bytes());
            b.extend(2u32.to_le_bytes());
            for v in [30f32, 30., 10., radius, 1., 0., 0., 327.66] {
                b.extend(v.to_le_bytes());
            }
            b.extend([0u8, 1, 1, 49]);
            b
        };
        // corners 0 and 1 of node 1's triangle: (28, 20) and (28, 40), a 20 m edge
        let mut verts = 2i32.to_le_bytes().to_vec();
        verts.extend(3i32.to_le_bytes());
        for p in [[28f32, 20., 10.], [28., 40., 10.], [5., 30., 10.]] {
            p.iter().for_each(|v| verts.extend(v.to_le_bytes()));
            verts.extend([0u8; 16]);
            verts.extend([1u8, 0, 0, 0]);
        }
        let radius = |b: &[u8]| f32_at(b, 32 + 2 * 60 + 4 + 20).unwrap();
        let smoothed = |x: f32, _y: f32| Some((if (28.0..32.0).contains(&x) { 16.0 } else { 10.0 }, 10.0, false));
        let mut b = block(-1.0);
        assert_eq!(adjust_net(&mut b, (0, 0), &smoothed, Some(&verts)).unwrap().opened, 1);
        assert_eq!(radius(&b), 10.0);
        let flat = |_x: f32, _y: f32| Some((10.0, 10.5, false));
        let mut b = block(-1.0);
        assert_eq!(adjust_net(&mut b, (0, 0), &flat, Some(&verts)).unwrap().opened, 0);
        assert_eq!(radius(&b), -1.0);
    }
}
