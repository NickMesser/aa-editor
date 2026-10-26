//! Terrain sculpting, texture painting and holes on the streamed terrain.
//!
//! Edited cells are full copies of their [`CellHeights`] (heights and the five surface bits per sample); the loaded
//! [`TerrainPatch`]es of a cell get the copy (`set_heights`). Height and hole edits rebuild the touched patch meshes
//! (`build_patch_mesh` cuts hole quads), surface edits rewrite the cell's surface map in place
//! ([`crate::terrain_paint`]), so every edit shows in the real terrain renderer with its materials and LODs. Saving writes each edited cell's `heightmap.dat` into the pak overlay folder
//! ([`crate::heightmap_write`]); the editor and the client (`AA_PAK_OVERLAY`) then load it like the original.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aa_world::coords::{CELL_SIZE, UNIT_SIZE};
use aa_world::heightmap::{CellHeights, GRID};
use aa_world::stream::TerrainPatch;
use aa_world::terrain::{PATCH_UNITS, PATCHES, build_patch_mesh};
use aa_world::terrain_layers::TerrainMaterial;
use aa_world::{TerrainQuery, cry_to_bevy};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::cover_paint::TileKey;
use crate::terrain_paint::{PaletteEntry, SurfaceTables, unhole_surface, update_surface_map};
use crate::tools::Cursor;
use crate::{Editor, Mode, Request};

/// The surface value of a hole.
pub const HOLE: u8 = 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushKind {
    Raise,
    Lower,
    Smooth,
    Flatten,
    /// Paint [`Brush::paint`] (a surface type) on the samples within the radius.
    Paint,
    /// Cut holes (surface 31); with [`Brush::erase`] or Shift held, fill them again.
    Hole,
    /// Blend the cover colour toward [`Brush::tint`] ([`crate::cover_paint`]); Alt+click picks the ground's colour.
    Tint,
}

#[derive(Resource)]
pub struct Brush {
    pub kind: BrushKind,
    /// Metres.
    pub radius: f32,
    /// Metres per second at the centre (Smooth / Flatten: fraction per second).
    pub strength: f32,
    /// Flatten target height, taken when a stroke starts.
    flatten_to: Option<f32>,
    /// The material the paint brush puts down (`DetailMaterial`, see [`crate::terrain_paint`]).
    pub paint: Option<String>,
    /// The hole brush fills holes instead of cutting them.
    pub erase: bool,
    /// Surface types of the cell under the cursor.
    pub palette: Vec<PaletteEntry>,
    pub palette_cell: Option<(i32, i32)>,
    /// The tint brush's colour (gamma space, 0..1).
    pub tint: [f32; 3],
    /// What the last stroke frame cost (ms), for the panel.
    pub last_ms: f32,
}

/// The cells' state from before one stroke.
#[derive(Default)]
struct Stroke {
    cells: HashMap<(i32, i32), Option<Arc<CellHeights>>>,
    covers: HashMap<(i32, i32), Option<Arc<Vec<u8>>>>,
}

impl Stroke {
    fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.covers.is_empty()
    }
}

/// The heights the editor works with: edited cells, else the pak's.
#[derive(Resource, Default)]
pub struct Heights {
    pub edited: HashMap<(i32, i32), Arc<CellHeights>>,
    /// Edited since the last save.
    pub unsaved: HashSet<(i32, i32)>,
    /// Edited `cover.ctc` files (the cover colour, [`crate::cover_paint`]).
    pub covers: HashMap<(i32, i32), Arc<Vec<u8>>>,
    pub covers_unsaved: HashSet<(i32, i32)>,
    /// Cell copies from before each stroke.
    undo: Vec<Stroke>,
    stroke: Option<Stroke>,
}

impl Heights {
    pub fn cell(&self, query: &TerrainQuery, c: (i32, i32)) -> Option<Arc<CellHeights>> {
        self.edited.get(&c).cloned().or_else(|| query.cell(c.0, c.1))
    }

    /// Whether a cry-space point lies over a hole (the native point query, like `TerrainQuery::ground_at`).
    pub fn is_hole_at(&self, query: &TerrainQuery, x: f32, y: f32) -> bool {
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 {
            return false;
        }
        let c = ((x / CELL_SIZE).floor() as i32, (y / CELL_SIZE).floor() as i32);
        self.cell(query, c).is_some_and(|cell| cell.is_hole(x - c.0 as f32 * CELL_SIZE, y - c.1 as f32 * CELL_SIZE))
    }

