//! One cell's editable static objects: its placement files as record models ([`aa_world::placement_write`]) and
//! the display copy of its [`CellObjects`] that the streamer draws.
//!
//! Display indices never shift: a deleted object stays in the lists (left out of the drawing by the `removed`
//! set), a copy is appended. Each display index maps to the record it came from ([`Source`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aa_world::entities::PlacedProp;
use aa_world::objects::edit::ObjectKind;
use aa_world::placement::{Brush, CellObjects, Vegetation};
use aa_world::placement_write::{ObjectTree, Record, SectorFile, entities_xml, record_bbox, set_brush, set_vegetation, transform_bbox};
use bevy::prelude::*;

/// Where an object's record lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Record index in `brush.dat`.
    BrushDat(usize),
    /// Item index in the `object.dat` tree (brush or vegetation).
    Octree(usize),
    VegDat(usize),
    /// `vegetation/SX_SY.dat` (`sx * 4 + sy`) and record index.
    VegSector(usize, usize),
    /// `entities.xml` entity by `EntityGuid`.
    Entity(u64),
    /// Not editable here (interior `visareas.dat` objects, entities without a unique guid).
    ReadOnly,
}

/// The files a cell can save.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum File {
    Brush,
    Object,
    Vegetation,
    VegSector(usize),
    Entities,
}

impl File {
    pub fn name(self) -> String {
        match self {
            File::Brush => "brush.dat".into(),
            File::Object => "object.dat".into(),
            File::Vegetation => "vegetation.dat".into(),
            File::VegSector(i) => format!("vegetation/{:02}_{:02}.dat", i / 4, i % 4),
            File::Entities => "entities.xml".into(),
        }
    }
}

/// An object's display data.
#[derive(Clone, Debug)]
pub enum Item {
    Brush(Brush),
    Veg(Vegetation),
    Entity(PlacedProp),
}

impl Item {
    /// Cell-local cry-space object matrix.
    pub fn matrix(&self) -> Mat4 {
        match self {
            Item::Brush(b) => b.mat4(),
            Item::Veg(v) => v.mat4(),
            Item::Entity(p) => p.matrix,
        }
    }

    pub fn position(&self) -> Vec3 {
        match self {
            Item::Brush(b) => b.translation(),
            Item::Veg(v) => v.pos,
            Item::Entity(p) => p.translation(),
        }
    }

    /// Turn about Z (degrees, counter-clockwise from +X), and uniform scale.
    pub fn yaw_scale(&self) -> (f32, f32) {
        match self {
            Item::Veg(v) => (v.angle_z as f32 * 360.0 / 256.0, v.scale),
            _ => {
                let m = self.matrix();
                let x = m.x_axis.truncate();
                (x.y.atan2(x.x).to_degrees(), x.length())
            }
        }
    }
}

/// The complete state of one object: display data, its file record(s), alive or deleted.
#[derive(Clone, Debug)]
pub struct State {
    pub alive: bool,
    pub item: Item,
    /// Sector and bytes of a binary record.
    pub rec: Option<(u16, Vec<u8>)>,
    /// The `object.dat` copy of a `brush.dat` brush.
    pub twin: Option<Vec<u8>>,
    /// `entities.xml` text in this state.
    pub xml: Option<Arc<String>>,
}

/// A change of one object, for undo.
#[derive(Clone, Debug)]
pub struct Change {
    pub kind: ObjectKind,
    pub index: u32,
    pub before: State,
    pub after: State,
}

pub struct EditCell {
    pub cell: (i32, i32),
    pub display: CellObjects,
    pub removed: HashSet<(ObjectKind, u32)>,
    pub brush_src: Vec<Source>,
    pub veg_src: Vec<Source>,
    pub entity_src: Vec<Source>,
    /// Matrix and box each brush / vegetation instance was loaded with (copies: their source's), so boxes are not
    /// grown by repeated turns.
    pub brush_orig: Vec<(Mat4, [f32; 6])>,
    pub veg_orig: Vec<(Mat4, [f32; 6])>,
    pub brush_dat: Option<SectorFile>,
    pub octree: Option<ObjectTree>,
    pub veg_dat: Option<SectorFile>,
    pub veg_sectors: Vec<Option<SectorFile>>,
    pub entities: Option<Arc<String>>,
    /// `brush.dat` record -> its `object.dat` copy.
    pub twins: HashMap<usize, usize>,
    pub unsaved: HashSet<File>,
    /// 64 m sectors to rebuild on the next commit.
    pub respawn: HashSet<u8>,
    /// The display data differs from what the streamer draws.
    pub uncommitted: bool,
    /// Changed since loaded (a reload of the cell by the streamer must get the edited copy back).
    pub edited: bool,
}

