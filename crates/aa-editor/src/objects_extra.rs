//! The "More objects" window (Objects mode): new world objects from a model browser, and the static objects the
//! world-object tools do not pick: decals, terrain roads, water volumes and interior (`visareas.dat`) brushes.
//!
//! * **New objects.** A searchable list of the pak's `objects/**.cgf` models (or the world's vegetation groups). The
//!   picked model follows the cursor as a ghost; a left click places it. A brush goes into the cell's `brush.dat`
//!   (cells with `brush.dat` + `statobjs.dat`; the model is appended to `statobjs.dat` and to the `object.dat` model
//!   table so both tables keep the same order) or into the `object.dat` octree (cells without `brush.dat`). The new
//!   record is a copy of an existing record of the cell with the model, matrix and box set (box from the model's
//!   LOD 0 bounds), material -1 (the model's own `.mtl`). A vegetation instance is a copy of a `vegetation.dat`
//!   record with the group set. The new object is then an ordinary world object: selected, movable, deletable and
//!   saved by the Objects tools (Ctrl+S).
//! * **Decals / roads / water** live in the `object.dat` octree (item types 9, 13, 11; layouts in
//!   `aa_world::placement`). They can be moved (whole object: box, position, spline / outline points, the water
//!   plane) and deleted, editing the same octree the Objects tools save, so untouched bytes stay as they were. The
//!   streamer builds their meshes once when a cell loads, so the drawn geometry changes after "Reload objects"
//!   (which drops and reloads every loaded cell's objects from the files: save first); until then the outlines show
//!   the edited place.
//! * **Interior brushes** (`visareas.dat`: area / portal records, each with its own small octree) can be moved and
//!   deleted; the drawn brush follows at once. Saved with Ctrl+S next to the other files of the cell.

use std::collections::HashMap;

use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use aa_world::coords::cell_origin;
use aa_world::objects::ObjectState;
use aa_world::objects::edit::{ObjectKind, ObjectPart};
use aa_world::placement::{Brush, Vegetation};
use aa_world::placement_write::{ObjectTree, set_brush, set_vegetation, transform_bbox};
use aa_world::{CELL_SIZE, cry_to_bevy};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::tools::Cursor;
use crate::world_objects::WorldObjects;
use crate::world_objects::cell::{EditCell, File, Source};
use crate::{Editor, Mode, Request};

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn put_f32(b: &mut [u8], o: usize, v: f32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// The kinds this window edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExtraKind {
    Decal,
    Road,
    Water,
    Interior,
}

impl ExtraKind {
    pub const ALL: [ExtraKind; 4] = [ExtraKind::Decal, ExtraKind::Road, ExtraKind::Water, ExtraKind::Interior];
    /// `object.dat` item type.
    pub fn item(self) -> u32 {
        match self {
            ExtraKind::Decal => 9,
            ExtraKind::Road => 13,
            ExtraKind::Water => 11,
            ExtraKind::Interior => 1,
        }
    }
    fn label(self) -> &'static str {
        match self {
            ExtraKind::Decal => "Decals",
            ExtraKind::Road => "Roads",
            ExtraKind::Water => "Water",
            ExtraKind::Interior => "Interior brushes",
        }
    }
    fn colour(self) -> Color {
        match self {
            ExtraKind::Decal => Color::srgb(1.0, 0.55, 0.1),
            ExtraKind::Road => Color::srgb(0.95, 0.9, 0.2),
            ExtraKind::Water => Color::srgb(0.2, 0.8, 1.0),
            ExtraKind::Interior => Color::srgb(0.9, 0.3, 0.9),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Octree items: decals, roads, water

/// Moves an octree item of type `kind` by `d` (cell-local): its box and every position it stores.
pub fn translate_item(kind: u32, b: &mut [u8], d: Vec3) {
    let shift = |b: &mut [u8], o: usize| {
        for k in 0..3 {
            put_f32(b, o + k * 4, f32_at(b, o + k * 4) + d[k]);
        }
    };
    shift(b, 0);
    shift(b, 12);
    match kind {
        9 => shift(b, 43),
        13 => {
            let n = u32_at(b, 39) as usize;
            for i in 0..n {
                shift(b, 63 + i * 12);
            }
        }
        11 => {
            // the plane's distance is `-surface` (normal (0, 0, 1))
            if f32_at(b, 79) > 0.99 {
                put_f32(b, 83, f32_at(b, 83) - d.z);
            }
            let n = u32_at(b, 103) as usize + u32_at(b, 115) as usize;
            for i in 0..n {
                shift(b, 119 + i * 12);
            }
        }
        _ => {}
    }
}

/// Cell-local outline of an item for drawing: decals / interior brushes their box, roads the spline edges, water
/// the outline.
fn outline(kind: u32, b: &[u8]) -> Vec<Vec3> {
    let p = |o: usize| Vec3::new(f32_at(b, o), f32_at(b, o + 4), f32_at(b, o + 8));
    match kind {
        13 => (0..u32_at(b, 39) as usize).map(|i| p(63 + i * 12)).collect(),
        11 => (0..u32_at(b, 103) as usize).map(|i| p(119 + i * 12)).collect(),
        _ => Vec::new(),
    }
}

fn bbox(b: &[u8]) -> (Vec3, Vec3) {
    (Vec3::new(f32_at(b, 0), f32_at(b, 4), f32_at(b, 8)), Vec3::new(f32_at(b, 12), f32_at(b, 16), f32_at(b, 20)))
}

// ---------------------------------------------------------------------------------------------------------------
// visareas.dat

/// One area / portal / occluder record: its fixed part verbatim and its object octree.
#[derive(Clone, Debug)]
struct VisRecord {
    /// Head (264 B, + 12 for type 3), point count and points, verbatim. `u32 @84` is the object bytes' length.
    prefix: Vec<u8>,
    /// The record's objects (`None`: kept verbatim in `raw`).
    tree: Option<ObjectTree>,
    raw: Vec<u8>,
}

/// `visareas.dat` as an editable model (layout: `aa_world::placement::visareas`).
#[derive(Clone, Debug)]
pub struct Visareas {
    tables: Vec<u8>,
    header: Vec<u8>,
    records: Vec<VisRecord>,
    pub models: Vec<String>,
}

const EMPTY_TABLES: [u8; 8] = [0; 8];

impl Visareas {
    pub fn parse(data: &[u8]) -> Option<Visareas> {
        let table_end = |pos: usize| -> Option<usize> {
            let n = u32_at(data.get(pos..pos + 4)?, 0) as usize;
            pos.checked_add(4)?.checked_add(n.checked_mul(260)?)
        };
        let t1 = table_end(0)?;
        let t2 = table_end(t1)?;
        let models = data
            .get(4..t1)?
            .as_chunks::<260>()
            .0
            .iter()
            .map(|e| {
                let name = &e[4..];
                String::from_utf8_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())]).trim().to_string()
            })
            .collect();
        let header = data.get(t2..t2 + 20)?.to_vec();
        if u32_at(&header, 0) != 4 || u32_at(&header, 4) as usize != data.len() - t2 {
            return None;
        }
        let count = u32_at(&header, 8) as usize + u32_at(&header, 12) as usize + u32_at(&header, 16) as usize;
        let mut pos = t2 + 20;
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            let kind = u32_at(data.get(pos..pos + 4)?, 0);
            let head = 264 + if kind == 3 { 12 } else if kind == 2 { 0 } else { return None };
            let points = u32_at(data.get(pos + head..pos + head + 4)?, 0) as usize;
            let plen = head + 4 + points.checked_mul(12)?;
            let prefix = data.get(pos..pos + plen)?.to_vec();
            let olen = u32_at(&prefix, 84) as usize;
            let raw = data.get(pos + plen..pos + plen + olen)?.to_vec();
            pos += plen + olen;
            let tree = (raw.len() > 4)
                .then(|| {
                    let mut buf = EMPTY_TABLES.to_vec();
                    buf.extend_from_slice(&raw);
                    ObjectTree::parse(&buf).filter(|t| t.tail.is_empty() && t.write()[8..] == raw[..])
                })
                .flatten();
            records.push(VisRecord { prefix, tree, raw });
        }
        (pos == data.len()).then_some(Visareas { tables: data[..t2].to_vec(), header, records, models })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut body = Vec::new();
        for r in &self.records {
            let objects = match &r.tree {
                Some(t) => t.write()[8..].to_vec(),
                None => r.raw.clone(),
            };
            let mut prefix = r.prefix.clone();
            put_u32(&mut prefix, 84, objects.len() as u32);
            body.extend(prefix);
            body.extend(objects);
        }
        let mut header = self.header.clone();
        put_u32(&mut header, 4, (20 + body.len()) as u32);
        let mut out = self.tables.clone();
        out.extend(header);
        out.extend(body);
        out
    }

    /// Live brushes: (record, item index), in the reader's order.
    pub fn brushes(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (ri, r) in self.records.iter().enumerate() {
            if let Some(t) = &r.tree {
                out.extend(t.items_of(1).into_iter().map(|i| (ri, i)));
            }
        }
        out
    }

    fn item(&self, (r, i): (usize, usize)) -> Option<&aa_world::placement_write::Item> {
        self.records.get(r)?.tree.as_ref()?.items.get(i)
    }

    fn item_mut(&mut self, (r, i): (usize, usize)) -> Option<&mut aa_world::placement_write::Item> {
        self.records.get_mut(r)?.tree.as_mut()?.items.get_mut(i)
    }
}