    /// Height of the solid ground at a cry-space point: `None` over a hole or outside the map.
    pub fn ground_at(&self, query: &TerrainQuery, x: f32, y: f32) -> Option<f32> {
        if self.is_hole_at(query, x, y) {
            return None;
        }
        self.height_at(query, x, y)
    }

    /// Height at a cry-space point (bilinear, like the reader's `sample`), defined over holes too.
    pub fn height_at(&self, query: &TerrainQuery, x: f32, y: f32) -> Option<f32> {
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 {
            return None;
        }
        let c = ((x / CELL_SIZE).floor() as i32, (y / CELL_SIZE).floor() as i32);
        let cell = self.cell(query, c)?;
        Some(cell.sample(x - c.0 as f32 * CELL_SIZE, y - c.1 as f32 * CELL_SIZE))
    }

    /// First terrain hit of a cry-space ray (`dir` normalised), within `max` metres. The ray passes through holes.
    pub fn raycast(&self, query: &TerrainQuery, origin: Vec3, dir: Vec3, max: f32) -> Option<Vec3> {
        let above = |p: Vec3| self.height_at(query, p.x, p.y).map(|h| p.z - h);
        let mut prev_t = 0.0;
        if above(origin)? < 0.0 {
            return None;
        }
        // below the surface inside a hole is not a hit; the ray goes on until it is above ground again
        let mut under = false;
        let mut t = 0.0;
        let mut step = 1.0f32;
        while t < max {
            t += step;
            step = (step * 1.02).min(8.0);
            let p = origin + dir * t;
            let Some(d) = above(p) else { continue };
            if d < 0.0 && (under || self.is_hole_at(query, p.x, p.y)) {
                under = true;
                prev_t = t;
                continue;
            }
            under = false;
            if d < 0.0 {
                // bisect between prev_t and t
                let (mut a, mut b) = (prev_t, t);
                for _ in 0..16 {
                    let m = (a + b) * 0.5;
                    if above(origin + dir * m).is_some_and(|d| d < 0.0) { b = m } else { a = m }
                }
                let hit = origin + dir * b;
                if self.is_hole_at(query, hit.x, hit.y) {
                    // entered the ground at a hole's rim: keep marching inside the hole
                    under = true;
                    prev_t = t;
                    continue;
                }
                return Some(hit);
            }
            prev_t = t;
        }
        None
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
}

pub fn plugin(app: &mut App) {
    let num = |name: &str, default: f32| crate::arg(name).and_then(|v| v.parse().ok()).unwrap_or(default);
    let kind = match crate::arg("--brush").as_deref() {
        Some("lower") => BrushKind::Lower,
        Some("smooth") => BrushKind::Smooth,
        Some("flatten") => BrushKind::Flatten,
        Some("paint") => BrushKind::Paint,
        Some("hole") => BrushKind::Hole,
        Some("tint") => BrushKind::Tint,
        _ => BrushKind::Raise,
    };
    app.insert_resource(Brush {
        kind,
        radius: num("--brush-radius", 12.0),
        strength: num("--brush-strength", 4.0),
        flatten_to: None,
        paint: None,
        erase: false,
        palette: Vec::new(),
        palette_cell: None,
        tint: crate::arg("--tint-colour")
            .map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<_>>())
            .filter(|v| v.len() == 3)
            .map_or([0.55, 0.45, 0.3], |v| [v[0], v[1], v[2]]),
        last_ms: 0.0,
    })
    .init_resource::<Heights>()
    .add_plugins(crate::terrain_paint::plugin)
    .add_systems(
        Update,
        (scripted_edits, sculpt, refresh_new_patches, draw_brush, requests).chain().after(crate::tools::update_cursor),
    );
}

