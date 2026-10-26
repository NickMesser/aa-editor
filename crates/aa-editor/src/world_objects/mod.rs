//! Static world objects of the cell files (Objects mode, key 4): brushes (`brush.dat` / `object.dat`), vegetation
//! (`vegetation.dat`, the octree, `vegetation/SX_SY.dat`) and static `entities.xml` slots.
//!
//! * pick: a ray against the drawn parts' oriented boxes ([`ObjectPart`] on every part says whose it is);
//! * edit: drag over the ground, `[` `]` / Shift + wheel turn, PageUp / PageDown raise / lower, `+` / `-` scale,
//!   Del, Ctrl+D, Ctrl+Z / Ctrl+Y; the panel "World object" edits the numbers;
//! * display: an edit moves the drawn parts at once ([`live_parts`]); when no drag or stroke runs the cell's edited
//!   copy goes to the streamer ([`ObjectState::replace_cell_objects`]), which rebuilds the touched sectors;
//! * save (Ctrl+S, [`Request::SaveObjects`]): the edited files into the pak overlay, in their original formats;
//! * conform: after a terrain sculpt the objects standing on the edited ground follow it ([`conform`]).

pub mod cell;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aa_world::coords::{CRY_TO_BEVY, cell_origin};
use aa_world::heightmap::CellHeights;
use aa_world::objects::edit::{ObjectKind, ObjectPart};
use aa_world::objects::{ObjectSector, ObjectState};
use aa_world::placement_write::entities_xml;
use aa_world::{TerrainQuery, cry_to_bevy};
use bevy::camera::primitives::Aabb;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use self::cell::{Change, EditCell, Item, State, transformed};
use crate::terrain_edit::Heights;
use crate::tools::Cursor;
use crate::{Editor, Mode, Request};

/// The parts' transform when they were spawned, kept while [`live_parts`] moves them.
#[derive(Component)]
struct LiveBase(Transform);

#[derive(Default)]
struct Drag {
    /// Selection position (world, cry) minus the grabbed ground point.
    offset: Option<Vec3>,
    above: f32,
    press: Option<Vec2>,
    started: bool,
    before: Option<State>,
}

#[derive(Resource)]
pub struct WorldObjects {
    pub cells: HashMap<(i32, i32), EditCell>,
    pub selected: Option<ObjectPart>,
    pub hover: Option<ObjectPart>,
    undo: Vec<Vec<((i32, i32), Change)>>,
    redo: Vec<Vec<((i32, i32), Change)>>,
    /// Objects whose drawn parts are moved by hand until the next commit.
    live: HashSet<ObjectPart>,
    drag: Drag,
    pub auto_conform: bool,
    /// Per terrain cell: the heights the objects stand on now.
    seen: HashMap<(i32, i32), Arc<CellHeights>>,
    /// A conform ran this frame (a stroke is going on: no commit yet).
    conforming: bool,
    /// The streamer's data of each edited cell after our last commit (address), to notice a reload.
    installed: HashMap<(i32, i32), usize>,
    /// Object-space box of the selection.
    bounds: Option<(ObjectPart, Vec3, Vec3)>,
    /// The last undo entry came from the panel for this object (panel drags merge into one entry).
    panel_merge: Option<ObjectPart>,
    /// The panel asked for a conform run.
    conform_now: bool,
    /// Objects moved by conform runs since the last commit (for the log).
    conformed: usize,
}

impl Default for WorldObjects {
    fn default() -> Self {
        WorldObjects {
            cells: HashMap::new(),
            selected: None,
            hover: None,
            undo: Vec::new(),
            redo: Vec::new(),
            live: HashSet::new(),
            drag: Drag::default(),
            auto_conform: !crate::flag("--no-conform"),
            seen: HashMap::new(),
            conforming: false,
            installed: HashMap::new(),
            bounds: None,
            panel_merge: None,
            conform_now: false,
            conformed: 0,
        }
    }
}

impl WorldObjects {
    /// The cell's edit state, read from its files on first use (needs the streamer's data of the cell).
    fn ensure(&mut self, world: &str, state: &ObjectState, c: (i32, i32)) -> Option<&mut EditCell> {
        if !self.cells.contains_key(&c) {
            let data = state.cell_data(c)?;
            let cell = EditCell::load(world, c, &data.objects);
            self.installed.insert(c, Arc::as_ptr(data) as usize);
            self.cells.insert(c, cell);
        }
        self.cells.get_mut(&c)
    }

    fn item(&self, p: ObjectPart) -> Option<Item> {
        self.cells.get(&p.cell)?.item(p.kind, p.index)
    }