fn sector64(p: Vec3) -> u8 {
    let sx = (p.x / 64.0).floor().clamp(0.0, 15.0) as u8;
    let sy = (p.y / 64.0).floor().clamp(0.0, 15.0) as u8;
    sx * 16 + sy
}

impl EditCell {
    /// Reads the cell's files (pak plus overlay) and maps them onto `loaded`, the objects the streamer drew from
    /// the same files.
    pub fn load(world: &str, cell: (i32, i32), loaded: &CellObjects) -> Self {
        let dir = aa_world::world::cell_dir(world, cell.0, cell.1);
        let pak = aa_pak::Pak::global();
        let read = |f: &str| pak.read(&format!("{dir}/client/{f}")).ok();
        let object_data = read("object.dat");
        // the octree counts only when the client reader accepts the file
        let octree_parsed = object_data.as_deref().and_then(|d| aa_world::placement::parse_object_dat(d).ok());
        let octree = object_data.as_deref().filter(|_| octree_parsed.is_some()).and_then(ObjectTree::parse);
        let mut c = EditCell {
            cell,
            display: loaded.clone(),
            removed: HashSet::new(),
            brush_src: Vec::new(),
            veg_src: Vec::new(),
            entity_src: Vec::new(),
            brush_orig: Vec::new(),
            veg_orig: Vec::new(),
            brush_dat: None,
            octree,
            veg_dat: None,
            veg_sectors: vec![None; 16],
            entities: None,
            twins: HashMap::new(),
            unsaved: HashSet::new(),
            respawn: HashSet::new(),
            uncommitted: false,
            edited: false,
        };

        // brushes: brush.dat (with statobjs.dat), else the octree's
        let brush_data = read("brush.dat").filter(|d| aa_world::placement::parse_brush_dat(d).is_ok());
        let (brush_file, statobjs) = (brush_data.as_deref().and_then(SectorFile::parse_brushes), read("statobjs.dat"));
        if let (Some(file), Some(_)) = (brush_file, statobjs) {
            let n = file.records.len();
            c.brush_src.extend((0..n).map(Source::BrushDat));
            // the octree copies, matched by model and placement
            if let (Some(tree), Some(parsed)) = (&c.octree, &octree_parsed) {
                let mut by_key: HashMap<(String, [i64; 3]), Vec<usize>> = HashMap::new();
                let key = |m: &str, t: Vec3| (m.to_ascii_lowercase(), [(t.x * 100.0).round() as i64, (t.y * 100.0).round() as i64, (t.z * 100.0).round() as i64]);
                for (item, b) in tree.items_of(1).into_iter().zip(&parsed.brushes) {
                    if let Some(m) = parsed.models.get(b.model as usize) {
                        by_key.entry(key(m, b.translation())).or_default().push(item);
                    }
                }
                for (r, b) in loaded.brushes.iter().take(n).enumerate() {
                    let Some(m) = loaded.models.get(b.model as usize) else { continue };
                    if let Some(items) = by_key.get_mut(&key(m, b.translation()))
                        && let Some(i) = items.iter().position(|&it| {
                            let tb = &tree.items[it].bytes;
                            (0..12).all(|k| (aa_world::reader::f32_at(tb, 67 + k * 4) - b.matrix[k]).abs() < 1e-3 || k % 4 == 3)
                        })
                    {
                        c.twins.insert(r, items.remove(i));
                    }
                }
            }
            c.brush_dat = Some(file);
        } else if let Some(tree) = &c.octree {
            c.brush_src.extend(tree.items_of(1).into_iter().map(Source::Octree));
        }
        if c.brush_src.len() > loaded.brushes.len() {
            c.brush_src.clear(); // the files changed under us: nothing editable
        }
        c.brush_src.resize(loaded.brushes.len(), Source::ReadOnly);

        // vegetation: vegetation.dat, the octree's, then the dense-forest sector files
        let mut ok = true;
        if let Some(d) = read("vegetation.dat").filter(|d| aa_world::placement::parse_vegetation_dat(d).is_ok()) {
            match SectorFile::parse_vegetation(&d) {
                Some(f) => {
                    c.veg_src.extend((0..f.records.len()).map(Source::VegDat));
                    c.veg_dat = Some(f);
                }
                None => ok = false,
            }
        }
        if ok && let Some(tree) = &c.octree {
            c.veg_src.extend(tree.items_of(2).into_iter().map(Source::Octree));
        }
        for i in 0..16 {
            let Some(d) = read(&format!("vegetation/{:02}_{:02}.dat", i / 4, i % 4)) else { continue };
            if aa_world::placement::parse_vegetation_sector(&d, i / 4, i % 4).is_empty() {
                continue;
            }
            match SectorFile::parse_vegetation_sector(&d, i / 4, i % 4) {
                Some(f) if ok => {
                    c.veg_src.extend((0..f.records.len()).map(|r| Source::VegSector(i, r)));
                    c.veg_sectors[i] = Some(f);
                }
                _ => ok = false,
            }
        }
        if !ok || c.veg_src.len() > loaded.vegetation.len() {
            c.veg_src.clear();
        }
        c.veg_src.resize(loaded.vegetation.len(), Source::ReadOnly);

        // entities by unique guid
        if let Some(xml) = read("entities.xml").and_then(|d| String::from_utf8(d).ok()) {
            let mut count: HashMap<u64, usize> = HashMap::new();
            for p in &loaded.entity_props {
                *count.entry(p.guid).or_default() += 1;
            }
            c.entity_src = loaded
                .entity_props
                .iter()
                .map(|p| if p.guid != 0 && count[&p.guid] == 1 && entities_xml::local(&xml, p.guid).is_some() { Source::Entity(p.guid) } else { Source::ReadOnly })
                .collect();
            c.entities = Some(Arc::new(xml));
        }
        c.entity_src.resize(loaded.entity_props.len(), Source::ReadOnly);

        // boxes as loaded
        c.brush_orig = loaded.brushes.iter().map(|b| (b.mat4(), b.bbox)).collect();
        for (i, v) in loaded.vegetation.iter().enumerate() {
            let bbox = c.record(ObjectKind::Vegetation, i as u32).map(|(_, r, origin)| record_bbox(&r, origin)).unwrap_or_else(|| {
                let p = v.pos;
                [p.x - 1.0, p.y - 1.0, p.z, p.x + 1.0, p.y + 1.0, p.z + 2.0]
            });
            c.veg_orig.push((v.mat4(), bbox));
        }
        c
    }