fn falloff(d: f32, radius: f32) -> f32 {
    let t = (1.0 - d / radius).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The rendered terrain the edits go to: patch meshes, the cells' materials and their surface maps.
#[derive(SystemParam)]
pub struct Targets<'w, 's> {
    meshes: ResMut<'w, Assets<Mesh>>,
    images: ResMut<'w, Assets<Image>>,
    materials: ResMut<'w, Assets<TerrainMaterial>>,
    patches: Query<'w, 's, (&'static mut TerrainPatch, &'static mut Mesh3d, &'static MeshMaterial3d<TerrainMaterial>)>,
}

/// One brush application.
struct Dab<'a> {
    kind: BrushKind,
    radius: f32,
    strength: f32,
    flatten_to: f32,
    dt: f32,
    /// Paint: the material; erase: the fallback surface's material.
    paint: Option<&'a str>,
    erase: bool,
}

/// A cell a dab changed.
struct Touched {
    cell: (i32, i32),
    heights: Arc<CellHeights>,
    /// Sample rectangle `(x0, y0, x1, y1)` whose meshes need rebuilding.
    rect: (usize, usize, usize, usize),
    mesh: bool,
    surface: bool,
}

/// Applies the brush at cry-space `hit` to every cell it covers. Changed cells go into `heights.edited` (and the
/// running stroke's undo copies).
fn dab(heights: &mut Heights, query: &TerrainQuery, tables: &mut SurfaceTables, world: &str, hit: Vec3, b: &Dab) -> Vec<Touched> {
    let r = b.radius;
    let (x0, x1) = (((hit.x - r) / CELL_SIZE).floor() as i32, ((hit.x + r) / CELL_SIZE).floor() as i32);
    let (y0, y1) = (((hit.y - r) / CELL_SIZE).floor() as i32, ((hit.y + r) / CELL_SIZE).floor() as i32);
    let mut touched = Vec::new();
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            let c = (cx, cy);
            let Some(source) = heights.cell(query, c) else { continue };
            let origin = Vec2::new(cx as f32 * CELL_SIZE, cy as f32 * CELL_SIZE);
            let local = Vec2::new(hit.x, hit.y) - origin;
            let lo = ((local - r) / UNIT_SIZE).floor().max(Vec2::ZERO);
            let hi = ((local + r) / UNIT_SIZE).ceil().min(Vec2::splat((GRID - 1) as f32));
            if lo.x > hi.x || lo.y > hi.y {
                continue;
            }
            let rect = (lo.x as usize, lo.y as usize, hi.x as usize, hi.y as usize);
            let edited = match b.kind {
                BrushKind::Paint => {
                    let Some(slot) = b.paint.and_then(|m| tables.slot_of(world, c, m)) else { continue };
                    paint(&source, local, r, rect, slot)
                }
                BrushKind::Hole => {
                    let fallback = b.paint.and_then(|m| tables.slot_of(world, c, m)).unwrap_or(0);
                    let original = query.cell(cx, cy);
                    holes(&source, original.as_deref(), local, r, rect, b.erase, fallback)
                }
                BrushKind::Tint => continue,
                _ => sculpt_cell(&source, local, rect, b),
            };
            let Some(cell) = edited else { continue };
            let before = heights.edited.get(&c).cloned();
            if let Some(stroke) = heights.stroke.as_mut() {
                stroke.cells.entry(c).or_insert(before);
            }
            let cell = Arc::new(cell);
            heights.edited.insert(c, cell.clone());
            heights.unsaved.insert(c);
            let (ix0, iy0, ix1, iy1) = rect;
            let rect = if b.kind == BrushKind::Hole {
                // a sector's hole gate covers all of it
                (ix0 / 32 * 32, iy0 / 32 * 32, (ix1 / 32 * 32 + 32).min(GRID - 1), (iy1 / 32 * 32 + 32).min(GRID - 1))
            } else {
                rect
            };
            touched.push(Touched {
                cell: c,
                heights: cell,
                rect,
                mesh: b.kind != BrushKind::Paint,
                surface: matches!(b.kind, BrushKind::Paint | BrushKind::Hole),
            });
        }
    }
    touched
}

/// Raise / lower / smooth / flatten; `None` when nothing changed.
fn sculpt_cell(source: &CellHeights, local: Vec2, (ix0, iy0, ix1, iy1): (usize, usize, usize, usize), b: &Dab) -> Option<CellHeights> {
    let at = |ix: usize, iy: usize| source.heights[iy * GRID + ix];
    let mut cell: Option<CellHeights> = None;
    for iy in iy0..=iy1 {
        for ix in ix0..=ix1 {
            let p = Vec2::new(ix as f32, iy as f32) * UNIT_SIZE;
            let w = falloff(p.distance(local), b.radius);
            if w <= 0.0 {
                continue;
            }
            let h = at(ix, iy);
            let new = match b.kind {
                BrushKind::Raise => h + b.strength * w * b.dt,
                BrushKind::Lower => h - b.strength * w * b.dt,
                BrushKind::Smooth => {
                    let (a, e) = (ix.saturating_sub(1), (ix + 1).min(GRID - 1));
                    let (c0, d) = (iy.saturating_sub(1), (iy + 1).min(GRID - 1));
                    let avg = (at(a, iy) + at(e, iy) + at(ix, c0) + at(ix, d)) * 0.25;
                    h + (avg - h) * (w * b.strength * 0.5 * b.dt).min(1.0)
                }
                BrushKind::Flatten => h + (b.flatten_to - h) * (w * b.strength * 0.5 * b.dt).min(1.0),
                BrushKind::Paint | BrushKind::Hole | BrushKind::Tint => h,
            };
            if new != h {
                cell.get_or_insert_with(|| source.clone()).heights[iy * GRID + ix] = new;
            }
        }
    }
    let mut cell = cell?;
    let (lo, hi) = cell.heights.iter().fold((f32::MAX, f32::MIN), |(a, b), &h| (a.min(h), b.max(h)));
    cell.min_z = lo;
    cell.max_z = hi;
    Some(cell)
}