    /// Puts objects into new states as one undo step.
    fn change(&mut self, changes: Vec<(ObjectPart, State)>) {
        let mut entry = Vec::new();
        for (p, after) in changes {
            let Some(c) = self.cells.get_mut(&p.cell) else { continue };
            let Some(before) = c.state(p.kind, p.index) else { continue };
            c.apply(p.kind, p.index, &after);
            self.live.insert(p);
            entry.push((p.cell, Change { kind: p.kind, index: p.index, before, after }));
        }
        if !entry.is_empty() {
            self.undo.push(entry);
            self.redo.clear();
            self.panel_merge = None;
        }
    }

    /// Moves / turns / scales the selection as one undo step.
    fn transform_selected(&mut self, offset: Vec3, degrees: f32, factor: f32) -> bool {
        let Some(p) = self.selected else { return false };
        let Some(item) = self.item(p) else { return false };
        let Some(after) = self.cells[&p.cell].moved(p.kind, p.index, transformed(&item, offset, degrees, factor)) else { return false };
        self.change(vec![(p, after)]);
        true
    }

    fn delete_selected(&mut self) -> Result<String, String> {
        let p = self.selected.ok_or("nothing selected")?;
        let c = self.cells.get(&p.cell).ok_or("cell not loaded")?;
        if !c.editable(p.kind, p.index) {
            return Err("this object is read-only".into());
        }
        let mut after = c.state(p.kind, p.index).ok_or("no such object")?;
        after.alive = false;
        if let cell::Source::Entity(guid) = c.source(p.kind, p.index) {
            let xml = c.entities.clone().ok_or("no entities.xml")?;
            if entities_xml::has_children(&xml, guid) {
                return Err("other entities are attached to this one: not deleted".into());
            }
            after.xml = Some(Arc::new(entities_xml::remove(&xml, guid).ok_or("entity not found in entities.xml")?));
        }
        let what = describe(c, p);
        self.change(vec![(p, after)]);
        self.selected = None;
        Ok(what)
    }

    fn duplicate_selected(&mut self) -> Result<String, String> {
        let p = self.selected.ok_or("nothing selected")?;
        let c = self.cells.get_mut(&p.cell).ok_or("cell not loaded")?;
        let (index, before, after) = c.duplicate(p.kind, p.index, Vec3::new(2.0, 0.0, 0.0)).ok_or("this object cannot be copied")?;
        c.apply(p.kind, index, &after);
        let copy = ObjectPart { index, ..p };
        self.live.insert(copy);
        self.undo.push(vec![(p.cell, Change { kind: p.kind, index, before, after })]);
        self.redo.clear();
        self.panel_merge = None;
        self.selected = Some(copy);
        Ok(describe(&self.cells[&p.cell], copy))
    }

    fn undo_redo(&mut self, redo: bool) -> bool {
        let entry = if redo { self.redo.pop() } else { self.undo.pop() };
        let Some(entry) = entry else { return false };
        for (c, ch) in if redo { entry.clone() } else { entry.iter().rev().cloned().collect() } {
            if let Some(cell) = self.cells.get_mut(&c) {
                cell.apply(ch.kind, ch.index, if redo { &ch.after } else { &ch.before });
                self.live.insert(ObjectPart { cell: c, kind: ch.kind, index: ch.index });
            }
        }
        if redo { self.undo.push(entry) } else { self.redo.push(entry) }
        self.panel_merge = None;
        true
    }

    pub fn unsaved(&self) -> usize {
        self.cells.values().map(|c| c.unsaved.len()).sum()
    }
}

fn kind_label(k: ObjectKind) -> &'static str {
    match k {
        ObjectKind::Brush => "brush",
        ObjectKind::Vegetation => "vegetation",
        ObjectKind::Entity => "entity",
        ObjectKind::Animated => "animated entity",
    }
}

fn describe(c: &EditCell, p: ObjectPart) -> String {
    let pos = c.item(p.kind, p.index).map(|i| i.position() + cell_origin(p.cell.0, p.cell.1)).unwrap_or_default();
    format!("{} #{} of cell {:03}_{:03} at {:.1}, {:.1}, {:.1}", kind_label(p.kind), p.index, p.cell.0, p.cell.1, pos.x, pos.y, pos.z)
}

pub fn plugin(app: &mut App) {
    app.init_resource::<WorldObjects>()
        .add_systems(
            Update,
            (reloaded, pick, click_drag, keys, scripted, conform, live_parts, commit, draw, save_requests)
                .chain()
                .after(crate::tools::update_cursor)
                .before(aa_world::objects::stream_objects),
        )
        .add_systems(EguiPrimaryContextPass, panel);
}

