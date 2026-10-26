//! Shaping decals, roads and water in the view ([`super`]).
//!
//! The selected item shows handles: the white one moves the whole item (decals and roads follow the ground, water
//! keeps its surface height), yellow ones move a road's spline node (both edge points, so the width stays), small ones
//! a single road edge point or water outline point (the physics outline point with it when the two outlines match).
//! The box is refitted to the points after each change.
//!
//! Water is drawn again at once: the cell's edited `object.dat` is parsed and meshed the way the streamer does it, and
//! the streamer's own water of that cell is hidden until "Reload objects". Roads and decals are built by `aa-world`
//! code the editor cannot call, so they redraw after "Save and redraw" (or Ctrl+S and "Reload objects").

use aa_world::placement::parse_object_dat;
use aa_world::water::{WaterSurface, water_meshes};
use aa_world::water_material::{WaterMaterial, WaterStyles};
use aa_world::TerrainQuery;
use bevy::light::NotShadowCaster;
use bevy::window::PrimaryWindow;

use super::*;
use crate::camera::EditorCamera;
use crate::terrain_edit::Heights;

/// A handle of the selected item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Whole,
    /// Road spline node `k`: points `2k` and `2k + 1`.
    Pair(usize),
    /// One stored point (road edge point / water render outline point).
    Point(usize),
}

pub struct GripDrag {
    cell: (i32, i32),
    item: usize,
    kind: ExtraKind,
    grip: Grip,
    /// Cry-space position of the grip and the ground under the cursor when the drag began.
    at: Vec3,
    start: Vec3,
    before: Vec<u8>,
    press: Vec2,
    started: bool,
}

/// One undoable change of an octree item: its bytes and deleted flag before.
pub struct Step {
    pub cell: (i32, i32),
    pub item: usize,
    pub bytes: Vec<u8>,
    pub deleted: bool,
}

/// The editor's own water meshes of a cell.
#[derive(Component)]
pub struct LiveWater;

const ROAD_COUNT: usize = 39;
const ROAD_POINTS: usize = 63;
const WATER_COUNT: usize = 103;
const WATER_PHYSICS: usize = 115;
const WATER_POINTS: usize = 119;

fn point(b: &[u8], o: usize) -> Vec3 {
    Vec3::new(f32_at(b, o), f32_at(b, o + 4), f32_at(b, o + 8))
}

fn shift(b: &mut [u8], o: usize, d: Vec3) {
    for k in 0..3 {
        put_f32(b, o + k * 4, f32_at(b, o + k * 4) + d[k]);
    }
}

/// The grips of an item (cell-local positions).
pub fn grips(kind: ExtraKind, b: &[u8]) -> Vec<(Grip, Vec3)> {
    let (lo, hi) = bbox(b);
    let mut out = Vec::new();
    match kind {
        ExtraKind::Decal if b.len() >= 55 => out.push((Grip::Whole, point(b, 43))),
        ExtraKind::Road if b.len() >= ROAD_POINTS => {
            let n = u32_at(b, ROAD_COUNT) as usize;
            if b.len() < ROAD_POINTS + n * 12 {
                return out;
            }
            let pts: Vec<Vec3> = (0..n).map(|i| point(b, ROAD_POINTS + i * 12)).collect();
            let mid = pts.iter().copied().sum::<Vec3>() / n.max(1) as f32;
            out.push((Grip::Whole, mid));
            for k in 0..n / 2 {
                out.push((Grip::Pair(k), (pts[2 * k] + pts[2 * k + 1]) / 2.0));
            }
            out.extend(pts.into_iter().enumerate().map(|(i, p)| (Grip::Point(i), p)));
        }
        ExtraKind::Water if b.len() >= WATER_POINTS => {
            let n = u32_at(b, WATER_COUNT) as usize;
            if b.len() < WATER_POINTS + n * 12 {
                return out;
            }
            let surface = if f32_at(b, 79) > 0.99 { -f32_at(b, 83) } else { hi.z };
            let c = (lo + hi) / 2.0;
            out.push((Grip::Whole, Vec3::new(c.x, c.y, surface)));
            out.extend((0..n).map(|i| (Grip::Point(i), point(b, WATER_POINTS + i * 12))));
        }
        _ => {}
    }
    out
}