/// A cell's interiors with the display index of each interior brush.
struct Interiors {
    file: Visareas,
    /// (record, item) -> display brush index (`None`: not matched).
    display: Vec<((usize, usize), Option<usize>)>,
    unsaved: bool,
}

// ---------------------------------------------------------------------------------------------------------------

/// What the window works on.
#[derive(Resource, Default)]
pub struct Extra {
    pub open: bool,
    tab: Tab,
    /// The cell the lists show.
    cell: Option<(i32, i32)>,
    /// Selected (kind, octree item index / interior list index).
    selected: Option<(ExtraKind, usize)>,
    step: f32,
    interiors: HashMap<(i32, i32), Interiors>,
    /// `objects/**.cgf` of the pak.
    models: Option<Vec<String>>,
    filter: String,
    /// The model (or vegetation group) being placed.
    placing: Option<Placing>,
    ghost: Option<(String, Entity)>,
    yaw: f32,
    /// Select this new object after the Objects tools ran.
    select: Option<ObjectPart>,
    reload: bool,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Placing {
    Model(String),
    Veg(u16, String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    New,
    Vegetation,
    Kind(ExtraKind),
}

pub fn plugin(app: &mut App) {
    app.insert_resource(Extra { open: crate::flag("--more-objects"), step: 1.0, ..default() })
        .add_systems(Update, (focus_cell, ghost, place, scripted, draw).chain().after(crate::tools::update_cursor))
        .add_systems(Update, save_interiors.after(scripted).after(crate::tools::keys).before(crate::world_objects::save_requests))
        .add_systems(PostUpdate, select_new)
        .add_systems(Last, reload_objects)
        .add_systems(EguiPrimaryContextPass, window);
}

fn cell_of(p: Vec3) -> (i32, i32) {
    ((p.x / CELL_SIZE).floor() as i32, (p.y / CELL_SIZE).floor() as i32)
}

/// The lists follow the cell under the cursor (else the camera).
fn focus_cell(editor: Res<Editor>, cursor: Res<Cursor>, camera: Query<&Transform, With<crate::camera::EditorCamera>>, mut extra: ResMut<Extra>) {
    if editor.mode != Mode::Objects || !extra.open || extra.placing.is_some() {
        return;
    }
    let at = cursor.ground.or_else(|| camera.single().ok().map(|t| aa_world::bevy_to_cry(t.translation)));
    if let Some(p) = at {
        let c = cell_of(p);
        if extra.cell != Some(c) && !cursor.over_ui {
            extra.cell = Some(c);
            extra.selected = None;
        }
    }
}

/// Reads a cell's `visareas.dat` and finds its brushes among the cell's drawn brushes (by model and position).
fn load_interiors(world: &str, cell: &EditCell) -> Option<Interiors> {
    let dir = aa_world::world::cell_dir(world, cell.cell.0, cell.cell.1);
    let data = aa_pak::Pak::global().read(&format!("{dir}/client/visareas.dat")).ok()?;
    let file = Visareas::parse(&data)?;
    let mut used = std::collections::HashSet::new();
    let mut display = Vec::new();
    for key in file.brushes() {
        let Some(item) = file.item(key) else { continue };
        let model_index = u32_at(&item.bytes, 123) as usize;
        let model = file.models.get(model_index).map(|m| m.to_ascii_lowercase());
        let t = Vec3::new(f32_at(&item.bytes, 67 + 12), f32_at(&item.bytes, 67 + 28), f32_at(&item.bytes, 67 + 44));
        let found = (0..cell.display.brushes.len()).find(|&i| {
            if used.contains(&i) || cell.source(ObjectKind::Brush, i as u32) != Source::ReadOnly {
                return false;
            }
            let b = &cell.display.brushes[i];
            let same_model = model.as_deref().is_some_and(|m| cell.display.models.get(b.model as usize).is_some_and(|n| n.eq_ignore_ascii_case(m)));
            same_model && b.translation().distance(t) < 0.01
        });
        if let Some(i) = found {
            used.insert(i);
        }
        display.push((key, found));
    }
    Some(Interiors { file, display, unsaved: false })
}

/// Center (cell-local) of an extra object.
fn center(extra: &Extra, cell: &EditCell, kind: ExtraKind, i: usize) -> Option<Vec3> {
    let bytes = match kind {
        ExtraKind::Interior => {
            let int = extra.interiors.get(&cell.cell)?;
            &int.file.item(int.display.get(i)?.0)?.bytes
        }
        _ => &cell.octree.as_ref()?.items.get(i)?.bytes,
    };
    let (lo, hi) = bbox(bytes);
    Some((lo + hi) / 2.0)
}

/// Live items of a kind in a cell: octree item indices, or interior list indices.
fn items(extra: &Extra, cell: &EditCell, kind: ExtraKind) -> Vec<usize> {
    match kind {
        ExtraKind::Interior => extra
            .interiors
            .get(&cell.cell)
            .map(|int| (0..int.display.len()).filter(|&i| int.file.item(int.display[i].0).is_some_and(|it| !it.deleted)).collect())
            .unwrap_or_default(),
        _ => cell.octree.as_ref().map(|t| t.items_of(kind.item())).unwrap_or_default(),
    }
}

/// Moves (`Some(delta)`) or deletes (`None`) an extra object.
fn edit(extra: &mut Extra, cell: &mut EditCell, kind: ExtraKind, i: usize, delta: Option<Vec3>) -> bool {
    match kind {
        ExtraKind::Interior => {
            let Some(int) = extra.interiors.get_mut(&cell.cell) else { return false };
            let Some(&(key, shown)) = int.display.get(i) else { return false };
            let Some(item) = int.file.item_mut(key) else { return false };
            match delta {
                Some(d) => {
                    let mut b = Brush {
                        matrix: std::array::from_fn(|k| f32_at(&item.bytes, 67 + k * 4)),
                        model: 0,
                        material: 0,
                        flags: 0,
                        max_view_dist: 0.0,
                        lod_ratio: 0,
                        bbox: std::array::from_fn(|k| f32_at(&item.bytes, k * 4)),
                    };
                    b.matrix[3] += d.x;
                    b.matrix[7] += d.y;
                    b.matrix[11] += d.z;
                    b.bbox = transform_bbox(&b.bbox, Mat4::from_translation(d));
                    set_brush(&mut item.bytes, &b, Vec3::ZERO);
                    if let Some(s) = shown
                        && let Some(db) = cell.display.brushes.get_mut(s)
                    {
                        let old = db.translation();
                        db.matrix[3] += d.x;
                        db.matrix[7] += d.y;
                        db.matrix[11] += d.z;
                        db.bbox = transform_bbox(&db.bbox, Mat4::from_translation(d));
                        cell.respawn.insert(sector64(old));
                        cell.respawn.insert(sector64(db.translation()));
                    }
                }
                None => {
                    item.deleted = true;
                    if let Some(s) = shown {
                        cell.removed.insert((ObjectKind::Brush, s as u32));
                        if let Some(db) = cell.display.brushes.get(s) {
                            cell.respawn.insert(sector64(db.translation()));
                        }
                    }
                }
            }
            int.unsaved = true;
            cell.uncommitted = true;
            cell.edited = true;
            true
        }
        _ => {
            let Some(tree) = cell.octree.as_mut() else { return false };
            let Some(item) = tree.items.get_mut(i) else { return false };
            match delta {
                Some(d) => translate_item(item.kind, &mut item.bytes, d),
                None => item.deleted = true,
            }
            cell.unsaved.insert(File::Object);
            cell.edited = true;
            true
        }
    }
}

fn sector64(p: Vec3) -> u8 {
    let sx = (p.x / 64.0).floor().clamp(0.0, 15.0) as u8;
    let sy = (p.y / 64.0).floor().clamp(0.0, 15.0) as u8;
    sx * 16 + sy
}

// ---------------------------------------------------------------------------------------------------------------
// New objects

/// The pak's placeable models (`objects/**.cgf`), as the cells' model tables write them (`game/...`).
fn model_list() -> Vec<String> {
    let mut v: Vec<String> = aa_pak::Pak::global()
        .names()
        .filter(|n| n.ends_with(".cgf") && n.contains("objects/") && !n.contains("_lod"))
        .map(|n| if n.starts_with("game/") { n.to_string() } else { format!("game/{n}") })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Model-space bounds of a `.cgf` (LOD 0), cry space.
fn model_bounds(path: &str) -> Option<(Vec3, Vec3)> {
    aa_cry::mesh::Model::load(path).ok()?.bounds()
}

/// Appends a model name to a model table (`u32 n` then `n` x { `u32`, `char[256]` } for `object.dat`, `n` x
/// `char[256]` for `statobjs.dat`). Returns the table with the name and its index (an existing entry is reused).
pub fn add_model(table: &[u8], entry: usize, name: &str) -> Option<(Vec<u8>, u32)> {
    let n = u32_at(table.get(..4)?, 0) as usize;
    let name_at = entry - 256;
    for i in 0..n {
        let e = table.get(4 + i * entry..4 + (i + 1) * entry)?;
        let s = &e[name_at..];
        let s = &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())];
        if String::from_utf8_lossy(s).trim().eq_ignore_ascii_case(name) {
            return Some((table.to_vec(), i as u32));
        }
    }
    if name.len() >= 256 {
        return None;
    }
    let mut out = table[..4 + n * entry].to_vec();
    put_u32(&mut out, 0, n as u32 + 1);
    let mut e = vec![0u8; entry];
    e[name_at..name_at + name.len()].copy_from_slice(name.as_bytes());
    out.extend(e);
    out.extend_from_slice(&table[4 + n * entry..]);
    Some((out, n as u32))
}

/// Places a new brush with `model` at the cell-local position `p` (yaw in degrees). Returns its display index.
pub fn place_brush(world: &str, overlay: &std::path::Path, cell: &mut EditCell, model: &str, p: Vec3, yaw: f32) -> Result<u32, String> {
    let (lo, hi) = model_bounds(model).ok_or_else(|| format!("{model}: cannot read the model"))?;
    let m = Mat4::from_rotation_translation(Quat::from_rotation_z(yaw.to_radians()), p);
    let rows = m.transpose().to_cols_array();
    let matrix: [f32; 12] = std::array::from_fn(|i| rows[i]);
    let bbox = transform_bbox(&[lo.x, lo.y, lo.z, hi.x, hi.y, hi.z], m);
    let dir = aa_world::world::cell_dir(world, cell.cell.0, cell.cell.1);
    if let Some(file) = cell.brush_dat.as_mut() {
        // brush.dat: statobjs.dat names the models
        let template = file.records.iter().find(|r| !r.deleted).ok_or("brush.dat has no record to copy")?.bytes.clone();
        let stat_path = format!("{dir}/client/statobjs.dat");
        let stat = aa_pak::Pak::global().read(&stat_path).map_err(|e| format!("statobjs.dat: {e}"))?;
        let (stat, index) = add_model(&stat, 256, model).ok_or("statobjs.dat: cannot add the model")?;
        let out = overlay.join(&stat_path);
        std::fs::create_dir_all(out.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&out, &stat).map_err(|e| e.to_string())?;
        // keep object.dat's table in the same order (brush.dat materials index it)
        if let Some(tree) = cell.octree.as_mut() {
            let models_end = 4 + u32_at(&tree.head, 0) as usize * 260;
            if let Some((t, i)) = add_model(&tree.head[..models_end], 260, model)
                && i == index
            {
                let mut head = t;
                head.extend_from_slice(&tree.head[models_end..]);
                tree.head = head;
                cell.unsaved.insert(File::Object);
            }
        }
        if cell.display.models.len() <= index as usize {
            cell.display.models.resize(index as usize + 1, String::new());
        }
        cell.display.models[index as usize] = model.to_string();
        let sector = file.sector_at(p);
        let brush = Brush { matrix, model: index, material: -1, flags: u32_at(&template, 32), max_view_dist: f32_at(&template, 28), lod_ratio: template[37], bbox };
        let mut rec = template;
        put_u32(&mut rec, 123, index);
        rec[115..119].copy_from_slice(&(-1i32).to_le_bytes());
        set_brush(&mut rec, &brush, file.sector_origin(sector));
        file.records.push(aa_world::placement_write::Record { sector, bytes: rec, deleted: false });
        let src = Source::BrushDat(file.records.len() - 1);
        cell.unsaved.insert(File::Brush);
        Ok(push_brush(cell, brush, src))
    } else if let Some(tree) = cell.octree.as_mut() {
        let template = tree.items_of(1).first().map(|&i| tree.items[i].bytes.clone()).ok_or("object.dat has no brush to copy")?;
        let models_end = 4 + u32_at(&tree.head, 0) as usize * 260;
        let (t, index) = add_model(&tree.head[..models_end], 260, model).ok_or("object.dat: cannot add the model")?;
        let mut head = t;
        head.extend_from_slice(&tree.head[models_end..]);
        tree.head = head;
        if cell.display.models.len() <= index as usize {
            cell.display.models.resize(index as usize + 1, String::new());
        }
        cell.display.models[index as usize] = model.to_string();
        let brush = Brush { matrix, model: index, material: -1, flags: u32_at(&template, 32), max_view_dist: f32_at(&template, 28), lod_ratio: template[37], bbox };
        let mut rec = template;
        put_u32(&mut rec, 123, index);
        rec[115..119].copy_from_slice(&(-1i32).to_le_bytes());
        set_brush(&mut rec, &brush, Vec3::ZERO);
        let i = tree.push(1, rec);
        cell.unsaved.insert(File::Object);
        Ok(push_brush(cell, brush, Source::Octree(i)))
    } else {
        Err("this cell has no brush.dat / object.dat to add to".into())
    }
}

fn push_brush(cell: &mut EditCell, brush: Brush, src: Source) -> u32 {
    let n = cell.display.brushes.len();
    cell.respawn.insert(sector64(brush.translation()));
    cell.brush_orig.push((brush.mat4(), brush.bbox));
    cell.display.brushes.push(brush);
    cell.brush_src.push(src);
    cell.uncommitted = true;
    cell.edited = true;
    n as u32
}

/// Places a vegetation instance of `group` (model `model`) at `p`, copied from a `vegetation.dat` record.
pub fn place_vegetation(cell: &mut EditCell, group: u16, model: &str, p: Vec3) -> Result<u32, String> {
    let file = cell.veg_dat.as_mut().ok_or("this cell has no vegetation.dat")?;
    let ti = cell.veg_src.iter().position(|s| matches!(s, Source::VegDat(_))).ok_or("vegetation.dat has no record to copy")?;
    let Source::VegDat(r) = cell.veg_src[ti] else { unreachable!() };
    let mut v: Vegetation = cell.display.vegetation[ti].clone();
    v.group = group;
    v.pos = p;
    v.scale = 1.0;
    v.angle_x = 0;
    v.angle_y = 0;
    v.angle_z = 0;
    let (lo, hi) = model_bounds(model).unwrap_or((Vec3::new(-1.0, -1.0, 0.0), Vec3::new(1.0, 1.0, 2.0)));
    let bbox = [p.x + lo.x, p.y + lo.y, p.z + lo.z, p.x + hi.x, p.y + hi.y, p.z + hi.z];
    let sector = file.sector_at(p);
    let mut rec = file.records[r].bytes.clone();
    rec[55..57].copy_from_slice(&group.to_le_bytes());
    set_vegetation(&mut rec, &v, &bbox, file.sector_origin(sector));
    file.records.push(aa_world::placement_write::Record { sector, bytes: rec, deleted: false });
    let src = Source::VegDat(file.records.len() - 1);
    let n = cell.display.vegetation.len();
    cell.respawn.insert(sector64(p));
    cell.veg_orig.push((v.mat4(), bbox));
    cell.display.vegetation.push(v);
    cell.veg_src.push(src);
    cell.unsaved.insert(File::Vegetation);
    cell.uncommitted = true;
    cell.edited = true;
    Ok(n as u32)
}

/// The ghost of the model being placed follows the cursor.
fn ghost(mut commands: Commands, editor: Res<Editor>, cursor: Res<Cursor>, mut extra: ResMut<Extra>, mut cry: CryAssets, mut q: Query<&mut Transform>) {
    let want = match (&extra.placing, editor.mode == Mode::Objects) {
        (Some(Placing::Model(m)), true) => Some(m.clone()),
        (Some(Placing::Veg(_, m)), true) => Some(m.clone()),
        _ => None,
    };
    if extra.ghost.as_ref().map(|g| &g.0) != want.as_ref() {
        if let Some((_, e)) = extra.ghost.take() {
            commands.entity(e).despawn();
        }
        if let Some(m) = want {
            let options = ModelOptions::default();
            match cry.load_model(&m, &options) {
                Ok(loaded) => {
                    let model = spawn_loaded(&mut commands, &m, &loaded, &options);
                    let root = commands.spawn((Name::new("placement ghost"), Transform::default(), Visibility::default())).add_child(model).id();
                    extra.ghost = Some((m, root));
                }
                Err(e) => {
                    extra.status = format!("{m}: {e}");
                    extra.placing = None;
                }
            }
        }
    }
    if let (Some((_, e)), Some(p)) = (&extra.ghost, cursor.ground)
        && let Ok(mut t) = q.get_mut(*e)
    {
        *t = Transform::from_translation(cry_to_bevy(p)).with_rotation(Quat::from_rotation_y(extra.yaw.to_radians()));
    }
}

#[allow(clippy::too_many_arguments)]
fn place(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut editor: ResMut<Editor>,
    cursor: Res<Cursor>,
    state: Res<ObjectState>,
    mut objs: ResMut<WorldObjects>,
    mut extra: ResMut<Extra>,
) {
    if extra.placing.is_none() || editor.mode != Mode::Objects {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        extra.placing = None;
        return;
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        extra.yaw -= 15.0;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        extra.yaw += 15.0;
    }
    if !mouse.just_released(MouseButton::Left) || cursor.over_ui {
        return;
    }
    let Some(p) = cursor.ground else { return };
    let placing = extra.placing.clone().unwrap();
    let r = place_at(&editor, &state, &mut objs, &mut extra, &placing, p);
    match r {
        Ok(msg) => {
            if !keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
                extra.placing = None;
            }
            editor.status(msg);
        }
        Err(e) => editor.status(format!("place: {e}")),
    }
}