/// A cell the streamer loaded again (from the files) gets the edited copy back.
fn reloaded(mut objs: ResMut<WorldObjects>, state: Res<ObjectState>) {
    let objs = &mut *objs;
    for (c, cell) in objs.cells.iter_mut() {
        let Some(data) = state.cell_data(*c) else { continue };
        if objs.installed.get(c) != Some(&(Arc::as_ptr(data) as usize)) && cell.edited {
            cell.uncommitted = true;
            cell.respawn.extend(0..=255u8);
        }
    }
}

/// Oriented box of a part hit by a ray (bevy space): distance along the ray.
fn ray_box(origin: Vec3, dir: Vec3, t: &GlobalTransform, aabb: &Aabb) -> Option<f32> {
    let inv = t.affine().inverse();
    let o = inv.transform_point3(origin);
    let d = inv.transform_vector3(dir);
    let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
    let (lo, hi) = (c - h, c + h);
    let (mut t0, mut t1) = (f32::MIN, f32::MAX);
    for k in 0..3 {
        if d[k].abs() < 1e-9 {
            if o[k] < lo[k] || o[k] > hi[k] {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo[k] - o[k]) / d[k], (hi[k] - o[k]) / d[k]);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    // a box around the camera (a big rock, a castle) is not picked from inside
    (t1 >= t0 && t0 > 0.0).then_some(t0)
}

fn pick(
    editor: Res<Editor>,
    cursor: Res<Cursor>,
    mut objs: ResMut<WorldObjects>,
    parts: Query<(&ObjectPart, &GlobalTransform, &Aabb, &ViewVisibility)>,
) {
    objs.hover = None;
    if editor.mode != Mode::Objects || editor.playing || cursor.over_ui {
        return;
    }
    let Some((origin, dir)) = cursor.ray else { return };
    let ground = cursor.ground.map_or(f32::MAX, |g| g.distance(origin) + 0.5);
    let (o, d) = (cry_to_bevy(origin), cry_to_bevy(dir));
    let mut best = (ground, None);
    for (p, t, aabb, vis) in &parts {
        if !vis.get() || p.kind == ObjectKind::Animated {
            continue;
        }
        if let Some(hit) = ray_box(o, d, t, aabb)
            && hit < best.0
        {
            best = (hit, Some(*p));
        }
    }
    objs.hover = best.1.filter(|p| objs.cells.get(&p.cell).is_none_or(|c| c.alive(p.kind, p.index)));
}

fn ground_or(heights: &Heights, query: &TerrainQuery, p: Vec3) -> f32 {
    heights.height_at(query, p.x, p.y).unwrap_or(p.z)
}

#[allow(clippy::too_many_arguments)]
fn click_drag(
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Res<Cursor>,
    window: Query<&Window, With<PrimaryWindow>>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    state: Res<ObjectState>,
    mut editor: ResMut<Editor>,
    mut objs: ResMut<WorldObjects>,
) {
    if editor.mode != Mode::Objects || editor.playing {
        objs.drag = Drag::default();
        return;
    }
    // the spawn tools act on `editor.selected`: none in this mode
    if editor.selected.is_some() {
        editor.selected = None;
    }
    let objs = &mut *objs;
    // (a scripted run keeps its selection)
    if mouse.just_pressed(MouseButton::Left) && !cursor.over_ui && crate::arg("--select-object").is_none() {
        objs.selected = objs.hover;
        objs.drag = Drag::default();
        if let Some(p) = objs.hover {
            let world = editor.world.clone();
            if let Some(c) = objs.ensure(&world, &state, p.cell) {
                let msg = format!("{} — {}", describe(c, p), c.file_of(p.kind, p.index));
                let grab = match (c.item(p.kind, p.index), cursor.ground, c.editable(p.kind, p.index)) {
                    (Some(item), Some(g), true) => Some((item.position() + cell_origin(p.cell.0, p.cell.1), g, c.state(p.kind, p.index))),
                    _ => None,
                };
                if let Some((pos, g, before)) = grab {
                    objs.drag.offset = Some(pos - g);
                    objs.drag.above = pos.z - ground_or(&heights, &query, pos);
                    objs.drag.press = window.single().ok().and_then(Window::cursor_position);
                    objs.drag.before = before;
                }
                editor.status(msg);
            }
        }
        return;
    }
    if !mouse.pressed(MouseButton::Left) {
        // a finished drag is one undo step
        if objs.drag.started
            && let (Some(p), Some(before)) = (objs.selected, objs.drag.before.take())
            && let Some(after) = objs.cells.get(&p.cell).and_then(|c| c.state(p.kind, p.index))
        {
            objs.undo.push(vec![(p.cell, Change { kind: p.kind, index: p.index, before, after })]);
            objs.redo.clear();
            objs.panel_merge = None;
        }
        objs.drag = Drag::default();
        return;
    }
    let (Some(p), Some(offset), Some(g)) = (objs.selected, objs.drag.offset, cursor.ground) else { return };
    if !objs.drag.started {
        let now = window.single().ok().and_then(Window::cursor_position);
        match (objs.drag.press, now) {
            (Some(a), Some(b)) if a.distance(b) >= 4.0 => objs.drag.started = true,
            _ => return,
        }
    }
    let Some(item) = objs.item(p) else { return };
    let origin = cell_origin(p.cell.0, p.cell.1);
    let pos = item.position() + origin;
    let mut to = g + offset;
    to.z = if editor.snap_to_ground { ground_or(&heights, &query, to) + objs.drag.above } else { pos.z };
    if to.distance(pos) < 0.01 {
        return;
    }
    let c = objs.cells.get_mut(&p.cell).unwrap();
    if let Some(after) = c.moved(p.kind, p.index, transformed(&item, to - pos, 0.0, 1.0)) {
        c.apply(p.kind, p.index, &after);
        objs.live.insert(p);
    }
}

#[allow(clippy::too_many_arguments)]
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    mouse: Res<ButtonInput<MouseButton>>,
    egui: Res<EguiWantsInput>,
    cursor: Res<Cursor>,
    mut editor: ResMut<Editor>,
    mut objs: ResMut<WorldObjects>,
) {
    if editor.mode != Mode::Objects || editor.playing || egui.wants_any_keyboard_input() {
        return;
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if ctrl && keys.just_pressed(KeyCode::KeyZ) {
        let done = objs.undo_redo(false);
        editor.status(if done { "objects: undo" } else { "objects: nothing to undo" });
    }
    if ctrl && keys.just_pressed(KeyCode::KeyY) {
        let done = objs.undo_redo(true);
        editor.status(if done { "objects: redo" } else { "objects: nothing to redo" });
    }
    if keys.just_pressed(KeyCode::Escape) {
        objs.selected = None;
    }
    let Some(p) = objs.selected else { return };
    if objs.drag.started {
        return;
    }
    if keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace) {
        match objs.delete_selected() {
            Ok(what) => editor.status(format!("deleted {what}")),
            Err(e) => editor.status(e),
        }
        return;
    }
    if ctrl && keys.just_pressed(KeyCode::KeyD) {
        match objs.duplicate_selected() {
            Ok(what) => editor.status(format!("copied: {what}")),
            Err(e) => editor.status(e),
        }
        return;
    }
    if keys.just_pressed(KeyCode::KeyF) && !ctrl
        && let Some(item) = objs.item(p)
    {
        editor.requests.push(Request::Focus(item.position() + cell_origin(p.cell.0, p.cell.1)));
    }
    let mut turn = 0.0;
    if keys.just_pressed(KeyCode::BracketLeft) {
        turn += 15.0;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        turn -= 15.0;
    }
    if shift && !mouse.pressed(MouseButton::Right) && scroll.delta.y != 0.0 && !cursor.over_ui {
        turn += 5.0 * scroll.delta.y.signum();
    }
    let step = if shift { 2.0 } else { 0.25 };
    let mut lift = 0.0;
    if keys.just_pressed(KeyCode::PageUp) {
        lift += step;
    }
    if keys.just_pressed(KeyCode::PageDown) {
        lift -= step;
    }
    let mut factor = 1.0;
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        factor *= 1.05;
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        factor /= 1.05;
    }
    if (turn != 0.0 || lift != 0.0 || factor != 1.0) && !objs.transform_selected(Vec3::Z * lift, turn, factor) {
        editor.status("this object is read-only");
    }
}