    pub fn source(&self, kind: ObjectKind, index: u32) -> Source {
        let list = match kind {
            ObjectKind::Brush => &self.brush_src,
            ObjectKind::Vegetation => &self.veg_src,
            ObjectKind::Entity => &self.entity_src,
            ObjectKind::Animated => return Source::ReadOnly,
        };
        list.get(index as usize).copied().unwrap_or(Source::ReadOnly)
    }

    pub fn editable(&self, kind: ObjectKind, index: u32) -> bool {
        self.source(kind, index) != Source::ReadOnly
    }

    pub fn alive(&self, kind: ObjectKind, index: u32) -> bool {
        !self.removed.contains(&(kind, index))
    }

    /// Short description of the record's file, for the panel.
    pub fn file_of(&self, kind: ObjectKind, index: u32) -> String {
        match self.source(kind, index) {
            Source::BrushDat(r) => format!("brush.dat #{r}{}", if self.twins.contains_key(&r) { " (+ object.dat copy)" } else { "" }),
            Source::Octree(i) => format!("object.dat item {i}"),
            Source::VegDat(r) => format!("vegetation.dat #{r}"),
            Source::VegSector(f, r) => format!("{} #{r}", File::VegSector(f).name()),
            Source::Entity(g) => format!("entities.xml {g:x}"),
            Source::ReadOnly => "read-only".into(),
        }
    }

    pub fn item(&self, kind: ObjectKind, index: u32) -> Option<Item> {
        let i = index as usize;
        Some(match kind {
            ObjectKind::Brush => Item::Brush(self.display.brushes.get(i)?.clone()),
            ObjectKind::Vegetation => Item::Veg(self.display.vegetation.get(i)?.clone()),
            ObjectKind::Entity => Item::Entity(self.display.entity_props.get(i)?.clone()),
            ObjectKind::Animated => return None,
        })
    }

