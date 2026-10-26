//! Cover colour painting: edits the BC1 tiles of a cell's `cover.ctc` (see `aa_world::cover`) in place.
//!
//! The file is a quadtree of 128 x 128 px BC1 tiles, one tile set per level (level `n`: `2^n x 2^n` tiles over the
//! 1024 m cell, the engine's mip chain; image rows run along world X, columns along world Y). A tint dab decodes the
//! tiles of every level under the brush, blends their texels toward the target and re-encodes them at the same
//! place, so the edited file keeps the original layout, table and size byte for byte outside the touched tiles.
//! Missing fine tiles (coasts) stay missing: the client derives them from their edited parents.
//!
//! **The texels are not colours** (`terrain_layers.wesl`): `r' = (r + 0.001012) * 31 / 30`, `g' = g`,
//! `b' = 1 - r' - g'` is a chromaticity and the blue channel `B` the brightness, `colour = saturate(chroma * 3 * B)`.
//! The brush converts its colour to that encoding ([`encode`]) and blends encoded texels.
//!
//! The rendered cell keeps its cover image: the touched tiles' blocks are copied into the image's mip levels.

use std::collections::HashMap;

use aa_world::cover::{Cover, CoverFormat, TILE_PX, bc1_decode, bc1_encode};
use bevy::math::Vec2;
use bevy::prelude::Image;

const BLOCKS: usize = TILE_PX / 4;
const BLOCK_BYTES: usize = 8;

/// A tile of the quadtree: `(level, x, y)`.
pub type TileKey = (usize, usize, usize);

/// Byte offset of every tile of layer 0, and the deepest level.
fn tiles(bytes: &[u8]) -> Option<(HashMap<TileKey, usize>, usize)> {
    let cover = Cover::parse(bytes).ok()?;
    if cover.layers.first()?.format != CoverFormat::Bc1 {
        return None;
    }
    let base = bytes.as_ptr() as usize;
    let mut out = HashMap::new();
    for level in 0..=cover.max_level {
        for x in 0..1usize << level {
            for y in 0..1usize << level {
                if let Some(t) = cover.tile(0, level, x, y) {
                    out.insert((level, x, y), t.as_ptr() as usize - base);
                }
            }
        }
    }
    Some((out, cover.max_level))
}

/// The cover texel (0..1) of a gamma-space colour (0..1).
pub fn encode(colour: [f32; 3]) -> [f32; 3] {
    let sum = colour[0] + colour[1] + colour[2];
    if sum <= 1e-4 {
        return [1.0 / 3.0 * 30.0 / 31.0 - 0.001012, 1.0 / 3.0, 0.0];
    }
    let (r, g) = (colour[0] / sum, colour[1] / sum);
    [(r * 30.0 / 31.0 - 0.001012).clamp(0.0, 1.0), g.clamp(0.0, 1.0), (sum / 3.0).clamp(0.0, 1.0)]
}

/// The gamma-space colour a cover texel shows.
pub fn decode(texel: [f32; 3]) -> [f32; 3] {
    let r = (texel[0] + 0.001012) * 31.0 / 30.0;
    let g = texel[1];
    let b = 1.0 - r - g;
    [r, g, b].map(|c| (c * 3.0 * texel[2]).clamp(0.0, 1.0))
}

/// The decoded colour at cell-local metres `p` (finest level that has a tile there).
pub fn colour_at(bytes: &[u8], p: Vec2) -> Option<[f32; 3]> {
    let (offsets, max) = tiles(bytes)?;
    for level in (0..=max).rev() {
        let tile_m = 1024.0 / (1usize << level) as f32;
        let (x, y) = ((p.x / tile_m).floor() as usize, (p.y / tile_m).floor() as usize);
        let Some(&off) = offsets.get(&(level, x, y)) else { continue };
        let px = tile_m / TILE_PX as f32;
        let row = (((p.x - x as f32 * tile_m) / px) as usize).min(TILE_PX - 1);
        let col = (((p.y - y as f32 * tile_m) / px) as usize).min(TILE_PX - 1);
        let rgb = bc1_decode(&bytes[off..off + BLOCKS * BLOCKS * BLOCK_BYTES])[row * TILE_PX + col];
        return Some(decode(rgb.map(|c| c as f32 / 255.0)));
    }
    None
}