/// The box around the item's points (decals keep theirs).
fn refit(kind: ExtraKind, b: &mut [u8]) {
    let pts: Vec<Vec3> = match kind {
        ExtraKind::Road => (0..u32_at(b, ROAD_COUNT) as usize).map(|i| point(b, ROAD_POINTS + i * 12)).collect(),
        ExtraKind::Water => (0..u32_at(b, WATER_COUNT) as usize).map(|i| point(b, WATER_POINTS + i * 12)).collect(),
        _ => return,
    };
    if pts.is_empty() {
        return;
    }
    let (lo, hi) = bbox(b);
    let (mut a, mut c) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &pts {
        a = a.min(*p);
        c = c.max(*p);
    }
    let margin = Vec3::new(1.0, 1.0, 1.0);
    let lo = Vec3::new(a.x - margin.x, a.y - margin.y, lo.z.min(a.z - margin.z));
    let hi = Vec3::new(c.x + margin.x, c.y + margin.y, hi.z.max(c.z + margin.z));
    for k in 0..3 {
        put_f32(b, k * 4, lo[k]);
        put_f32(b, 12 + k * 4, hi[k]);
    }
}

/// Moves a grip by `d` (cell-local).
pub fn move_grip(kind: ExtraKind, b: &mut [u8], grip: Grip, d: Vec3) {
    match (kind, grip) {
        (_, Grip::Whole) => translate_item(kind.item(), b, d),
        (ExtraKind::Road, Grip::Pair(k)) => {
            shift(b, ROAD_POINTS + 2 * k * 12, d);
            shift(b, ROAD_POINTS + (2 * k + 1) * 12, d);
            refit(kind, b);
        }
        (ExtraKind::Road, Grip::Point(i)) => {
            shift(b, ROAD_POINTS + i * 12, d);
            refit(kind, b);
        }
        (ExtraKind::Water, Grip::Point(i)) => {
            let n = u32_at(b, WATER_COUNT) as usize;
            let d = Vec3::new(d.x, d.y, 0.0);
            shift(b, WATER_POINTS + i * 12, d);
            // the physics outline follows when it is the same outline
            if u32_at(b, WATER_PHYSICS) as usize == n && b.len() >= WATER_POINTS + (n + i + 1) * 12 {
                shift(b, WATER_POINTS + (n + i) * 12, d);
            }
            refit(kind, b);
        }
        _ => {}
    }
}

fn ground(heights: &Heights, query: &TerrainQuery, p: Vec3) -> f32 {
    heights.height_at(query, p.x, p.y).unwrap_or(p.z)
}

/// The selected octree item: (cell, kind, item index).
fn selected(extra: &Extra) -> Option<((i32, i32), ExtraKind, usize)> {
    let c = extra.cell?;
    let (kind, i) = extra.selected?;
    (kind != ExtraKind::Interior).then_some((c, kind, i))
}