    /// The model file of an object.
    pub fn model(&self, kind: ObjectKind, index: u32, groups: Option<&aa_world::vegetation::VegGroups>) -> String {
        match self.item(kind, index) {
            Some(Item::Brush(b)) => self.display.models.get(b.model as usize).cloned().unwrap_or_default(),
            Some(Item::Veg(v)) => groups.and_then(|g| g.groups.get(&v.group)).map_or_else(|| format!("vegetation group {}", v.group), |g| g.model.clone()),
            Some(Item::Entity(p)) => format!("{} ({})", p.model, p.name),
            None => String::new(),
        }
    }

    /// The binary record of an object: sector, bytes and the cell-local origin its coordinates are relative to.
    fn record(&self, kind: ObjectKind, index: u32) -> Option<(u16, Vec<u8>, Vec3)> {
        let src = self.source(kind, index);
        let r = match src {
            Source::BrushDat(r) | Source::VegDat(r) | Source::VegSector(_, r) => r,
            Source::Octree(i) => return Some((0, self.octree.as_ref()?.items.get(i)?.bytes.clone(), Vec3::ZERO)),
            _ => return None,
        };
        let f = self.file_for(src)?;
        let rec = f.records.get(r)?;
        Some((rec.sector, rec.bytes.clone(), f.sector_origin(rec.sector)))
    }

    fn file_for(&self, src: Source) -> Option<&SectorFile> {
        match src {
            Source::BrushDat(_) => self.brush_dat.as_ref(),
            Source::VegDat(_) => self.veg_dat.as_ref(),
            Source::VegSector(s, _) => self.veg_sectors[s].as_ref(),
            _ => None,
        }
    }

    /// The object's current state.
    pub fn state(&self, kind: ObjectKind, index: u32) -> Option<State> {
        let item = self.item(kind, index)?;
        let src = self.source(kind, index);
        let rec = self.record(kind, index).map(|(s, b, _)| (s, b));
        let twin = match src {
            Source::BrushDat(r) => self.twins.get(&r).and_then(|&t| self.octree.as_ref().map(|o| o.items[t].bytes.clone())),
            _ => None,
        };
        let xml = matches!(src, Source::Entity(_)).then(|| self.entities.clone()).flatten();
        Some(State { alive: self.alive(kind, index), item, rec, twin, xml })
    }

    /// The state with the object at the cell-local matrix `m` (vegetation: position, yaw steps and scale taken
    /// from it). `None` when the object is not editable.
    pub fn moved(&self, kind: ObjectKind, index: u32, item: Item) -> Option<State> {
        let src = self.source(kind, index);
        if src == Source::ReadOnly {
            return None;
        }
        let mut s = self.state(kind, index)?;
        let i = index as usize;
        match (&item, src) {
            (Item::Brush(b), _) => {
                let mut b = b.clone();
                let (m0, box0) = self.brush_orig[i];
                b.bbox = transform_bbox(&box0, b.mat4() * m0.inverse());
                if let Some((sector, bytes)) = s.rec.as_mut() {
                    let f = self.file_for(src);
                    let new_sector = f.map_or(0, |f| f.sector_at(b.translation()));
                    set_brush(bytes, &b, f.map_or(Vec3::ZERO, |f| f.sector_origin(new_sector)));
                    *sector = new_sector;
                }
                if let Some(t) = s.twin.as_mut() {
                    set_brush(t, &b, Vec3::ZERO);
                }
                s.item = Item::Brush(b);
            }
            (Item::Veg(v), _) => {
                let (m0, box0) = self.veg_orig[i];
                let bbox = transform_bbox(&box0, v.mat4() * m0.inverse());
                if let Some((sector, bytes)) = s.rec.as_mut() {
                    let f = self.file_for(src);
                    let new_sector = f.map_or(0, |f| f.sector_at(v.pos));
                    set_vegetation(bytes, v, &bbox, f.map_or(Vec3::ZERO, |f| f.sector_origin(new_sector)));
                    *sector = new_sector;
                }
                s.item = item.clone();
            }
            (Item::Entity(p), Source::Entity(guid)) => {
                let xml = self.entities.as_ref()?;
                let Item::Entity(old) = self.item(kind, index)? else { return None };
                let (pos, rot, scale) = entities_xml::local(xml, guid)?;
                let local_old = Mat4::from_scale_rotation_translation(scale, rot.normalize(), pos);
                let parent = old.matrix * local_old.inverse();
                let (s2, r2, t2) = (parent.inverse() * p.matrix).to_scale_rotation_translation();
                s.xml = Some(Arc::new(entities_xml::set_transform(xml, guid, t2, r2, s2)?));
                s.item = item.clone();
            }
            _ => return None,
        }
        Some(s)
    }