/// `--select-object x,y [--select-kind brush|vegetation|entity] [--delete-selected-object] [--move-selected-object dx,dy,dz]
/// [--objects-save]`: unattended checks.
fn scripted(
    time: Res<Time>,
    state: Res<ObjectState>,
    mut editor: ResMut<Editor>,
    mut objs: ResMut<WorldObjects>,
    mut step: Local<u32>,
    mut at: Local<f32>,
) {
    let Some(target) = crate::arg("--select-object").and_then(|v| {
        let v: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        (v.len() >= 2).then(|| Vec2::new(v[0], v[1]))
    }) else {
        return;
    };
    let now = time.elapsed_secs();
    match *step {
        0 => {
            editor.mode = Mode::Objects;
            let c = ((target.x / 1024.0).floor() as i32, (target.y / 1024.0).floor() as i32);
            let world = editor.world.clone();
            let Some(cell) = objs.ensure(&world, &state, c) else { return };
            let local = target - cell_origin(c.0, c.1).truncate();
            let kind = match crate::arg("--select-kind").as_deref() {
                Some("brush") => Some(ObjectKind::Brush),
                Some("vegetation") => Some(ObjectKind::Vegetation),
                Some("entity") => Some(ObjectKind::Entity),
                _ => None,
            };
            let d = &cell.display;
            let candidates = (0..d.brushes.len())
                .map(|i| (ObjectKind::Brush, i))
                .chain((0..d.vegetation.len()).map(|i| (ObjectKind::Vegetation, i)))
                .chain((0..d.entity_props.len()).map(|i| (ObjectKind::Entity, i)))
                .filter(|(k, i)| kind.is_none_or(|x| x == *k) && cell.editable(*k, *i as u32) && cell.alive(*k, *i as u32));
            let best = candidates
                .filter_map(|(k, i)| Some(((cell.item(k, i as u32)?.position().truncate() - local).length(), k, i)))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let Some((dist, k, i)) = best else {
                editor.status("--select-object: no editable object in that cell");
                *step = 99;
                return;
            };
            let p = ObjectPart { cell: c, kind: k, index: i as u32 };
            let pos = cell.item(k, i as u32).unwrap().position() + cell_origin(c.0, c.1);
            let groups = state.vegetation_groups().map(|g| &**g);
            let msg = format!("world_objects: selected {} ({:.1} m away) model {} [{}]", describe(cell, p), dist, cell.model(k, i as u32, groups), cell.file_of(k, i as u32));
            editor.status(msg);
            info!("world_objects: cell {:03}_{:03} has {} brushes, {} vegetation, {} entity slots", c.0, c.1, d.brushes.len(), d.vegetation.len(), d.entity_props.len());
            objs.selected = Some(p);
            editor.requests.push(Request::Focus(pos));
            *step = 1;
            *at = now;
        }
        1 if now - *at > 6.0 => {
            *step = 2;
            if let Some(v) = crate::arg("--move-selected-object") {
                let v: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                if v.len() == 3 && objs.transform_selected(Vec3::new(v[0], v[1], v[2]), 0.0, 1.0) {
                    editor.status(format!("world_objects: moved by {v:?}"));
                }
            }
            if crate::flag("--delete-selected-object") {
                match objs.delete_selected() {
                    Ok(what) => editor.status(format!("world_objects: deleted {what}")),
                    Err(e) => editor.status(format!("world_objects: delete failed: {e}")),
                }
            }
            if crate::flag("--objects-save") {
                editor.requests.push(Request::SaveObjects);
            }
        }
        _ => {}
    }
}