/// Whether sample `(ix, iy)`'s 2 m square (a sample owns the square on its positive side) is under the brush.
fn covers(ix: usize, iy: usize, local: Vec2, r: f32) -> bool {
    (Vec2::new(ix as f32 + 0.5, iy as f32 + 0.5) * UNIT_SIZE).distance(local) <= r
}

/// Paints surface `slot` (holes stay holes); `None` when nothing changed.
fn paint(source: &CellHeights, local: Vec2, r: f32, (ix0, iy0, ix1, iy1): (usize, usize, usize, usize), slot: u8) -> Option<CellHeights> {
    let mut cell: Option<CellHeights> = None;
    for iy in iy0..=iy1 {
        for ix in ix0..=ix1 {
            let s = source.surface[iy * GRID + ix];
            if s != HOLE && s != slot && covers(ix, iy, local, r) {
                cell.get_or_insert_with(|| source.clone()).surface[iy * GRID + ix] = slot;
            }
        }
    }
    cell
}

/// Cuts holes (or fills them with `erase`) and keeps the sectors' hole gates consistent; `None` when nothing
/// changed. A filled sample gets its original surface back (else a neighbour's, else `fallback`).
fn holes(
    source: &CellHeights,
    original: Option<&CellHeights>,
    local: Vec2,
    r: f32,
    (ix0, iy0, ix1, iy1): (usize, usize, usize, usize),
    erase: bool,
    fallback: u8,
) -> Option<CellHeights> {
    // the last row and column are the far edge, owned by the next cell
    let (ix1, iy1) = (ix1.min(GRID - 2), iy1.min(GRID - 2));
    let mut cell: Option<CellHeights> = None;
    for iy in iy0..=iy1 {
        for ix in ix0..=ix1 {
            let i = iy * GRID + ix;
            if (source.surface[i] == HOLE) != erase || !covers(ix, iy, local, r) {
                continue;
            }
            let c = cell.get_or_insert_with(|| source.clone());
            c.surface[i] = if erase { unhole_surface(c, original, ix, iy, fallback) } else { HOLE };
        }
    }
    let mut cell = cell?;
    for sy in iy0 / 32..=iy1 / 32 {
        for sx in ix0 / 32..=ix1 / 32 {
            cell.hole_sectors[sy * 16 + sx] = if erase {
                // only a sector that keeps a hole keeps its gate
                cell.hole_sectors[sy * 16 + sx]
                    && (sy * 32..sy * 32 + 32).any(|y| (sx * 32..sx * 32 + 32).any(|x| cell.surface[y * GRID + x] == HOLE))
            } else {
                cell.hole_sectors[sy * 16 + sx] || (sy * 32..sy * 32 + 32).any(|y| (sx * 32..sx * 32 + 32).any(|x| cell.surface[y * GRID + x] == HOLE))
            };
        }
    }
    Some(cell)
}