fn place_at(editor: &Editor, state: &ObjectState, objs: &mut WorldObjects, extra: &mut Extra, placing: &Placing, p: Vec3) -> Result<String, String> {
    let c = cell_of(p);
    let local = p - cell_origin(c.0, c.1);
    let (world, overlay) = (editor.world.clone(), editor.paths.overlay.clone());
    let cell = objs.ensure(&world, state, c).ok_or("the cell's objects are not loaded yet")?;
    let (kind, index, what) = match placing {
        Placing::Model(m) => (ObjectKind::Brush, place_brush(&world, &overlay, cell, m, local, extra.yaw)?, m.clone()),
        Placing::Veg(g, m) => (ObjectKind::Vegetation, place_vegetation(cell, *g, m, local)?, m.clone()),
    };
    extra.select = Some(ObjectPart { cell: c, kind, index });
    Ok(format!("placed {what} in cell {:03}_{:03} (Ctrl+S saves)", c.0, c.1))
}

fn select_new(mut extra: ResMut<Extra>, mut objs: ResMut<WorldObjects>) {
    if let Some(p) = extra.select.take() {
        objs.selected = Some(p);
    }
}

/// `--place-model <path> --place-at x,y[,z]` places one brush (`veg:<group>`: a vegetation instance) once the cell is
/// loaded; `--extra-move kind[#n]:dx,dy,dz` moves the n-th object of a kind (decal / road / water / interior) of the
/// cell under the camera (`kind[#n]:` with no numbers deletes it); `--objects-save` saves the files afterwards.
fn scripted(
    time: Res<Time>,
    mut editor: ResMut<Editor>,
    state: Res<ObjectState>,
    query: Res<aa_world::TerrainQuery>,
    mut objs: ResMut<WorldObjects>,
    mut extra: ResMut<Extra>,
    mut done: Local<(bool, bool)>,
) {
    if time.elapsed_secs() < 8.0 {
        return;
    }
    let vec = |name: &str| -> Option<Vec<f32>> { Some(crate::arg(name)?.split(',').filter_map(|s| s.trim().parse().ok()).collect()) };
    if !done.0
        && let (Some(model), Some(v)) = (crate::arg("--place-model"), vec("--place-at"))
        && v.len() >= 2
    {
        let z = v.get(2).copied().or_else(|| query.height_at(v[0], v[1]));
        if let Some(z) = z {
            done.0 = true;
            let placing = if let Some(g) = model.strip_prefix("veg:").and_then(|g| g.parse::<u16>().ok()) {
                let m = state.vegetation_groups().and_then(|vg| vg.groups.get(&g)).map(|g| g.model.clone()).unwrap_or_default();
                Placing::Veg(g, m)
            } else {
                Placing::Model(model.clone())
            };
            let r = place_at(&editor, &state, &mut objs, &mut extra, &placing, Vec3::new(v[0], v[1], z));
            let msg = r.unwrap_or_else(|e| format!("place failed: {e}"));
            info!("objects_extra: {msg}");
            editor.status(msg);
            if crate::flag("--objects-save") {
                editor.requests.push(Request::SaveObjects);
            }
        }
    }
    if !done.1
        && let Some(spec) = crate::arg("--extra-move")
        && let Some((k, d)) = spec.split_once(':')
        && let Some(c) = extra.cell
    {
        let (k, nth) = match k.split_once('#') {
            Some((k, n)) => (k, n.parse::<usize>().unwrap_or(0)),
            None => (k, 0),
        };
        let kind = match k {
            "decal" => ExtraKind::Decal,
            "road" => ExtraKind::Road,
            "water" => ExtraKind::Water,
            _ => ExtraKind::Interior,
        };
        let d: Vec<f32> = d.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        let world = editor.world.clone();
        let objs = &mut *objs;
        let Some(cell) = objs.ensure(&world, &state, c) else { return };
        if kind == ExtraKind::Interior
            && !extra.interiors.contains_key(&c)
            && let Some(int) = load_interiors(&world, cell)
        {
            extra.interiors.insert(c, int);
        }
        done.1 = true;
        let list = items(&extra, cell, kind);
        let delete = d.is_empty();
        let msg = match list.get(nth) {
            Some(&i) if delete || d.len() == 3 => {
                extra.selected = Some((kind, i));
                let delta = (!delete).then(|| Vec3::new(d[0], d[1], d[2]));
                edit(&mut extra, cell, kind, i, delta);
                format!("{} #{i} of cell {c:?} {}", kind.label(), if delete { "deleted".to_string() } else { format!("moved by {delta:?}") })
            }
            _ => format!("no {} in cell {c:?}", kind.label()),
        };
        info!("objects_extra: {msg}");
        editor.status(msg);
        if crate::flag("--objects-save") {
            editor.requests.push(Request::SaveObjects);
        }
    }
}