/// Objects standing on edited terrain follow it: Z += new height - old height at their position.
fn conform(
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    state: Res<ObjectState>,
    mut editor: ResMut<Editor>,
    mut objs: ResMut<WorldObjects>,
) {
    objs.conforming = false;
    // the panel's button: also with the automatic mode off (from the heights the objects were last fitted to)
    let run_all = std::mem::take(&mut objs.conform_now);
    if !objs.auto_conform && !run_all {
        return;
    }
    let world = editor.world.clone();
    let mut moved = 0;
    let objs = &mut *objs;
    for (&c, now) in &heights.edited {
        let before = objs.seen.get(&c).cloned().or_else(|| query.cell(c.0, c.1));
        let Some(before) = before else { continue };
        if Arc::ptr_eq(&before, now) {
            continue;
        }
        if objs.ensure(&world, &state, c).is_none() {
            continue; // objects not loaded yet: try again later
        }
        objs.seen.insert(c, now.clone());
        let cell = objs.cells.get_mut(&c).unwrap();
        let mut changes = Vec::new();
        for (kind, n) in [
            (ObjectKind::Vegetation, cell.display.vegetation.len()),
            (ObjectKind::Brush, cell.display.brushes.len()),
            (ObjectKind::Entity, cell.display.entity_props.len()),
        ] {
            for i in 0..n as u32 {
                if !cell.alive(kind, i) || !cell.editable(kind, i) {
                    continue;
                }
                let Some(item) = cell.item(kind, i) else { continue };
                let p = item.position();
                if !(0.0..1024.0).contains(&p.x) || !(0.0..1024.0).contains(&p.y) {
                    continue;
                }
                let (old, new) = (before.sample(p.x, p.y), now.sample(p.x, p.y));
                let dz = new - old;
                if dz.abs() < 1e-4 {
                    continue;
                }
                // standing on the ground: vegetation within 3 m of it, others within 1.5 m and not huge
                let standing = match &item {
                    Item::Veg(_) => (p.z - old).abs() < 3.0,
                    Item::Brush(b) => (p.z - old).abs() < 1.5 && (b.bbox[3] - b.bbox[0]).max(b.bbox[4] - b.bbox[1]) < 40.0,
                    Item::Entity(_) => (p.z - old).abs() < 1.5,
                };
                if standing {
                    changes.push((kind, i, transformed(&item, Vec3::Z * dz, 0.0, 1.0)));
                }
            }
        }
        for (kind, i, item) in changes {
            if let Some(s) = cell.moved(kind, i, item) {
                cell.apply(kind, i, &s);
                objs.live.insert(ObjectPart { cell: c, kind, index: i });
                moved += 1;
            }
        }
    }
    if moved > 0 {
        objs.conforming = true;
        objs.conformed += moved;
        if run_all {
            editor.status(format!("objects: {moved} conformed to the terrain"));
        }
    }
}