    /// Puts an object into a state: display, records, removed set.
    pub fn apply(&mut self, kind: ObjectKind, index: u32, s: &State) {
        let i = index as usize;
        let old_pos = self.item(kind, index).map(|it| it.position());
        match &s.item {
            Item::Brush(b) => self.display.brushes[i] = b.clone(),
            Item::Veg(v) => self.display.vegetation[i] = v.clone(),
            Item::Entity(p) => self.display.entity_props[i] = p.clone(),
        }
        if s.alive {
            self.removed.remove(&(kind, index));
        } else {
            self.removed.insert((kind, index));
        }
        let src = self.source(kind, index);
        let set = |f: &mut SectorFile, r: usize| {
            if let Some((sector, bytes)) = &s.rec {
                f.records[r] = Record { sector: *sector, bytes: bytes.clone(), deleted: !s.alive };
            }
        };
        match src {
            Source::BrushDat(r) => {
                if let Some(f) = self.brush_dat.as_mut() {
                    set(f, r);
                    self.unsaved.insert(File::Brush);
                }
                if let (Some(&t), Some(tree), Some(bytes)) = (self.twins.get(&r), self.octree.as_mut(), &s.twin) {
                    tree.items[t].bytes = bytes.clone();
                    tree.items[t].deleted = !s.alive;
                    self.unsaved.insert(File::Object);
                }
            }
            Source::VegDat(r) => {
                if let Some(f) = self.veg_dat.as_mut() {
                    set(f, r);
                    self.unsaved.insert(File::Vegetation);
                }
            }
            Source::VegSector(f_i, r) => {
                if let Some(f) = self.veg_sectors[f_i].as_mut() {
                    set(f, r);
                    self.unsaved.insert(File::VegSector(f_i));
                }
            }
            Source::Octree(t) => {
                if let (Some(tree), Some((_, bytes))) = (self.octree.as_mut(), &s.rec) {
                    tree.items[t].bytes = bytes.clone();
                    tree.items[t].deleted = !s.alive;
                    self.unsaved.insert(File::Object);
                }
            }
            Source::Entity(_) => {
                if let Some(xml) = &s.xml {
                    self.entities = Some(xml.clone());
                    self.unsaved.insert(File::Entities);
                }
            }
            Source::ReadOnly => {}
        }
        self.respawn.insert(sector64(s.item.position()));
        if let Some(p) = old_pos {
            self.respawn.insert(sector64(p));
        }
        self.uncommitted = true;
        self.edited = true;
    }