/// Outlines of the current cell's decals, roads, water and interior brushes (the selection white).
fn draw(editor: Res<Editor>, objs: Res<WorldObjects>, extra: Res<Extra>, mut gizmos: Gizmos) {
    if editor.mode != Mode::Objects || !extra.open {
        return;
    }
    let Some(c) = extra.cell else { return };
    let Some(cell) = objs.cells.get(&c) else { return };
    let o = cell_origin(c.0, c.1);
    let w = |p: Vec3| cry_to_bevy(o + p);
    for kind in ExtraKind::ALL {
        for i in items(&extra, cell, kind) {
            let sel = extra.selected == Some((kind, i));
            let colour = if sel { Color::WHITE } else { kind.colour() };
            let bytes = match kind {
                ExtraKind::Interior => {
                    let Some(int) = extra.interiors.get(&c) else { continue };
                    let Some(it) = int.file.item(int.display[i].0) else { continue };
                    &it.bytes
                }
                _ => &cell.octree.as_ref().unwrap().items[i].bytes,
            };
            let pts = outline(kind.item(), bytes);
            match kind {
                ExtraKind::Road if pts.len() >= 2 => {
                    // left / right edges in pairs
                    for pair in pts.as_chunks::<2>().0.windows(2) {
                        gizmos.line(w(pair[0][0]), w(pair[1][0]), colour);
                        gizmos.line(w(pair[0][1]), w(pair[1][1]), colour);
                    }
                }
                ExtraKind::Water if pts.len() >= 3 => {
                    for k in 0..pts.len() {
                        gizmos.line(w(pts[k]), w(pts[(k + 1) % pts.len()]), colour);
                    }
                }
                _ => {
                    let (lo, hi) = bbox(bytes);
                    if (hi - lo).max_element() > 300.0 && !sel {
                        continue;
                    }
                    let corner = |k: u32| Vec3::new(if k & 1 == 0 { lo.x } else { hi.x }, if k & 2 == 0 { lo.y } else { hi.y }, if k & 4 == 0 { lo.z } else { hi.z });
                    for (a, b) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
                        gizmos.line(w(corner(a)), w(corner(b)), colour);
                    }
                }
            }
        }
    }
}