/// Moves (or hides) the drawn parts of edited objects until the streamer rebuilds them.
fn live_parts(
    mut commands: Commands,
    objs: Res<WorldObjects>,
    state: Res<ObjectState>,
    roots: Query<&GlobalTransform, With<ObjectSector>>,
    mut parts: Query<(Entity, &ObjectPart, &ChildOf, &mut Transform, &mut Visibility, Option<&LiveBase>)>,
) {
    if objs.live.is_empty() {
        return;
    }
    // world delta of each live object: display matrix * committed matrix^-1, in Bevy space
    let mut deltas: HashMap<ObjectPart, (Mat4, bool)> = HashMap::new();
    for p in &objs.live {
        let Some(cell) = objs.cells.get(&p.cell) else { continue };
        let Some(shown) = cell.item(p.kind, p.index) else { continue };
        let Some(data) = state.cell_data(p.cell) else { continue };
        let i = p.index as usize;
        let committed = match p.kind {
            ObjectKind::Brush => data.objects.brushes.get(i).map(|b| b.mat4()),
            ObjectKind::Vegetation => data.objects.vegetation.get(i).map(|v| v.mat4()),
            ObjectKind::Entity => data.objects.entity_props.get(i).map(|e| e.matrix),
            ObjectKind::Animated => None,
        };
        let Some(committed) = committed.filter(|m| m.determinant().abs() > 1e-12) else { continue };
        let w = Mat4::from_quat(CRY_TO_BEVY) * Mat4::from_translation(cell_origin(p.cell.0, p.cell.1));
        let delta = w * shown.matrix() * committed.inverse() * w.inverse();
        deltas.insert(*p, (delta, cell.alive(p.kind, p.index)));
    }
    for (e, p, parent, mut t, mut vis, base) in &mut parts {
        let Some(&(delta, alive)) = deltas.get(p) else { continue };
        let base = match base {
            Some(b) => b.0,
            None => {
                commands.entity(e).insert(LiveBase(*t));
                *t
            }
        };
        let root = roots.get(parent.parent()).map_or(Vec3::ZERO, |g| g.translation());
        let m = Mat4::from_translation(-root) * delta * Mat4::from_translation(root) * base.to_matrix();
        let next = Transform::from_matrix(m);
        if *t != next {
            *t = next;
        }
        let v = if alive { Visibility::Inherited } else { Visibility::Hidden };
        if *vis != v {
            *vis = v;
        }
    }
}

/// Hands the edited cells to the streamer when no drag or terrain stroke is going on.
fn commit(mut commands: Commands, mut objs: ResMut<WorldObjects>, mut state: ResMut<ObjectState>) {
    if objs.drag.started || objs.conforming {
        return;
    }
    let objs = &mut *objs;
    if objs.conformed > 0 {
        info!("world_objects: terrain stroke done, {} object moves to follow the ground", std::mem::take(&mut objs.conformed));
    }
    let mut any = false;
    for (c, cell) in objs.cells.iter_mut().filter(|(_, c)| c.uncommitted) {
        let removed = cell.removed.clone();
        let respawn: Vec<u8> = cell.respawn.drain().collect();
        match state.replace_cell_objects(&mut commands, *c, cell.display.clone(), |k, i| removed.contains(&(k, i)), &respawn) {
            Some(deferred) => {
                cell.uncommitted = !deferred.is_empty();
                cell.respawn.extend(deferred);
                if let Some(data) = state.cell_data(*c) {
                    objs.installed.insert(*c, Arc::as_ptr(data) as usize);
                }
                any = true;
            }
            // not loaded: done when it loads ([`reloaded`])
            None => cell.uncommitted = false,
        }
    }
    if any {
        // the rebuilt sectors draw the new state; parts still moved by hand are the old ones until the swap
        objs.live.retain(|p| objs.cells.get(&p.cell).is_some_and(|c| c.uncommitted));
    }
}

fn corners(lo: Vec3, hi: Vec3) -> [Vec3; 8] {
    std::array::from_fn(|i| Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z }))
}