fn sculpt(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut editor: ResMut<Editor>,
    cursor: Res<Cursor>,
    query: Res<TerrainQuery>,
    mut brush: ResMut<Brush>,
    mut heights: ResMut<Heights>,
    mut tables: ResMut<SurfaceTables>,
    mut targets: Targets,
    mut scripted: Local<u32>,
    mut worst: Local<Option<f32>>,
) {
    if editor.mode != Mode::Terrain || editor.playing {
        return;
    }
    // `--sculpt x,y`: hold the brush there for two seconds' worth of frames (unattended checks)
    let script = crate::arg("--sculpt").and_then(|v| {
        let v: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        (v.len() == 2 && *scripted < 120).then(|| Vec2::new(v[0], v[1]))
    });
    let script_hit = script.and_then(|p| heights.height_at(&query, p.x, p.y).map(|z| p.extend(z)));
    if script_hit.is_some() {
        *scripted += 1;
    } else if *scripted == 120 && worst.is_some() {
        info!("terrain: scripted stroke done, slowest frame {:.2} ms", worst.take().unwrap_or(0.0));
    }
    if script_hit.is_none() && *scripted == 120 && crate::flag("--sculpt-save") {
        *scripted += 1;
        editor.requests.push(Request::SaveTerrain);
    }
    if !mouse.pressed(MouseButton::Left) && script_hit.is_none() {
        if let Some(stroke) = heights.stroke.take().filter(|s| !s.is_empty()) {
            heights.undo.push(stroke);
        }
        brush.flatten_to = None;
        return;
    }
    let Some(hit) = script_hit.or(cursor.ground.filter(|_| !cursor.over_ui)) else { return };
    if heights.stroke.is_none() && !mouse.just_pressed(MouseButton::Left) && script_hit.is_none() {
        return; // a press that started on the panels
    }
    if heights.stroke.is_none() {
        heights.stroke = Some(Stroke::default());
        brush.flatten_to = Some(hit.z);
    }
    let started = std::time::Instant::now();
    if brush.kind == BrushKind::Tint {
        let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
        if alt {
            // eyedropper: the colour of the ground under the cursor
            if let Some(colour) = mouse.just_pressed(MouseButton::Left).then(|| cover_colour(&heights, &editor.world, hit)).flatten() {
                brush.tint = colour;
                editor.status(format!("tint: picked {:.2},{:.2},{:.2}", colour[0], colour[1], colour[2]));
            }
            return;
        }
        let dt = if script_hit.is_some() { 1.0 / 60.0 } else { time.delta_secs().min(0.1) };
        let amount = (brush.strength * 0.25 * dt).min(1.0);
        let touched = cover_dab(&mut heights, &editor.world, hit, brush.radius, crate::cover_paint::encode(brush.tint), amount);
        for (c, bytes, tiles) in &touched {
            apply_cover(*c, bytes, Some(tiles), &mut targets);
        }
        if !touched.is_empty() {
            brush.last_ms = started.elapsed().as_secs_f32() * 1000.0;
            if script_hit.is_some() {
                *worst = Some(worst.unwrap_or(0.0).max(brush.last_ms));
            }
        }
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let paint = brush.paint.clone();
    let b = Dab {
        kind: brush.kind,
        radius: brush.radius,
        strength: brush.strength,
        flatten_to: brush.flatten_to.unwrap_or(hit.z),
        dt: if script_hit.is_some() { 1.0 / 60.0 } else { time.delta_secs().min(0.1) },
        paint: paint.as_deref(),
        erase: brush.erase || shift,
    };
    let touched = dab(&mut heights, &query, &mut tables, &editor.world, hit, &b);
    for t in &touched {
        apply(t.cell, &t.heights, Some(t.rect), t.mesh, t.surface, &mut targets);
    }
    if !touched.is_empty() {
        brush.last_ms = started.elapsed().as_secs_f32() * 1000.0;
        if script_hit.is_some() {
            *worst = Some(worst.unwrap_or(0.0).max(brush.last_ms));
        }
    }
}

/// `--paint x,y --paint-surface <slot>` and `--hole x,y` (`--hole-erase` fills): one dab each at startup, in any
/// mode (also with `--play`), as one undoable stroke; `--edit-save` then saves the terrain.
fn scripted_edits(
    mut editor: ResMut<Editor>,
    query: Res<TerrainQuery>,
    mut brush: ResMut<Brush>,
    mut heights: ResMut<Heights>,
    mut tables: ResMut<SurfaceTables>,
    mut targets: Targets,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    let point = |name: &str| {
        let v: Vec<f32> = crate::arg(name)?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        (v.len() == 2).then(|| Vec2::new(v[0], v[1]))
    };
    let (paint_at, hole_at, tint_at) = (point("--paint"), point("--hole"), point("--tint"));
    if paint_at.is_none() && hole_at.is_none() && tint_at.is_none() {
        return;
    }
    heights.stroke = Some(Stroke::default());
    let mut touched = Vec::new();
    let world = editor.world.clone();
    if let Some(p) = paint_at {
        let c = ((p.x / CELL_SIZE).floor() as i32, (p.y / CELL_SIZE).floor() as i32);
        let slot: usize = crate::arg("--paint-surface").and_then(|v| v.parse().ok()).unwrap_or(0);
        let material = tables.get(&world, c).get(slot).cloned().flatten();
        match (material, heights.height_at(&query, p.x, p.y)) {
            (Some(m), Some(z)) => {
                brush.paint = Some(m.clone());
                let b = Dab { kind: BrushKind::Paint, radius: brush.radius, strength: 0.0, flatten_to: 0.0, dt: 0.0, paint: Some(&m), erase: false };
                touched.extend(dab(&mut heights, &query, &mut tables, &world, p.extend(z), &b));
                editor.status(format!("terrain: painted slot {slot} ({m}) at {},{}", p.x, p.y));
            }
            _ => editor.status(format!("terrain: cell {c:?} has no surface slot {slot}")),
        }
    }
    if let Some((p, z)) = hole_at.and_then(|p| Some((p, heights.height_at(&query, p.x, p.y)?))) {
        let erase = crate::flag("--hole-erase");
        let paint = brush.paint.clone();
        let b = Dab { kind: BrushKind::Hole, radius: brush.radius, strength: 0.0, flatten_to: 0.0, dt: 0.0, paint: paint.as_deref(), erase };
        touched.extend(dab(&mut heights, &query, &mut tables, &world, p.extend(z), &b));
        editor.status(format!("terrain: {} at {},{}", if erase { "holes filled" } else { "hole cut" }, p.x, p.y));
    }
    for t in &touched {
        apply(t.cell, &t.heights, None, t.mesh, t.surface, &mut targets);
    }
    if let Some((p, z)) = tint_at.and_then(|p| Some((p, heights.height_at(&query, p.x, p.y)?))) {
        let texel = crate::cover_paint::encode(brush.tint);
        for (c, bytes, tiles) in cover_dab(&mut heights, &world, p.extend(z), brush.radius, texel, 1.0) {
            apply_cover(c, &bytes, Some(&tiles), &mut targets);
        }
        editor.status(format!("terrain: tinted {:?} at {},{}", brush.tint, p.x, p.y));
    }
    if let Some(stroke) = heights.stroke.take().filter(|s| !s.is_empty()) {
        heights.undo.push(stroke);
    }
    if crate::flag("--edit-save") {
        editor.requests.push(Request::SaveTerrain);
    }
}

/// Gives the loaded patches of cell `c` the heights `cell`, rebuilds (`mesh`) the meshes of those overlapping the
/// sample rectangle `rect` (all when `None`) and rewrites (`surface`) the cell's surface map.
fn apply(c: (i32, i32), cell: &Arc<CellHeights>, rect: Option<(usize, usize, usize, usize)>, mesh: bool, surface: bool, t: &mut Targets) {
    let mut material = None;
    for (mut patch, mut patch_mesh, patch_material) in t.patches.iter_mut().filter(|(p, _, _)| p.cell == c) {
        let (px, py) = (patch.index % PATCHES, patch.index / PATCHES);
        let overlaps = rect.is_none_or(|(x0, y0, x1, y1)| {
            let (a, b) = (px * PATCH_UNITS, (px + 1) * PATCH_UNITS);
            let (d, e) = (py * PATCH_UNITS, (py + 1) * PATCH_UNITS);
            x1 >= a && x0 <= b && y1 >= d && y0 <= e
        });
        patch.set_heights(cell.clone());
        if mesh && overlaps {
            patch_mesh.0 = t.meshes.add(build_patch_mesh(cell, px, py, patch.lod));
        }
        material.get_or_insert_with(|| patch_material.0.clone());
    }
    if let (true, Some(material)) = (surface, material) {
        update_surface_map(cell, &material, &mut t.materials, &mut t.images);
    }
}

/// Patches streamed in again after an unload come with the pak's heights, surfaces and cover: give them the edits.
fn refresh_new_patches(heights: Res<Heights>, mut queries: ParamSet<(Query<&TerrainPatch, Added<TerrainPatch>>, Targets)>) {
    let added: HashSet<(i32, i32)> = queries.p0().iter().map(|p| p.cell).collect();
    let cells: HashSet<(i32, i32)> = queries
        .p0()
        .iter()
        .filter(|p| heights.edited.get(&p.cell).is_some_and(|h| !Arc::ptr_eq(h, p.heights())))
        .map(|p| p.cell)
        .collect();
    for c in cells {
        apply(c, &heights.edited[&c], None, true, true, &mut queries.p1());
    }
    for c in added {
        if let Some(bytes) = heights.covers.get(&c) {
            apply_cover(c, bytes, None, &mut queries.p1());
        }
    }
}

fn cover_path(world: &str, c: (i32, i32)) -> String {
    format!("{}/client/terrain/cover.ctc", aa_world::world::cell_dir(world, c.0, c.1))
}

/// The cover file the editor works with: the edited one, else the pak's.
fn cover_bytes(heights: &Heights, world: &str, c: (i32, i32)) -> Option<Arc<Vec<u8>>> {
    heights.covers.get(&c).cloned().or_else(|| aa_pak::Pak::global().read(&cover_path(world, c)).ok().map(Arc::new))
}

/// The cover colour at a cry-space point.
fn cover_colour(heights: &Heights, world: &str, hit: Vec3) -> Option<[f32; 3]> {
    let c = ((hit.x / CELL_SIZE).floor() as i32, (hit.y / CELL_SIZE).floor() as i32);
    let local = Vec2::new(hit.x - c.0 as f32 * CELL_SIZE, hit.y - c.1 as f32 * CELL_SIZE);
    crate::cover_paint::colour_at(&cover_bytes(heights, world, c)?, local)
}

/// Tints the cover of every cell under the brush; returns the changed files and their changed tiles.
fn cover_dab(heights: &mut Heights, world: &str, hit: Vec3, r: f32, texel: [f32; 3], amount: f32) -> Vec<((i32, i32), Arc<Vec<u8>>, Vec<TileKey>)> {
    let (x0, x1) = (((hit.x - r) / CELL_SIZE).floor() as i32, ((hit.x + r) / CELL_SIZE).floor() as i32);
    let (y0, y1) = (((hit.y - r) / CELL_SIZE).floor() as i32, ((hit.y + r) / CELL_SIZE).floor() as i32);
    let mut out = Vec::new();
    for cy in y0.max(0)..=y1 {
        for cx in x0.max(0)..=x1 {
            let c = (cx, cy);
            let Some(source) = cover_bytes(heights, world, c) else { continue };
            let local = Vec2::new(hit.x - cx as f32 * CELL_SIZE, hit.y - cy as f32 * CELL_SIZE);
            let Some((bytes, tiles)) = crate::cover_paint::tint(&source, local, r, texel, amount) else { continue };
            let before = heights.covers.get(&c).cloned();
            if let Some(stroke) = heights.stroke.as_mut() {
                stroke.covers.entry(c).or_insert(before);
            }
            let bytes = Arc::new(bytes);
            heights.covers.insert(c, bytes.clone());
            heights.covers_unsaved.insert(c);
            out.push((c, bytes, tiles));
        }
    }
    out
}

/// Copies cover tiles (all when `None`) into the loaded cell's cover image.
fn apply_cover(c: (i32, i32), bytes: &[u8], tiles: Option<&[TileKey]>, t: &mut Targets) {
    let Some(material) = t.patches.iter().find(|(p, _, _)| p.cell == c).map(|(_, _, m)| m.0.clone()) else { return };
    let Some(cover) = t.materials.get(&material).and_then(|m| m.base.base_color_texture.clone()) else { return };
    let uploaded = t.images.get_mut(&cover).is_some_and(|mut image| crate::cover_paint::upload(bytes, tiles, &mut image));
    if uploaded {
        // the bind group holds the old texture view: rebuild it
        if let Some(m) = t.materials.get_mut(&material) {
            m.into_inner();
        }
    }
}

fn draw_brush(editor: Res<Editor>, cursor: Res<Cursor>, brush: Res<Brush>, heights: Res<Heights>, query: Res<TerrainQuery>, mut gizmos: Gizmos) {
    if editor.mode != Mode::Terrain || editor.playing {
        return;
    }
    let Some(hit) = cursor.ground else { return };
    let color = match brush.kind {
        BrushKind::Raise => Color::srgb(0.3, 1.0, 0.4),
        BrushKind::Lower => Color::srgb(1.0, 0.4, 0.3),
        BrushKind::Smooth => Color::srgb(0.4, 0.7, 1.0),
        BrushKind::Flatten => Color::srgb(1.0, 0.9, 0.3),
        BrushKind::Paint => Color::srgb(1.0, 0.4, 1.0),
        BrushKind::Hole if brush.erase => Color::srgb(0.3, 1.0, 1.0),
        BrushKind::Hole => Color::srgb(0.1, 0.1, 0.1),
        BrushKind::Tint => Color::srgb(brush.tint[0], brush.tint[1], brush.tint[2]),
    };
    for (radius, alpha) in [(brush.radius, 1.0), (brush.radius * 0.5, 0.45)] {
        let n = 64;
        let pts: Vec<Vec3> = (0..=n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                let (x, y) = (hit.x + a.cos() * radius, hit.y + a.sin() * radius);
                let z = heights.height_at(&query, x, y).unwrap_or(hit.z) + 0.25;
                cry_to_bevy(Vec3::new(x, y, z))
            })
            .collect();
        gizmos.linestrip(pts, color.with_alpha(alpha));
    }
}