    /// Appends a copy of an object (not alive yet: apply the returned `after` state). Returns its index and the
    /// `(before, after)` states, `after` moved by `offset`.
    pub fn duplicate(&mut self, kind: ObjectKind, index: u32, offset: Vec3) -> Option<(u32, State, State)> {
        let src = self.source(kind, index);
        let s = self.state(kind, index)?;
        if src == Source::ReadOnly {
            return None;
        }
        let i = index as usize;
        let (new_src, new_index) = match src {
            Source::BrushDat(r) => {
                let f = self.brush_dat.as_mut()?;
                let mut rec = f.records[r].clone();
                rec.deleted = true;
                f.records.push(rec);
                let nr = f.records.len() - 1;
                if let Some(&t) = self.twins.get(&r) {
                    let tree = self.octree.as_mut()?;
                    let bytes = tree.items[t].bytes.clone();
                    let ni = tree.push(1, bytes);
                    tree.items[ni].deleted = true;
                    self.twins.insert(nr, ni);
                }
                (Source::BrushDat(nr), self.display.brushes.len())
            }
            Source::Octree(t) => {
                let tree = self.octree.as_mut()?;
                let item = tree.items[t].clone();
                let ni = tree.push(item.kind, item.bytes);
                tree.items[ni].deleted = true;
                let n = if kind == ObjectKind::Brush { self.display.brushes.len() } else { self.display.vegetation.len() };
                (Source::Octree(ni), n)
            }
            Source::VegDat(r) => {
                let f = self.veg_dat.as_mut()?;
                let mut rec = f.records[r].clone();
                rec.deleted = true;
                f.records.push(rec);
                (Source::VegDat(f.records.len() - 1), self.display.vegetation.len())
            }
            Source::VegSector(fi, r) => {
                let f = self.veg_sectors[fi].as_mut()?;
                let mut rec = f.records[r].clone();
                rec.deleted = true;
                f.records.push(rec);
                (Source::VegSector(fi, f.records.len() - 1), self.display.vegetation.len())
            }
            Source::Entity(guid) => {
                let xml = self.entities.clone()?;
                let used: HashSet<u64> = self.display.entity_props.iter().map(|p| p.guid).collect();
                let mut new_guid = guid ^ 0x5a5a_0000_0000_0000;
                while used.contains(&new_guid) || new_guid == 0 {
                    new_guid = new_guid.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                }
                let copied = Arc::new(entities_xml::copy(&xml, guid, new_guid)?);
                let Item::Entity(mut p) = s.item.clone() else { return None };
                p.guid = new_guid;
                let n = self.display.entity_props.len();
                self.display.entity_props.push(p.clone());
                self.entity_src.push(Source::Entity(new_guid));
                self.removed.insert((kind, n as u32));
                // move the copy inside the copied text, then back to the text without it until applied
                self.entities = Some(copied);
                let mut placed = p.clone();
                placed.matrix = Mat4::from_translation(offset) * p.matrix;
                let after = self.moved(kind, n as u32, Item::Entity(placed));
                self.entities = Some(xml.clone());
                let before = State { alive: false, item: Item::Entity(p), rec: None, twin: None, xml: Some(xml) };
                let mut after = after?;
                after.alive = true;
                return Some((n as u32, before, after));
            }
            Source::ReadOnly => return None,
        };
        // display entries for the copy
        match &s.item {
            Item::Brush(b) => {
                self.display.brushes.push(b.clone());
                self.brush_src.push(new_src);
                self.brush_orig.push(self.brush_orig[i]);
            }
            Item::Veg(v) => {
                self.display.vegetation.push(v.clone());
                self.veg_src.push(new_src);
                self.veg_orig.push(self.veg_orig[i]);
            }
            Item::Entity(_) => return None,
        }
        let n = new_index as u32;
        self.removed.insert((kind, n));
        let before = State { alive: false, ..self.state(kind, n)? };
        let item = match s.item {
            Item::Brush(mut b) => {
                b.matrix[3] += offset.x;
                b.matrix[7] += offset.y;
                b.matrix[11] += offset.z;
                Item::Brush(b)
            }
            Item::Veg(mut v) => {
                v.pos += offset;
                Item::Veg(v)
            }
            Item::Entity(_) => return None,
        };
        let mut after = self.moved(kind, n, item)?;
        after.alive = true;
        Some((n, before, after))
    }

    /// Writes the unsaved files into `overlay` (`<overlay>/<cell dir>/client/<file>`); returns the paths written.
    pub fn save(&mut self, world: &str, overlay: &std::path::Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
        let dir = overlay.join(aa_world::world::cell_dir(world, self.cell.0, self.cell.1)).join("client");
        let mut written = Vec::new();
        let mut files: Vec<File> = self.unsaved.iter().copied().collect();
        files.sort_by_key(|f| f.name());
        for f in files {
            let bytes = match f {
                File::Brush => self.brush_dat.as_ref().map(SectorFile::write),
                File::Object => self.octree.as_ref().map(ObjectTree::write),
                File::Vegetation => self.veg_dat.as_ref().map(SectorFile::write),
                File::VegSector(i) => self.veg_sectors[i].as_ref().map(SectorFile::write),
                File::Entities => self.entities.as_ref().map(|s| s.as_bytes().to_vec()),
            };
            let Some(bytes) = bytes else { continue };
            let path = dir.join(f.name());
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, bytes)?;
            written.push(path);
            self.unsaved.remove(&f);
        }
        Ok(written)
    }
}