fn draw_box(gizmos: &mut Gizmos, c: [Vec3; 8], color: Color) {
    for (a, b) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
        gizmos.line(c[a], c[b], color);
    }
}

fn draw(
    editor: Res<Editor>,
    state: Res<ObjectState>,
    mut objs: ResMut<WorldObjects>,
    parts: Query<(&ObjectPart, &GlobalTransform, &Aabb)>,
    mut gizmos: Gizmos,
) {
    if editor.mode != Mode::Objects || editor.playing {
        return;
    }
    let objs = &mut *objs;
    // hover: the parts' boxes
    if let Some(h) = objs.hover.filter(|h| Some(*h) != objs.selected) {
        for (p, t, aabb) in &parts {
            if *p == h {
                let (c, e) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
                draw_box(&mut gizmos, corners(c - e, c + e).map(|v| t.transform_point(v)), Color::srgba(1.0, 1.0, 1.0, 0.7));
            }
        }
    }
    let Some(sel) = objs.selected else { return };
    let Some(cell) = objs.cells.get(&sel.cell) else { return };
    if !cell.alive(sel.kind, sel.index) {
        return;
    }
    let Some(item) = cell.item(sel.kind, sel.index) else { return };
    let w = Mat4::from_quat(CRY_TO_BEVY) * Mat4::from_translation(cell_origin(sel.cell.0, sel.cell.1));
    // object-space box from the parts drawn for the committed state
    if objs.bounds.is_none_or(|b| b.0 != sel)
        && let Some(data) = state.cell_data(sel.cell)
    {
        let i = sel.index as usize;
        let committed = match sel.kind {
            ObjectKind::Brush => data.objects.brushes.get(i).map(|b| b.mat4()),
            ObjectKind::Vegetation => data.objects.vegetation.get(i).map(|v| v.mat4()),
            ObjectKind::Entity => data.objects.entity_props.get(i).map(|e| e.matrix),
            ObjectKind::Animated => None,
        };
        if let Some(m) = committed.filter(|m| m.determinant().abs() > 1e-12) {
            let to_object = (w * m).inverse();
            let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
            for (p, t, aabb) in &parts {
                if *p != sel {
                    continue;
                }
                let (c, e) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
                for v in corners(c - e, c + e) {
                    let q = to_object.transform_point3(t.transform_point(v));
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
            }
            if lo.x <= hi.x {
                objs.bounds = Some((sel, lo, hi));
            }
        }
    }
    let color = Color::srgb(1.0, 0.75, 0.1);
    let m = w * item.matrix();
    // brushes and vegetation: their record's box (the parts' boxes of swaying plants are loose spheres)
    let record_box = match &item {
        Item::Brush(b) => Some(b.bbox),
        Item::Veg(v) => cell.veg_orig.get(sel.index as usize).map(|(m0, b0)| aa_world::placement_write::transform_bbox(b0, v.mat4() * m0.inverse())),
        Item::Entity(_) => None,
    };
    if let Some(b) = record_box {
        let o = cell_origin(sel.cell.0, sel.cell.1);
        let (lo, hi) = (Vec3::from_slice(&b[..3]) + o, Vec3::from_slice(&b[3..]) + o);
        draw_box(&mut gizmos, corners(lo, hi).map(cry_to_bevy), color);
        let base = m.transform_point3(Vec3::ZERO);
        gizmos.line(base, base + Vec3::Y * 2.0, color);
        return;
    }
    match objs.bounds.filter(|b| b.0 == sel) {
        Some((_, lo, hi)) => draw_box(&mut gizmos, corners(lo, hi).map(|v| m.transform_point3(v)), color),
        None => {
            let p = m.transform_point3(Vec3::ZERO);
            gizmos.line(p, p + Vec3::Y * 3.0, color);
        }
    }
    let base = m.transform_point3(Vec3::ZERO);
    gizmos.line(base, base + Vec3::Y * 2.0, color);
}

fn save_requests(mut editor: ResMut<Editor>, mut objs: ResMut<WorldObjects>) {
    let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::SaveObjects)) else { return };
    editor.requests.remove(i);
    let (world, overlay) = (editor.world.clone(), editor.paths.overlay.clone());
    let mut written = Vec::new();
    let mut errors = Vec::new();
    for cell in objs.cells.values_mut().filter(|c| !c.unsaved.is_empty()) {
        match cell.save(&world, &overlay) {
            Ok(files) => written.extend(files),
            Err(e) => errors.push(format!("{:03}_{:03}: {e:#}", cell.cell.0, cell.cell.1)),
        }
    }
    let msg = if !errors.is_empty() {
        format!("objects: save failed: {}", errors.join("; "))
    } else if written.is_empty() {
        "objects: nothing to save".into()
    } else {
        for f in &written {
            info!("world_objects: wrote {}", f.display());
        }
        format!("objects: {} file(s) saved to {}", written.len(), overlay.display())
    };
    editor.status(msg);
}