fn requests(
    mut editor: ResMut<Editor>,
    mut heights: ResMut<Heights>,
    query: Res<TerrainQuery>,
    mut targets: Targets,
) {
    if let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::UndoTerrain)) {
        editor.requests.remove(i);
        if let Some(stroke) = heights.undo.pop() {
            for (c, before) in stroke.covers {
                heights.covers.remove(&c);
                // back to the file the editor started with (kept as an edit, so a save writes it)
                let Some(bytes) = before.or_else(|| cover_bytes(&heights, &editor.world, c)) else { continue };
                heights.covers.insert(c, bytes.clone());
                heights.covers_unsaved.insert(c);
                apply_cover(c, &bytes, None, &mut targets);
            }
            for (c, before) in stroke.cells {
                let cell = match before {
                    Some(b) => {
                        heights.edited.insert(c, b.clone());
                        b
                    }
                    // back to the heights the editor started with (kept as an edit, so a save writes them)
                    None => match query.cell(c.0, c.1) {
                        Some(orig) => {
                            heights.edited.insert(c, orig.clone());
                            orig
                        }
                        None => continue,
                    },
                };
                heights.unsaved.insert(c);
                apply(c, &cell, None, true, true, &mut targets);
            }
            editor.status("terrain: stroke undone");
        }
    }
    if let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::SaveTerrain)) {
        editor.requests.remove(i);
        let result = save(&editor, &mut heights);
        match result {
            Ok(0) => editor.status("terrain: nothing to save"),
            Ok(n) => {
                let msg = format!("terrain: {n} cell(s) saved to {}", editor.paths.overlay.display());
                editor.status(msg)
            }
            Err(e) => editor.status(format!("terrain save failed: {e:#}")),
        }
    }
}