/// Hover and drag of the grips. Runs before the Objects tools, which then see the pointer as taken.
#[allow(clippy::too_many_arguments)]
pub fn grip_input(
    mouse: Res<ButtonInput<MouseButton>>,
    mut cursor: ResMut<Cursor>,
    window: Query<&Window, With<PrimaryWindow>>,
    cam: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
    editor: Res<Editor>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    mut objs: ResMut<WorldObjects>,
    mut extra: ResMut<Extra>,
) {
    extra.grip_hover = None;
    let active = editor.mode == Mode::Objects && extra.open && !editor.playing;
    let Some((c, kind, i)) = selected(&extra).filter(|_| active) else {
        extra.grip_drag = None;
        return;
    };
    let (Ok(window), Ok((camera, ct))) = (window.single(), cam.single()) else { return };
    let Some(m) = window.cursor_position() else { return };
    let Some(cell) = objs.cells.get_mut(&c) else { return };
    let Some(item) = cell.octree.as_mut().and_then(|t| t.items.get_mut(i)).filter(|it| !it.deleted) else { return };
    let o = cell_origin(c.0, c.1);

    if extra.grip_drag.is_none() && !cursor.over_ui {
        let mut best: Option<(f32, Grip, Vec3)> = None;
        for (g, p) in grips(kind, &item.bytes) {
            let Ok(s) = camera.world_to_viewport(ct, cry_to_bevy(o + p)) else { continue };
            // the whole-item grip loses ties to the points on it
            let d = s.distance(m) + if g == Grip::Whole { 2.0 } else { 0.0 };
            if d < 10.0 && best.is_none_or(|b| d < b.0) {
                best = Some((d, g, p));
            }
        }
        extra.grip_hover = best.map(|b| b.1);
        if let Some((_, g, p)) = best
            && mouse.just_pressed(MouseButton::Left)
            && let Some(start) = cursor.ground
        {
            extra.grip_drag = Some(GripDrag { cell: c, item: i, kind, grip: g, at: o + p, start, before: item.bytes.clone(), press: m, started: false });
        }
    }
    if extra.grip_hover.is_some() || extra.grip_drag.is_some() {
        // the Objects tools leave the pointer to us
        cursor.over_ui = true;
    }
    let Some(drag) = extra.grip_drag.as_mut() else { return };
    if !mouse.pressed(MouseButton::Left) {
        if drag.started {
            let step = Step { cell: drag.cell, item: drag.item, bytes: std::mem::take(&mut drag.before), deleted: false };
            let kind = drag.kind;
            extra.undo.push(step);
            if kind == ExtraKind::Water {
                extra.water_dirty.insert(c);
            }
            extra.status = "moved (Undo in this window; Ctrl+S saves)".into();
            extra.changed(Some(kind));
        }
        extra.grip_drag = None;
        return;
    }
    let Some(g) = cursor.ground else { return };
    if !drag.started && drag.press.distance(m) < 3.0 {
        return;
    }
    drag.started = true;
    let mut d = g - drag.start;
    let to = drag.at + Vec3::new(d.x, d.y, 0.0);
    d.z = match drag.kind {
        ExtraKind::Water => 0.0,
        _ => ground(&heights, &query, to) - ground(&heights, &query, drag.at),
    };
    item.bytes.clone_from(&drag.before);
    move_grip(drag.kind, &mut item.bytes, drag.grip, d);
    cell.unsaved.insert(File::Object);
    cell.edited = true;
}

pub fn draw_grips(editor: Res<Editor>, objs: Res<WorldObjects>, extra: Res<Extra>, mut gizmos: Gizmos) {
    if editor.mode != Mode::Objects || !extra.open {
        return;
    }
    let Some((c, kind, i)) = selected(&extra) else { return };
    let Some(item) = objs.cells.get(&c).and_then(|cell| cell.octree.as_ref()).and_then(|t| t.items.get(i)).filter(|it| !it.deleted) else { return };
    let o = cell_origin(c.0, c.1);
    let hot = extra.grip_drag.as_ref().map(|d| d.grip).or(extra.grip_hover);
    for (g, p) in grips(kind, &item.bytes) {
        let (r, colour) = match g {
            Grip::Whole => (0.9, Color::WHITE),
            Grip::Pair(_) => (0.55, Color::srgb(1.0, 0.85, 0.2)),
            Grip::Point(_) => (0.35, Color::srgb(0.4, 0.95, 1.0)),
        };
        let colour = if hot == Some(g) { Color::srgb(1.0, 0.45, 0.1) } else { colour };
        gizmos.sphere(Isometry3d::from_translation(cry_to_bevy(o + p + Vec3::Z * 0.2)), if hot == Some(g) { r * 1.5 } else { r }, colour);
    }
}