/// A copy of `item` turned by `degrees` about Z around its position, scaled by `factor` and moved by `offset`.
pub fn transformed(item: &Item, offset: Vec3, degrees: f32, factor: f32) -> Item {
    match item {
        Item::Veg(v) => {
            let mut v = v.clone();
            v.pos += offset;
            let steps = (degrees / (360.0 / 256.0)).round() as i32;
            v.angle_z = (v.angle_z as i32 + steps).rem_euclid(256) as u8;
            v.scale = (v.scale * factor).clamp(0.1, 5.0);
            Item::Veg(v)
        }
        _ => {
            let m = item.matrix();
            let p = m.w_axis.truncate();
            let rot = Mat4::from_rotation_z(degrees.to_radians()) * Mat4::from_scale(Vec3::splat(factor));
            let new = Mat4::from_translation(p + offset) * rot * Mat4::from_translation(-p) * m;
            match item {
                Item::Brush(b) => {
                    let mut b = b.clone();
                    let c = new.to_cols_array_2d();
                    b.matrix = [c[0][0], c[1][0], c[2][0], c[3][0], c[0][1], c[1][1], c[2][1], c[3][1], c[0][2], c[1][2], c[2][2], c[3][2]];
                    Item::Brush(b)
                }
                Item::Entity(p) => {
                    let mut p = p.clone();
                    p.matrix = new;
                    Item::Entity(p)
                }
                Item::Veg(_) => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_world::placement::{load_cell_objects, parse_brush_dat, parse_object_dat, parse_vegetation_dat};

    fn pak() -> bool {
        std::path::Path::new(&std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string())).exists()
    }

    #[test]
    fn unedited_cell_saves_nothing_and_maps_every_object() {
        if !pak() {
            return;
        }
        for c in [(15, 15), (10, 14), (9, 12)] {
            let loaded = load_cell_objects("main_world", c.0, c.1);
            let cell = EditCell::load("main_world", c, &loaded);
            assert!(cell.unsaved.is_empty());
            // positions of the mapped records agree with what the streamer drew
            for (i, b) in loaded.brushes.iter().enumerate() {
                if let Some((_, rec, origin)) = cell.record(ObjectKind::Brush, i as u32) {
                    let t = Vec3::new(aa_world::reader::f32_at(&rec, 67 + 12), aa_world::reader::f32_at(&rec, 67 + 28), aa_world::reader::f32_at(&rec, 67 + 44)) + origin;
                    assert!((t - b.translation()).length() < 1e-2, "{c:?} brush {i}");
                }
            }
            for (i, v) in loaded.vegetation.iter().enumerate() {
                if let Some((_, rec, origin)) = cell.record(ObjectKind::Vegetation, i as u32) {
                    let p = Vec3::new(aa_world::reader::f32_at(&rec, 39), aa_world::reader::f32_at(&rec, 43), aa_world::reader::f32_at(&rec, 47)) + origin;
                    assert!((p - v.pos).length() < 1e-2, "{c:?} vegetation {i}");
                }
            }
            let editable = (0..loaded.brushes.len() as u32).filter(|&i| cell.editable(ObjectKind::Brush, i)).count();
            assert!(editable > 0, "{c:?}");
        }
    }

    #[test]
    fn delete_move_copy_save_and_reload() {
        if !pak() {
            return;
        }
        let c = (15, 15);
        let loaded = load_cell_objects("main_world", c.0, c.1);
        let mut cell = EditCell::load("main_world", c, &loaded);
        // delete brush 0, move brush 1 by 100 m (into another sector), copy vegetation 0, delete vegetation 1
        let mut dead = cell.state(ObjectKind::Brush, 0).unwrap();
        dead.alive = false;
        cell.apply(ObjectKind::Brush, 0, &dead);
        let item = cell.item(ObjectKind::Brush, 1).unwrap();
        let moved_to = item.position() + Vec3::new(100.0, 0.0, 1.0);
        let s = cell.moved(ObjectKind::Brush, 1, transformed(&item, Vec3::new(100.0, 0.0, 1.0), 30.0, 1.0)).unwrap();
        cell.apply(ObjectKind::Brush, 1, &s);
        let (n, _, after) = cell.duplicate(ObjectKind::Vegetation, 0, Vec3::new(2.0, 0.0, 0.0)).unwrap();
        cell.apply(ObjectKind::Vegetation, n, &after);
        let mut dead = cell.state(ObjectKind::Vegetation, 1).unwrap();
        dead.alive = false;
        cell.apply(ObjectKind::Vegetation, 1, &dead);

        let dir = std::env::temp_dir().join(format!("aa-editor-objects-test-{}", std::process::id()));
        let written = cell.save("main_world", &dir).unwrap();
        assert!(!written.is_empty());
        let client = dir.join("worlds/main_world/cells/015_015/client");
        let brushes = parse_brush_dat(&std::fs::read(client.join("brush.dat")).unwrap()).unwrap();
        assert_eq!(brushes.len(), loaded.brushes.len() - 1);
        assert!(brushes.iter().any(|b| (b.translation() - moved_to).length() < 1e-3));
        assert!(!brushes.iter().any(|b| (b.translation() - loaded.brushes[0].translation()).length() < 1e-4 && b.model == loaded.brushes[0].model));
        // the object.dat copy follows when the brush has one
        if cell.twins.contains_key(&0) || cell.twins.contains_key(&1) {
            let o = parse_object_dat(&std::fs::read(client.join("object.dat")).unwrap()).unwrap();
            let before = parse_object_dat(&aa_pak::Pak::global().read("worlds/main_world/cells/015_015/client/object.dat").unwrap()).unwrap();
            assert_eq!(o.brushes.len(), before.brushes.len() - usize::from(cell.twins.contains_key(&0)));
            if cell.twins.contains_key(&1) {
                assert!(o.brushes.iter().any(|b| (b.translation() - moved_to).length() < 1e-3));
            }
        }
        let veg_file = match cell.source(ObjectKind::Vegetation, 0) {
            Source::VegDat(_) => "vegetation.dat",
            _ => "",
        };
        if !veg_file.is_empty() {
            let v = parse_vegetation_dat(&std::fs::read(client.join(veg_file)).unwrap()).unwrap();
            let orig = parse_vegetation_dat(&aa_pak::Pak::global().read("worlds/main_world/cells/015_015/client/vegetation.dat").unwrap()).unwrap();
            assert_eq!(v.len(), orig.len()); // one copy, one deleted
            let copy = loaded.vegetation[0].pos + Vec3::new(2.0, 0.0, 0.0);
            assert!(v.iter().any(|x| (x.pos - copy).length() < 1e-3));
            assert!(!v.iter().any(|x| (x.pos - loaded.vegetation[1].pos).length() < 1e-5));
        }
        // undo everything: the files write back identical
        let mut fresh = EditCell::load("main_world", c, &loaded);
        let s0 = fresh.state(ObjectKind::Brush, 1).unwrap();
        let s = fresh.moved(ObjectKind::Brush, 1, transformed(&s0.item, Vec3::new(5.0, 5.0, 0.0), 45.0, 1.2)).unwrap();
        fresh.apply(ObjectKind::Brush, 1, &s);
        fresh.apply(ObjectKind::Brush, 1, &s0);
        let pak = aa_pak::Pak::global();
        assert!(fresh.brush_dat.as_ref().unwrap().write() == pak.read("worlds/main_world/cells/015_015/client/brush.dat").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn entity_move_keeps_parent_frame() {
        if !pak() {
            return;
        }
        // a town cell with static entity slots
        let c = (10, 11);
        let loaded = load_cell_objects("main_world", c.0, c.1);
        let mut cell = EditCell::load("main_world", c, &loaded);
        let i = (0..loaded.entity_props.len() as u32).find(|&i| cell.editable(ObjectKind::Entity, i)).expect("an editable entity prop");
        let item = cell.item(ObjectKind::Entity, i).unwrap();
        let s = cell.moved(ObjectKind::Entity, i, transformed(&item, Vec3::new(1.0, 2.0, 0.5), 90.0, 1.0)).unwrap();
        cell.apply(ObjectKind::Entity, i, &s);
        let props = aa_world::entities::parse_static_entities(cell.entities.as_ref().unwrap());
        let Item::Entity(want) = &s.item else { panic!() };
        let got = props.iter().find(|p| p.guid == want.guid).unwrap();
        assert!(got.matrix.abs_diff_eq(want.matrix, 1e-3), "{:?} vs {:?}", got.matrix, want.matrix);
    }
}