fn save(editor: &Editor, heights: &mut Heights) -> anyhow::Result<usize> {
    let mut saved = 0;
    let mut written = HashSet::new();
    let cells: Vec<(i32, i32)> = heights.unsaved.iter().copied().collect();
    for c in cells {
        let rel = format!("{}/client/terrain/heightmap.dat", aa_world::world::cell_dir(&editor.world, c.0, c.1));
        // the template: the overlay's earlier save or the original
        let original = aa_pak::Pak::global().read(&rel)?;
        let cell = match heights.edited.get(&c) {
            Some(cell) => cell.clone(),
            None => continue,
        };
        let bytes = crate::heightmap_write::write(&original, &cell)?;
        let path = editor.paths.overlay.join(&rel);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, bytes)?;
        heights.unsaved.remove(&c);
        written.insert(c);
        saved += 1;
    }
    // edited covers: the same file layout with re-encoded tiles, written as they are
    let covers: Vec<(i32, i32)> = heights.covers_unsaved.iter().copied().collect();
    for c in covers {
        let Some(bytes) = heights.covers.get(&c).cloned() else { continue };
        let path = editor.paths.overlay.join(cover_path(&editor.world, c));
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, bytes.as_slice())?;
        heights.covers_unsaved.remove(&c);
        if !written.contains(&c) {
            saved += 1;
        }
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat() -> CellHeights {
        CellHeights {
            heights: vec![100.0; GRID * GRID],
            surface: vec![3; GRID * GRID],
            hole_sectors: [false; 256],
            ocean_level: 0.0,
            min_z: 100.0,
            max_z: 100.0,
            sectors: 256,
        }
    }

    #[test]
    fn holes_keep_the_sector_gates_and_fill_back() {
        let cell = flat();
        let at = Vec2::new(70.0, 70.0); // sample (35, 35), sector (1, 1)
        let cut = holes(&cell, Some(&cell), at, 3.0, (32, 32, 38, 38), false, 0).unwrap();
        assert!(cut.hole_sectors[17]);
        assert_eq!(cut.hole_sectors.iter().filter(|g| **g).count(), 1);
        assert!(cut.is_hole(70.5, 70.5) && cut.quad_is_hole(35, 35, 1));
        assert!(!cut.is_hole(80.0, 70.0));
        // nothing new to cut: no change
        assert!(holes(&cut, Some(&cell), at, 3.0, (32, 32, 38, 38), false, 0).is_none());
        let filled = holes(&cut, Some(&cell), at, 3.0, (32, 32, 38, 38), true, 0).unwrap();
        assert_eq!(filled.surface, cell.surface);
        assert!(!filled.hole_sectors[17]);
        // painting leaves holes alone
        let painted = paint(&cut, at, 6.0, (30, 30, 40, 40), 5).unwrap();
        assert_eq!(painted.surface[35 * GRID + 35], HOLE);
        assert_eq!(painted.surface[32 * GRID + 35], 5);
    }
}
