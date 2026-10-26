//! Writes edited heights back into a cell's `heightmap.dat` (the format `aa_world::heightmap` reads).
//!
//! The original file is the template: the header, the quadtree order and every byte the reader skips are copied.
//! Each leaf node (one 64 m sector) is re-quantised as a full 33 x 33 node with its own `offset` / `range`
//! (`height = offset + (raw & 0xFFE0) * range`, node versions < 7) with the edited five surface bits of each sample
//! (31 = hole). A sample whose surface is unchanged keeps its original bits, including the far-edge copies the reader
//! ignores. Each node's `hasHoles` byte is the edited `hole_sectors` gate of its sector (a parent: any of its leaves),
//! leaf and parent boxes get the new height bounds and the header's chunk size the new length.

use aa_world::coords::{CELL_UNITS, UNIT_SIZE};
use aa_world::heightmap::{CellHeights, GRID};
use anyhow::{Result, bail, ensure};

const NODE_FIXED: usize = 45; // version..unkCount
const SECTOR: usize = 33;

fn u32_at(b: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(b.get(at..at + 4).ok_or_else(|| anyhow::anyhow!("truncated at {at}"))?.try_into()?))
}

fn f32_at(b: &[u8], at: usize) -> Result<f32> {
    Ok(f32::from_bits(u32_at(b, at)?))
}

struct Leaf {
    data: Vec<u16>,
    offset: f32,
    range: f32,
    lo: f32,
    hi: f32,
    holes: bool,
}

struct RawNode {
    /// Byte range of the whole node in the original.
    start: usize,
    end: usize,
    version: u8,
    min: [f32; 3],
    max: [f32; 3],
    has_holes: bool,
    size: usize,
    data: Vec<u16>,
}

fn read_nodes(b: &[u8]) -> Result<(usize, usize, Vec<RawNode>)> {
    let version = *b.first().ok_or_else(|| anyhow::anyhow!("empty heightmap"))?;
    let chunk = u32_at(b, 4)? as usize;
    let header = if version >= 24 { 32 + 128 } else { 32 };
    ensure!(chunk <= b.len() && chunk >= header, "bad chunk size {chunk}");
    let mut nodes = Vec::new();
    let mut at = header;
    while at < chunk {
        let start = at;
        let version = b[at];
        let min = [f32_at(b, at + 4)?, f32_at(b, at + 8)?, f32_at(b, at + 12)?];
        let max = [f32_at(b, at + 16)?, f32_at(b, at + 20)?, f32_at(b, at + 24)?];
        let has_holes = *b.get(at + 28).ok_or_else(|| anyhow::anyhow!("truncated node"))? != 0;
        let size = u32_at(b, at + 37)? as i32;
        let unk = u32_at(b, at + 41)? as i32;
        if !(0..=1024).contains(&size) || size == 1 {
            bail!("node size {size}");
        }
        let size = size as usize;
        let mut data = Vec::with_capacity(size * size);
        let mut p = at + NODE_FIXED;
        for _ in 0..size * size {
            data.push(u16::from_le_bytes([*b.get(p).ok_or_else(|| anyhow::anyhow!("truncated node"))?, b[p + 1]]));
            p += 2;
        }
        p += 4 + 16;
        if unk >= 1 {
            p = (p + unk as usize).min(b.len());
        }
        p = (p + 36).min(b.len());
        at = p;
        nodes.push(RawNode { start, end: at, version, min, max, has_holes, size, data });
    }
    ensure!(at == chunk, "nodes overrun the chunk");
    Ok((header, chunk, nodes))
}