/// Blends the texels within `radius` metres of cell-local `centre` toward `texel` (weight `amount * falloff`) on every
/// level. Returns the edited file and the tiles it changed (`None`: nothing changed or not a BC1 cover).
pub fn tint(bytes: &[u8], centre: Vec2, radius: f32, texel: [f32; 3], amount: f32) -> Option<(Vec<u8>, Vec<TileKey>)> {
    let (offsets, max) = tiles(bytes)?;
    let target = texel.map(|c| c * 255.0);
    let mut out: Option<Vec<u8>> = None;
    let mut changed = Vec::new();
    for level in 0..=max {
        let tile_m = 1024.0 / (1usize << level) as f32;
        let px = tile_m / TILE_PX as f32;
        let side = 1usize << level;
        let lo = ((centre - radius) / tile_m).floor().max(Vec2::ZERO);
        let hi = ((centre + radius) / tile_m).floor().min(Vec2::splat((side - 1) as f32));
        if lo.x > hi.x || lo.y > hi.y {
            continue;
        }
        for x in lo.x as usize..=hi.x as usize {
            for y in lo.y as usize..=hi.y as usize {
                let Some(&off) = offsets.get(&(level, x, y)) else { continue };
                let len = BLOCKS * BLOCKS * BLOCK_BYTES;
                let mut rgb = bc1_decode(&bytes[off..off + len]);
                let mut any = false;
                // the pixels under the brush only
                let origin = Vec2::new(x as f32, y as f32) * tile_m;
                let r0 = (((centre.x - radius - origin.x) / px).floor().max(0.0)) as usize;
                let r1 = (((centre.x + radius - origin.x) / px).ceil().max(0.0) as usize).min(TILE_PX - 1);
                let c0 = (((centre.y - radius - origin.y) / px).floor().max(0.0)) as usize;
                let c1 = (((centre.y + radius - origin.y) / px).ceil().max(0.0) as usize).min(TILE_PX - 1);
                for row in r0..=r1 {
                    for col in c0..=c1 {
                        let p = origin + Vec2::new(row as f32 + 0.5, col as f32 + 0.5) * px;
                        let t = (1.0 - p.distance(centre) / radius).clamp(0.0, 1.0);
                        let w = (t * t * (3.0 - 2.0 * t) * amount).min(1.0);
                        if w <= 0.0 {
                            continue;
                        }
                        let c = &mut rgb[row * TILE_PX + col];
                        let new = [0, 1, 2].map(|i| (c[i] as f32 + (target[i] - c[i] as f32) * w).round().clamp(0.0, 255.0) as u8);
                        if new != *c {
                            *c = new;
                            any = true;
                        }
                    }
                }
                if any {
                    out.get_or_insert_with(|| bytes.to_vec())[off..off + len].copy_from_slice(&bc1_encode(&rgb));
                    changed.push((level, x, y));
                }
            }
        }
    }
    out.map(|o| (o, changed))
}

/// Copies tiles of `bytes` (all of them when `only` is `None`) into the cell's cover image, which was built from the
/// same file layout by `Cover::atlas` (`128 << max_level` px, one mip per level). `false` when the image does not
/// match.
pub fn upload(bytes: &[u8], only: Option<&[TileKey]>, image: &mut Image) -> bool {
    let Some((offsets, max)) = tiles(bytes) else { return false };
    let size = (TILE_PX << max) as u32;
    if image.texture_descriptor.size.width != size || image.texture_descriptor.mip_level_count != max as u32 + 1 {
        return false;
    }
    let Some(data) = image.data.as_mut() else { return false };
    // mip m is level max - m: offsets of each level's data
    let mut level_start = vec![0usize; max + 1];
    let mut at = 0;
    for mip in 0..=max {
        let level = max - mip;
        level_start[level] = at;
        let blocks_side = (1usize << level) * BLOCKS;
        at += blocks_side * blocks_side * BLOCK_BYTES;
    }
    if data.len() < at {
        return false;
    }
    let keys: Vec<TileKey> = match only {
        Some(k) => k.to_vec(),
        None => offsets.keys().copied().collect(),
    };
    for (level, x, y) in keys {
        let Some(&off) = offsets.get(&(level, x, y)) else { continue };
        let blocks_side = (1usize << level) * BLOCKS;
        let row_bytes = BLOCKS * BLOCK_BYTES;
        for r in 0..BLOCKS {
            let src = &bytes[off + r * row_bytes..off + (r + 1) * row_bytes];
            let dst = level_start[level] + ((x * BLOCKS + r) * blocks_side + y * BLOCKS) * BLOCK_BYTES;
            data[dst..dst + row_bytes].copy_from_slice(src);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texel_encoding_round_trips() {
        for c in [[0.3, 0.5, 0.2], [0.8, 0.7, 0.6], [0.1, 0.1, 0.1], [0.5, 0.25, 0.1]] {
            let back = decode(encode(c));
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 1e-3, "{c:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn tint_changes_only_the_tiles_under_the_brush_on_a_real_cover() {
        if !aa_world::pak_available() {
            return;
        }
        let bytes = aa_pak::Pak::global().read("worlds/main_world/cells/015_014/client/terrain/cover.ctc").unwrap();
        let target = encode([0.8, 0.1, 0.1]);
        let (edited, changed) = tint(&bytes, Vec2::new(160.0, 928.0), 10.0, target, 1.0).unwrap();
        assert_eq!(edited.len(), bytes.len());
        assert!(changed.iter().any(|k| k.0 == 0) && changed.iter().any(|k| k.0 == 4), "{changed:?}");
        let centre = colour_at(&edited, Vec2::new(160.0, 928.0)).unwrap();
        assert!(centre[0] > 0.6 && centre[1] < 0.25, "{centre:?}");
        // far away nothing changed
        let (a, b) = (colour_at(&bytes, Vec2::new(600.0, 300.0)).unwrap(), colour_at(&edited, Vec2::new(600.0, 300.0)).unwrap());
        assert_eq!(a, b);
        // still a valid cover with the same atlas layout
        let atlas = Cover::parse(&edited).unwrap().atlas(0);
        let mut image = aa_world::terrain::cover_image(Cover::parse(&bytes).unwrap().atlas(0));
        assert!(upload(&edited, Some(&changed), &mut image));
        assert_eq!(image.data.as_ref().unwrap(), &atlas.data);
    }
}