fn panel(mut contexts: EguiContexts, mut editor: ResMut<Editor>, mut objs: ResMut<WorldObjects>, state: Res<ObjectState>) -> Result {
    if editor.mode != Mode::Objects || editor.playing {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let groups = state.vegetation_groups().cloned();
    egui::Window::new("World object").default_pos([320.0, 700.0]).default_width(330.0).show(ctx, |ui| {
        let objs = &mut *objs;
        let shown = objs.selected.or(objs.hover);
        match shown.and_then(|p| objs.cells.get(&p.cell).map(|c| (p, c))) {
            Some((p, c)) => {
                let item = c.item(p.kind, p.index);
                ui.label(egui::RichText::new(c.model(p.kind, p.index, groups.as_deref())).strong());
                ui.label(format!("{} #{}  ·  cell {:03}_{:03}", kind_label(p.kind), p.index, p.cell.0, p.cell.1));
                ui.label(c.file_of(p.kind, p.index));
                if objs.selected != Some(p) {
                    ui.weak("(hover — click to select)");
                }
                if let (Some(item), true) = (item, objs.selected == Some(p) && c.alive(p.kind, p.index)) {
                    let origin = cell_origin(p.cell.0, p.cell.1);
                    let pos = item.position() + origin;
                    let (yaw, scale) = item.yaw_scale();
                    let (mut x, mut y, mut z, mut yw, mut sc) = (pos.x, pos.y, pos.z, yaw, scale);
                    let editable = c.editable(p.kind, p.index);
                    let mut changed = false;
                    ui.add_enabled_ui(editable, |ui| {
                        egui::Grid::new("wo_xf").num_columns(2).show(ui, |ui| {
                            for (label, v, speed) in [("X", &mut x, 0.05), ("Y", &mut y, 0.05), ("Z", &mut z, 0.02), ("Yaw °", &mut yw, 0.5), ("Scale", &mut sc, 0.005)] {
                                ui.label(label);
                                changed |= ui.add(egui::DragValue::new(v).speed(speed).max_decimals(3)).changed();
                                ui.end_row();
                            }
                        });
                    });
                    if changed && editable {
                        let next = transformed(&item, Vec3::new(x, y, z) - pos, yw - yaw, if scale > 1e-6 { sc / scale } else { 1.0 });
                        if let Some(after) = c.moved(p.kind, p.index, next) {
                            if objs.panel_merge == Some(p) {
                                // continue the last panel step
                                let cell = objs.cells.get_mut(&p.cell).unwrap();
                                cell.apply(p.kind, p.index, &after);
                                if let Some(last) = objs.undo.last_mut().and_then(|e| e.last_mut()) {
                                    last.1.after = after;
                                }
                                objs.live.insert(p);
                            } else {
                                objs.change(vec![(p, after)]);
                                objs.panel_merge = Some(p);
                            }
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(editable, egui::Button::new("Delete (Del)")).clicked() {
                            match objs.delete_selected() {
                                Ok(what) => editor.status(format!("deleted {what}")),
                                Err(e) => editor.status(e),
                            }
                        }
                        if ui.add_enabled(editable, egui::Button::new("Copy (Ctrl+D)")).clicked() {
                            match objs.duplicate_selected() {
                                Ok(what) => editor.status(format!("copied: {what}")),
                                Err(e) => editor.status(e),
                            }
                        }
                        if ui.button("Focus (F)").clicked() {
                            editor.requests.push(Request::Focus(pos));
                        }
                    });
                }
            }
            None => {
                ui.label("Point at a brush, tree or prop and click it.");
            }
        }
        ui.separator();
        ui.weak("Drag: move · [ ] / Shift+wheel: turn · PgUp/PgDn: raise/lower (Shift ×8) · + −: scale");
        ui.horizontal(|ui| {
            if ui.add_enabled(!objs.undo.is_empty(), egui::Button::new("Undo")).clicked() {
                objs.undo_redo(false);
            }
            if ui.add_enabled(!objs.redo.is_empty(), egui::Button::new("Redo")).clicked() {
                objs.undo_redo(true);
            }
        });
        ui.checkbox(&mut objs.auto_conform, "Objects follow terrain sculpting");
        if ui.button("Conform objects to terrain now").clicked() {
            objs.conform_now = true;
        }
        let unsaved = objs.unsaved();
        if ui.add_enabled(unsaved > 0, egui::Button::new(format!("Save objects ({unsaved} file(s)) — Ctrl+S"))).clicked() {
            editor.requests.push(Request::SaveObjects);
        }
    });
    Ok(())
}