/// The original `heightmap.dat` bytes with the heights of `edited`.
pub fn write(original: &[u8], edited: &CellHeights) -> Result<Vec<u8>> {
    ensure!(edited.heights.len() == GRID * GRID, "edited heights are not a cell");
    let (header, chunk, nodes) = read_nodes(original)?;
    let before = CellHeights::parse(original)?;
    let root = nodes.first().ok_or_else(|| anyhow::anyhow!("no nodes"))?;
    let (ox, oy) = (root.min[0], root.min[1]);
    // new data, offset, range, z bounds and hole gate per leaf
    let mut leaves: Vec<Option<Leaf>> = Vec::with_capacity(nodes.len());
    for n in &nodes {
        let sx = ((n.min[0] - ox) / UNIT_SIZE).round() as isize;
        let sy = ((n.min[1] - oy) / UNIT_SIZE).round() as isize;
        if n.size == 0 || sx < 0 || sy < 0 || sx as usize + 32 > CELL_UNITS || sy as usize + 32 > CELL_UNITS {
            leaves.push(None);
            continue;
        }
        ensure!(n.version < 7, "node version {} is not a 1.2 heightmap", n.version);
        let (sx, sy) = (sx as usize, sy as usize);
        let h = |x: usize, y: usize| edited.heights[(sy + y) * GRID + sx + x];
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for y in 0..SECTOR {
            for x in 0..SECTOR {
                lo = lo.min(h(x, y));
                hi = hi.max(h(x, y));
            }
        }
        let range = if hi > lo { (hi - lo) / 0xFFE0 as f32 } else { 0.0 };
        let mut data = vec![0u16; SECTOR * SECTOR];
        for x in 0..SECTOR {
            for y in 0..SECTOR {
                let i = (sy + y) * GRID + sx + x;
                let surface = if n.size == SECTOR && edited.surface[i] == before.surface[i] {
                    n.data[x * SECTOR + y] & 0x1F
                } else {
                    edited.surface[i] as u16 & 0x1F
                };
                let q = if range > 0.0 { ((h(x, y) - lo) / (range * 32.0)).round().clamp(0.0, 2047.0) as u16 } else { 0 };
                data[x * SECTOR + y] = (q << 5) | surface;
            }
        }
        let holes = edited.hole_sectors[sy / 32 * 16 + sx / 32];
        leaves.push(Some(Leaf { data, offset: lo, range, lo, hi, holes }));
    }
    // parent boxes and hole gates: the union of the leaves inside them
    let bounds: Vec<(f32, f32, bool)> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if let Some(l) = &leaves[i] {
                return (l.lo, l.hi, l.holes);
            }
            let (mut lo, mut hi, mut holes, mut any) = (f32::MAX, f32::MIN, false, false);
            for (j, m) in nodes.iter().enumerate() {
                if let Some(l) = &leaves[j] {
                    let cx = (m.min[0] + m.max[0]) * 0.5;
                    let cy = (m.min[1] + m.max[1]) * 0.5;
                    if cx >= n.min[0] && cx <= n.max[0] && cy >= n.min[1] && cy <= n.max[1] {
                        lo = lo.min(l.lo);
                        hi = hi.max(l.hi);
                        holes |= l.holes;
                        any = true;
                    }
                }
            }
            if any { (lo, hi, holes) } else { (n.min[2], n.max[2], n.has_holes) }
        })
        .collect();

    let mut out = Vec::with_capacity(original.len() + 64 * 1024);
    out.extend_from_slice(&original[..header]);
    for (i, n) in nodes.iter().enumerate() {
        let raw = &original[n.start..n.end];
        let (lo, hi, holes) = bounds[i];
        let mut fixed = raw[..NODE_FIXED].to_vec();
        fixed[12..16].copy_from_slice(&lo.to_le_bytes());
        fixed[24..28].copy_from_slice(&hi.to_le_bytes());
        fixed[28] = holes as u8;
        let tail_from = NODE_FIXED + n.size * n.size * 2;
        match &leaves[i] {
            Some(Leaf { data, offset, range, .. }) => {
                fixed[29..33].copy_from_slice(&offset.to_le_bytes());
                fixed[33..37].copy_from_slice(&range.to_le_bytes());
                fixed[37..41].copy_from_slice(&(SECTOR as i32).to_le_bytes());
                out.extend_from_slice(&fixed);
                for v in data {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
            None => {
                out.extend_from_slice(&fixed);
                out.extend_from_slice(&raw[NODE_FIXED..tail_from]);
            }
        }
        out.extend_from_slice(&raw[tail_from..]);
    }
    let new_chunk = out.len() as u32;
    out[4..8].copy_from_slice(&new_chunk.to_le_bytes());
    out.extend_from_slice(&original[chunk..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_a_real_cell_within_quantisation_and_keeps_surfaces() {
        if !aa_world::pak_available() {
            return;
        }
        let bytes = aa_pak::Pak::global().read("worlds/main_world/cells/015_015/client/terrain/heightmap.dat").unwrap();
        let original = CellHeights::parse(&bytes).unwrap();
        let same = CellHeights::parse(&write(&bytes, &original).unwrap()).unwrap();
        let worst = original.heights.iter().zip(&same.heights).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(worst < 0.05, "round trip error {worst} m");
        assert_eq!(original.surface, same.surface);
        assert_eq!(original.hole_sectors, same.hole_sectors);

        let mut edited = original.clone();
        for iy in 100..140 {
            for ix in 200..260 {
                edited.heights[iy * GRID + ix] += 25.0;
            }
        }
        let back = CellHeights::parse(&write(&bytes, &edited).unwrap()).unwrap();
        let i = 120 * GRID + 230;
        assert!((back.heights[i] - edited.heights[i]).abs() < 0.05, "{} vs {}", back.heights[i], edited.heights[i]);
        assert!((back.max_z - edited.heights.iter().copied().fold(f32::MIN, f32::max)).abs() < 0.05);
    }

    #[test]
    fn paint_and_holes_survive_a_write() {
        if !aa_world::pak_available() {
            return;
        }
        let bytes = aa_pak::Pak::global().read("worlds/main_world/cells/015_015/client/terrain/heightmap.dat").unwrap();
        let original = CellHeights::parse(&bytes).unwrap();
        assert!(original.hole_sectors.iter().all(|h| !h), "the spawn cell has no holes");
        let mut edited = original.clone();
        // paint slot 2 over a block that crosses sector edges (x 60..70 crosses 64)
        for iy in 50..90 {
            for ix in 60..70 {
                edited.surface[iy * GRID + ix] = 2;
            }
        }
        // a hole of 6 x 4 samples in sector (5, 6), and one on the far cell edge
        for iy in 200..204 {
            for ix in 170..176 {
                edited.surface[iy * GRID + ix] = 31;
            }
        }
        edited.hole_sectors[200 / 32 * 16 + 170 / 32] = true;
        let back = CellHeights::parse(&write(&bytes, &edited).unwrap()).unwrap();
        assert_eq!(back.surface, edited.surface);
        assert_eq!(back.hole_sectors, edited.hole_sectors);
        assert!(back.quad_is_hole(172, 201, 1));
        assert!(!back.quad_is_hole(180, 201, 1));
        let worst = original.heights.iter().zip(&back.heights).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(worst < 0.05, "heights moved {worst} m");
        // the root and the sector's ancestors carry the gate too
        let (_, _, nodes) = read_nodes(&write(&bytes, &edited).unwrap()).unwrap();
        let gated: Vec<bool> = nodes.iter().map(|n| n.has_holes).collect();
        assert!(gated[0], "root hasHoles");
        assert_eq!(gated.iter().filter(|g| **g).count(), 5, "root, three parents and the leaf");

        // erasing the hole again gives the original file's surfaces and gates back
        let restored = CellHeights::parse(&write(&write(&bytes, &edited).unwrap(), &original).unwrap()).unwrap();
        assert_eq!(restored.surface, original.surface);
        assert_eq!(restored.hole_sectors, original.hole_sectors);
    }

    #[test]
    fn unmodified_file_keeps_every_surface_bit() {
        if !aa_world::pak_available() {
            return;
        }
        // a cell with authored holes: far-edge copies and gates must stay byte-identical
        let bytes = aa_pak::Pak::global().read("worlds/main_world/cells/020_008/client/terrain/heightmap.dat").unwrap();
        let original = CellHeights::parse(&bytes).unwrap();
        let out = write(&bytes, &original).unwrap();
        let (_, _, a) = read_nodes(&bytes).unwrap();
        let (_, _, b) = read_nodes(&out).unwrap();
        assert_eq!(a.len(), b.len());
        for (n, m) in a.iter().zip(&b) {
            assert_eq!(n.has_holes, m.has_holes);
            if n.size == SECTOR {
                let sa: Vec<u16> = n.data.iter().map(|v| v & 0x1F).collect();
                let sb: Vec<u16> = m.data.iter().map(|v| v & 0x1F).collect();
                assert_eq!(sa, sb);
            }
        }
        let same = CellHeights::parse(&out).unwrap();
        assert_eq!(same.surface, original.surface);
        assert_eq!(same.hole_sectors, original.hole_sectors);
    }
}