/// Saves edited `visareas.dat` files on Ctrl+S (before the Objects tools take the request).
fn save_interiors(mut editor: ResMut<Editor>, mut extra: ResMut<Extra>) {
    if !editor.requests.iter().any(|r| matches!(r, Request::SaveObjects)) {
        return;
    }
    let (world, overlay) = (editor.world.clone(), editor.paths.overlay.clone());
    let mut msgs = Vec::new();
    for (c, int) in extra.interiors.iter_mut().filter(|(_, i)| i.unsaved) {
        let path = overlay.join(aa_world::world::cell_dir(&world, c.0, c.1)).join("client/visareas.dat");
        let r = std::fs::create_dir_all(path.parent().unwrap()).and_then(|_| std::fs::write(&path, int.file.write()));
        match r {
            Ok(()) => {
                int.unsaved = false;
                info!("objects_extra: wrote {}", path.display());
            }
            Err(e) => msgs.push(format!("{}: {e}", path.display())),
        }
    }
    if !msgs.is_empty() {
        editor.status(format!("visareas save failed: {}", msgs.join("; ")));
    }
}

/// "Reload objects": every loaded cell's objects are dropped and read again from the files (pak plus overlay).
fn reload_objects(world: &mut World) {
    let Some(mut extra) = world.get_resource_mut::<Extra>() else { return };
    if !std::mem::take(&mut extra.reload) {
        return;
    }
    world.resource_scope(|world, mut state: Mut<ObjectState>| state.unload_all(world));
    info!("objects_extra: objects reloaded from the files");
}