/// Draws the edited water of the cells in [`Extra::water_dirty`] and keeps the streamer's water of those cells hidden.
#[allow(clippy::too_many_arguments)]
pub fn live_water(
    mut commands: Commands,
    mut extra: ResMut<Extra>,
    objs: Res<WorldObjects>,
    mut styles: ResMut<WaterStyles>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut originals: Query<(&WaterSurface, &mut Visibility), Without<LiveWater>>,
) {
    let dirty: Vec<(i32, i32)> = extra.water_dirty.drain().collect();
    for c in dirty {
        let Some(tree) = objs.cells.get(&c).and_then(|cell| cell.octree.as_ref()) else { continue };
        let objects = match parse_object_dat(&tree.write()) {
            Ok(o) => o,
            Err(e) => {
                warn!("live water: cell {c:?}: {e}");
                continue;
            }
        };
        for e in extra.live_water.remove(&c).unwrap_or_default() {
            commands.entity(e).despawn();
        }
        let mut entities = Vec::new();
        for (style, mesh) in water_meshes(&objects.water, &objects.materials) {
            let material = styles.material(&style, &mut water_materials, &mut images);
            entities.push(
                commands
                    .spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material),
                        WaterSurface { cell: c },
                        LiveWater,
                        Transform::from_translation(cry_to_bevy(cell_origin(c.0, c.1))),
                        NotShadowCaster,
                    ))
                    .id(),
            );
        }
        // an empty list still marks the cell: its streamer water stays hidden (all of it was deleted)
        extra.live_water.insert(c, entities);
    }
    if extra.live_water.is_empty() {
        return;
    }
    for (s, mut vis) in &mut originals {
        if extra.live_water.contains_key(&s.cell) && *vis != Visibility::Hidden {
            *vis = Visibility::Hidden;
        }
    }
}

/// Undoes the last change of an octree item.
pub fn undo(extra: &mut Extra, objs: &mut WorldObjects) -> bool {
    let Some(step) = extra.undo.pop() else { return false };
    let Some(cell) = objs.cells.get_mut(&step.cell) else { return false };
    let Some(item) = cell.octree.as_mut().and_then(|t| t.items.get_mut(step.item)) else { return false };
    let water = item.kind == ExtraKind::Water.item();
    item.bytes = step.bytes;
    item.deleted = step.deleted;
    cell.unsaved.insert(File::Object);
    cell.edited = true;
    if water {
        extra.water_dirty.insert(step.cell);
    }
    true
}

/// Remembers an octree item before a change made from the window.
pub fn remember(extra: &mut Extra, cell: &EditCell, item: usize) {
    if let Some(it) = cell.octree.as_ref().and_then(|t| t.items.get(item)) {
        extra.undo.push(Step { cell: cell.cell, item, bytes: it.bytes.clone(), deleted: it.deleted });
    }
}

/// A copy of an octree item `offset` metres away (cell-local); returns its index.
pub fn duplicate(extra: &mut Extra, cell: &mut EditCell, kind: ExtraKind, item: usize, offset: Vec3) -> Option<usize> {
    let tree = cell.octree.as_mut()?;
    let src = tree.items.get(item).filter(|it| !it.deleted)?;
    let mut bytes = src.bytes.clone();
    translate_item(src.kind, &mut bytes, offset);
    let j = tree.push(src.kind, bytes);
    // undoing the copy deletes it
    let copy = tree.items[j].bytes.clone();
    extra.undo.push(Step { cell: cell.cell, item: j, bytes: copy, deleted: true });
    cell.unsaved.insert(File::Object);
    cell.edited = true;
    if kind == ExtraKind::Water {
        extra.water_dirty.insert(cell.cell);
    }
    Some(j)
}

/// Drops the editor's water meshes (the streamer's come back with the reload).
pub fn drop_live_water(world: &mut World) {
    let entities: Vec<Entity> = {
        let Some(mut extra) = world.get_resource_mut::<Extra>() else { return };
        extra.live_water.drain().flat_map(|(_, v)| v).collect()
    };
    for e in entities {
        if world.get_entity(e).is_ok() {
            world.despawn(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn road(points: &[Vec3]) -> Vec<u8> {
        let mut b = vec![0u8; ROAD_POINTS + points.len() * 12];
        b[ROAD_COUNT..ROAD_COUNT + 4].copy_from_slice(&(points.len() as u32).to_le_bytes());
        for (i, p) in points.iter().enumerate() {
            for k in 0..3 {
                put_f32(&mut b, ROAD_POINTS + i * 12 + k * 4, p[k]);
            }
        }
        refit(ExtraKind::Road, &mut b);
        b
    }

    #[test]
    fn road_grips_move_nodes_and_points() {
        let pts = [Vec3::new(0.0, 0.0, 10.0), Vec3::new(0.0, 4.0, 10.0), Vec3::new(20.0, 0.0, 10.0), Vec3::new(20.0, 4.0, 10.0)];
        let mut b = road(&pts);
        let g = grips(ExtraKind::Road, &b);
        assert_eq!(g.iter().filter(|g| matches!(g.0, Grip::Pair(_))).count(), 2);
        assert_eq!(g.iter().filter(|g| matches!(g.0, Grip::Point(_))).count(), 4);
        // node 1 moves 5 m north: both its edge points, the width stays, the box grows
        move_grip(ExtraKind::Road, &mut b, Grip::Pair(1), Vec3::new(0.0, 5.0, 0.0));
        assert_eq!(point(&b, ROAD_POINTS + 2 * 12), Vec3::new(20.0, 5.0, 10.0));
        assert_eq!(point(&b, ROAD_POINTS + 3 * 12), Vec3::new(20.0, 9.0, 10.0));
        assert_eq!(point(&b, ROAD_POINTS), pts[0]);
        assert_eq!(bbox(&b).1.y, 10.0);
        // one edge point alone
        move_grip(ExtraKind::Road, &mut b, Grip::Point(0), Vec3::new(-1.0, 0.0, 0.0));
        assert_eq!(point(&b, ROAD_POINTS), Vec3::new(-1.0, 0.0, 10.0));
        assert_eq!(bbox(&b).0.x, -2.0);
    }

    #[test]
    fn water_points_keep_the_surface_and_drag_the_physics_outline() {
        let n = 3;
        let mut b = vec![0u8; WATER_POINTS + 2 * n * 12];
        put_f32(&mut b, 79, 1.0);
        put_f32(&mut b, 83, -50.0);
        b[WATER_COUNT..WATER_COUNT + 4].copy_from_slice(&(n as u32).to_le_bytes());
        b[WATER_PHYSICS..WATER_PHYSICS + 4].copy_from_slice(&(n as u32).to_le_bytes());
        let pts = [Vec3::new(0.0, 0.0, 50.0), Vec3::new(10.0, 0.0, 50.0), Vec3::new(0.0, 10.0, 50.0)];
        for (i, p) in pts.iter().chain(pts.iter()).enumerate() {
            for k in 0..3 {
                put_f32(&mut b, WATER_POINTS + i * 12 + k * 4, p[k]);
            }
        }
        let g = grips(ExtraKind::Water, &b);
        assert_eq!(g[0], (Grip::Whole, Vec3::new(0.0, 0.0, 50.0)), "the box is empty before a refit; the grip sits on the surface");
        move_grip(ExtraKind::Water, &mut b, Grip::Point(1), Vec3::new(5.0, 0.0, 3.0));
        assert_eq!(point(&b, WATER_POINTS + 12), Vec3::new(15.0, 0.0, 50.0), "z stays");
        assert_eq!(point(&b, WATER_POINTS + (n + 1) * 12), Vec3::new(15.0, 0.0, 50.0), "physics outline follows");
        assert_eq!(bbox(&b).1.x, 16.0);
    }
}