#[allow(clippy::too_many_arguments)]
fn window(
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    state: Res<ObjectState>,
    mut objs: ResMut<WorldObjects>,
    mut extra: ResMut<Extra>,
) -> Result {
    if editor.mode != Mode::Objects || editor.playing {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let mut open = extra.open;
    if !open {
        egui::Area::new(egui::Id::new("more objects button")).anchor(egui::Align2::CENTER_BOTTOM, [0.0, -36.0]).show(ctx, |ui| {
            if ui.button("More objects: new models, decals, roads, water, interiors").clicked() {
                open = true;
            }
        });
        extra.open = open;
        return Ok(());
    }
    let world = editor.world.clone();
    let extra = &mut *extra;
    let objs = &mut *objs;
    egui::Window::new("More objects").open(&mut open).default_pos([880.0, 120.0]).default_width(380.0).show(ctx, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut extra.tab, Tab::New, "New model");
            ui.selectable_value(&mut extra.tab, Tab::Vegetation, "New vegetation");
            for k in ExtraKind::ALL {
                ui.selectable_value(&mut extra.tab, Tab::Kind(k), k.label());
            }
        });
        ui.separator();
        match extra.tab {
            Tab::New => {
                let list = extra.models.get_or_insert_with(model_list);
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.text_edit_singleline(&mut extra.filter);
                });
                let f = extra.filter.to_ascii_lowercase();
                let shown: Vec<&String> = list.iter().filter(|m| f.is_empty() || m.contains(&f)).take(400).collect();
                ui.small(format!("{} models ({} shown)", list.len(), shown.len()));
                let mut pick = None;
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for m in shown {
                        let name = m.strip_prefix("game/objects/").unwrap_or(m);
                        let on = extra.placing == Some(Placing::Model(m.clone()));
                        if ui.selectable_label(on, name).clicked() {
                            pick = Some(m.clone());
                        }
                    }
                });
                if let Some(m) = pick {
                    extra.placing = Some(Placing::Model(m));
                }
                ui.small("Pick a model, then click the ground to place it ([ ] turn, Shift keeps placing, Esc stops). The new brush is a normal world object afterwards: drag, turn, delete, Ctrl+S.");
            }
            Tab::Vegetation => {
                let Some(groups) = state.vegetation_groups() else {
                    ui.label("vegetation groups not loaded yet");
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label("Search");
                    ui.text_edit_singleline(&mut extra.filter);
                });
                let f = extra.filter.to_ascii_lowercase();
                let mut list: Vec<(u16, String)> = groups.groups.iter().map(|(id, g)| (*id, g.model.clone())).filter(|(_, m)| f.is_empty() || m.to_ascii_lowercase().contains(&f)).collect();
                list.sort();
                let mut pick = None;
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for (id, m) in list.iter().take(400) {
                        let on = extra.placing == Some(Placing::Veg(*id, m.clone()));
                        if ui.selectable_label(on, format!("{id}  {}", m.rsplit('/').next().unwrap_or(m))).on_hover_text(m.as_str()).clicked() {
                            pick = Some(Placing::Veg(*id, m.clone()));
                        }
                    }
                });
                if pick.is_some() {
                    extra.placing = pick;
                }
                ui.small("Vegetation is added to the cell's vegetation.dat (cells without one cannot take new instances).");
            }
            Tab::Kind(kind) => {
                let Some(c) = extra.cell else {
                    ui.label("point at the ground");
                    return;
                };
                let Some(cell) = objs.ensure(&world, &state, c) else {
                    ui.label(format!("cell {:03}_{:03}: objects not loaded", c.0, c.1));
                    return;
                };
                if kind == ExtraKind::Interior && !extra.interiors.contains_key(&c) {
                    match load_interiors(&world, cell) {
                        Some(int) => {
                            extra.interiors.insert(c, int);
                        }
                        None => {
                            ui.label(format!("cell {:03}_{:03} has no (readable) visareas.dat", c.0, c.1));
                            return;
                        }
                    }
                }
                let list = items(extra, cell, kind);
                ui.label(format!("cell {:03}_{:03}: {} {}", c.0, c.1, list.len(), kind.label().to_lowercase()));
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for &i in &list {
                        let p = center(extra, cell, kind, i).unwrap_or_default() + cell_origin(c.0, c.1);
                        let name = match kind {
                            ExtraKind::Interior => extra
                                .interiors
                                .get(&c)
                                .and_then(|int| int.file.item(int.display[i].0))
                                .and_then(|it| extra.interiors[&c].file.models.get(u32_at(&it.bytes, 123) as usize))
                                .map(|m| m.rsplit('/').next().unwrap_or(m).to_string())
                                .unwrap_or_default(),
                            _ => format!("item {i}"),
                        };
                        let on = extra.selected == Some((kind, i));
                        if ui.selectable_label(on, format!("{name}  at {:.0}, {:.0}, {:.0}", p.x, p.y, p.z)).clicked() {
                            extra.selected = Some((kind, i));
                            editor.requests.push(Request::Focus(p));
                        }
                    }
                });
                if let Some((k, i)) = extra.selected.filter(|(k, _)| *k == kind) {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Step (m)");
                        ui.add(egui::DragValue::new(&mut extra.step).range(0.05..=100.0).speed(0.1));
                    });
                    let s = extra.step;
                    let mut delta = None;
                    ui.horizontal(|ui| {
                        for (label, d) in [("-X", Vec3::NEG_X), ("+X", Vec3::X), ("-Y", Vec3::NEG_Y), ("+Y", Vec3::Y), ("-Z", Vec3::NEG_Z), ("+Z", Vec3::Z)] {
                            if ui.button(label).clicked() {
                                delta = Some(d * s);
                            }
                        }
                    });
                    if let Some(d) = delta {
                        edit(extra, cell, k, i, Some(d));
                    }
                    if ui.button("Delete").clicked() {
                        edit(extra, cell, k, i, None);
                        extra.selected = None;
                    }
                    if k != ExtraKind::Interior {
                        ui.small("The outline moves now; the drawn geometry after Ctrl+S and \"Reload objects\".");
                    }
                }
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Save (Ctrl+S)").clicked() {
                editor.requests.push(Request::SaveObjects);
            }
            if ui.button("Reload objects").on_hover_text("Drops every loaded cell's objects and reads them again from the files (pak + overlay). Save first.").clicked() {
                extra.reload = true;
            }
        });
        if !extra.status.is_empty() {
            ui.small(extra.status.clone());
        }
    });
    extra.open = open;
    if !open {
        extra.placing = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pak() -> bool {
        std::path::Path::new(aa_pak::DEFAULT_PAK).exists() || std::env::var_os("AA_PAK").is_some()
    }

    #[test]
    fn model_tables_append_and_reuse() {
        let mut t = 1u32.to_le_bytes().to_vec();
        let mut e = vec![0u8; 260];
        e[4..9].copy_from_slice(b"a.cgf");
        t.extend(e);
        t.extend(7u32.to_le_bytes()); // the material table after it stays
        let (t2, i) = add_model(&t[..264], 260, "b.cgf").unwrap();
        assert_eq!(i, 1);
        assert_eq!(u32_at(&t2, 0), 2);
        assert_eq!(&t2[4 + 260 + 4..4 + 260 + 9], b"b.cgf");
        assert_eq!(add_model(&t2, 260, "A.CGF").unwrap().1, 0);
        let (s, i) = add_model(&0u32.to_le_bytes(), 256, "x.cgf").unwrap();
        assert_eq!((s.len(), i), (260, 0));
    }

    #[test]
    fn translate_road_and_water() {
        let mut road = vec![0u8; 63 + 24];
        put_u32(&mut road, 39, 2);
        put_f32(&mut road, 63, 5.0);
        translate_item(13, &mut road, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(f32_at(&road, 63), 6.0);
        assert_eq!(f32_at(&road, 63 + 12 + 8), 3.0);
        assert_eq!(f32_at(&road, 12), 1.0);
        let mut water = vec![0u8; 119 + 36];
        put_f32(&mut water, 79, 1.0);
        put_f32(&mut water, 83, -10.0);
        put_u32(&mut water, 103, 2);
        put_u32(&mut water, 115, 1);
        translate_item(11, &mut water, Vec3::new(0.0, 0.0, 2.0));
        assert_eq!(f32_at(&water, 83), -12.0, "surface 10 -> 12");
        assert_eq!(f32_at(&water, 119 + 24 + 8), 2.0, "the physics point moves too");
    }

    /// Every `visareas.dat` of a sample of main_world cells parses and writes back byte for byte; deleting a brush
    /// keeps the file readable by the client reader with one brush less.
    #[test]
    fn visareas_round_trip() {
        if !pak() {
            return;
        }
        let pak = aa_pak::Pak::global();
        let names: Vec<String> = pak.names().filter(|n| n.starts_with("game/worlds/main_world/cells/") && n.ends_with("/visareas.dat")).map(str::to_string).collect();
        let mut checked = 0;
        let mut edited = false;
        for n in names.iter().step_by(25) {
            let data = pak.read(n).unwrap();
            let Ok(parsed) = aa_world::placement::parse_visareas_dat(&data) else { continue };
            let v = Visareas::parse(&data).unwrap_or_else(|| panic!("{n}"));
            assert_eq!(v.write(), data, "{n}");
            checked += 1;
            if !edited && let Some(&key) = v.brushes().first() {
                let mut v2 = v.clone();
                v2.item_mut(key).unwrap().deleted = true;
                let out = v2.write();
                let again = aa_world::placement::parse_visareas_dat(&out).expect("readable after a delete");
                assert_eq!(again.brushes.len() + 1, parsed.brushes.len(), "{n}");
                edited = true;
            }
        }
        assert!(checked > 20, "{checked}");
        assert!(edited);
    }

    /// `AA_EDITOR_LIST_INTERIORS=world/x_y`: a cell's interior brushes with model and position (manual check).
    #[test]
    fn list_interiors() {
        let Some(spec) = std::env::var("AA_EDITOR_LIST_INTERIORS").ok() else { return };
        let (world, xy) = spec.split_once('/').unwrap();
        let (x, y) = xy.split_once('_').unwrap();
        let dir = aa_world::world::cell_dir(world, x.parse().unwrap(), y.parse().unwrap());
        let v = Visareas::parse(&aa_pak::Pak::global().read(&format!("{dir}/client/visareas.dat")).unwrap()).unwrap();
        for (n, key) in v.brushes().into_iter().enumerate() {
            let b = &v.item(key).unwrap().bytes;
            let t = Vec3::new(f32_at(b, 79), f32_at(b, 95), f32_at(b, 111));
            println!("#{n} {t:?} {}", v.models.get(u32_at(b, 123) as usize).map(String::as_str).unwrap_or("?"));
        }
    }

    /// Worlds other than main_world: every `object.dat` and `visareas.dat` of a few instances writes back byte-identical.
    #[test]
    fn other_worlds_round_trip() {
        if !pak() {
            return;
        }
        let pak = aa_pak::Pak::global();
        let worlds = ["instance_library", "instance_prologue", "instance_sal_temple", "arche_mall_world"];
        let mut checked = 0;
        for n in pak.names().filter(|n| worlds.iter().any(|w| n.starts_with(&format!("game/worlds/{w}/cells/")))) {
            let data = if n.ends_with("/object.dat") || n.ends_with("/visareas.dat") { pak.read(n).unwrap() } else { continue };
            if n.ends_with("/object.dat") {
                let tree = ObjectTree::parse(&data).unwrap_or_else(|| panic!("{n}"));
                assert_eq!(tree.write(), data, "{n}");
            } else {
                if aa_world::placement::parse_visareas_dat(&data).is_err() {
                    continue;
                }
                assert_eq!(Visareas::parse(&data).unwrap_or_else(|| panic!("{n}")).write(), data, "{n}");
            }
            checked += 1;
        }
        assert!(checked > 40, "{checked}");
    }

    /// `object.dat` decals / roads / water move and delete through the octree: the reader sees the moved values.
    #[test]
    fn octree_extras_move_and_delete() {
        if !pak() {
            return;
        }
        let pak = aa_pak::Pak::global();
        let data = pak.read("game/worlds/main_world/cells/015_014/client/object.dat").unwrap();
        let before = aa_world::placement::parse_object_dat(&data).unwrap();
        let mut tree = ObjectTree::parse(&data).unwrap();
        assert_eq!(tree.write(), data);
        let d = Vec3::new(3.0, -2.0, 1.0);
        let mut touched = 0;
        for kind in [9u32, 13, 11] {
            if let Some(&i) = tree.items_of(kind).first() {
                let it = &mut tree.items[i];
                translate_item(it.kind, &mut it.bytes, d);
                touched += 1;
            }
        }
        let after = aa_world::placement::parse_object_dat(&tree.write()).unwrap();
        if let (Some(a), Some(b)) = (before.decals.first(), after.decals.first()) {
            assert!((b.pos - a.pos - d).length() < 1e-4);
        }
        if let (Some(a), Some(b)) = (before.roads.first(), after.roads.first()) {
            assert!((b.points[0] - a.points[0] - d).length() < 1e-4);
        }
        if let (Some(a), Some(b)) = (before.water.first(), after.water.first()) {
            assert!((b.surface - a.surface - d.z).abs() < 1e-3, "{} {}", a.surface, b.surface);
            assert!((b.points[0] - a.points[0] - d).length() < 1e-4);
        }
        assert!(touched > 0);
        if let Some(&i) = tree.items_of(9).first() {
            tree.items[i].deleted = true;
            let again = aa_world::placement::parse_object_dat(&tree.write()).unwrap();
            assert_eq!(again.decals.len() + 1, after.decals.len());
        }
    }
}

#[cfg(test)]
mod probe {
    /// `AA_EDITOR_PROBE_MODEL=<cgf>`: prints the model's bounds (manual check).
    #[test]
    fn probe_model() {
        let Some(m) = std::env::var("AA_EDITOR_PROBE_MODEL").ok() else { return };
        let model = aa_cry::mesh::Model::load(&m);
        println!("{m}: {:?}", model.as_ref().map(|x| (x.bounds(), x.meshes.len(), x.nodes.len())).map_err(|e| e.to_string()));
    }
}

#[cfg(test)]
mod count {
    /// `AA_EDITOR_COUNT_EXTRAS=1`: decals / roads / water per main_world cell near Nuia (manual check).
    #[test]
    fn count_extras() {
        if std::env::var_os("AA_EDITOR_COUNT_EXTRAS").is_none() {
            return;
        }
        for (x, y) in [(15, 14), (15, 15), (14, 14), (16, 14), (14, 15), (16, 15)] {
            let d = aa_pak::Pak::global().read(&format!("game/worlds/main_world/cells/{x:03}_{y:03}/client/object.dat")).unwrap();
            let t = super::ObjectTree::parse(&d).unwrap();
            let roads: Vec<_> = t.items_of(13).iter().map(|&i| { let (lo, hi) = super::bbox(&t.items[i].bytes); (lo + hi) / 2.0 }).take(3).collect();
            let water: Vec<_> = t.items_of(11).iter().map(|&i| { let (lo, hi) = super::bbox(&t.items[i].bytes); (lo + hi) / 2.0 }).take(3).collect();
            println!("{x}_{y}: decals {} roads {} {roads:?} water {} {water:?}", t.items_of(9).len(), t.items_of(13).len(), t.items_of(11).len());
        }
    }
}
