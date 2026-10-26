//! Housing zones: where players may build, and what (Housing mode, key 6).
//!
//! **Shapes.** A housing zone is an `AreaShape` entity of a zone's
//! `game/worlds/<world>/level_design/zone/<zone key>/client/housing_area.xml`:
//!
//! ```text
//! <Objects>
//!  <Entity Name="terrorhousing1" Pos="232.48621,782.39746,103.75372" EntityClass="AreaShape" EntityId="5313" ... cellX="1" cellY="0">
//!   <Area Id="0" Group="1" Proximity="0" Priority="0" value1="212" value2="0" flags="0" Height="0">
//!    <Points>
//!     <Point Pos="0,0,0"/>            (relative to the entity's Pos)
//!    ...
//! ```
//!
//! The server (`aaemu-game/src/systems/npcs/subzone.rs`) places a point at
//! `(zone origin cell + cellX / cellY) * 1024 + Pos + point` (the origin is `originX` / `originY` of the zone's
//! `<Zone>` in `world.xml`), reads the area id from `value1` and tests a building's position against the polygon
//! (2D crossing test; `Rotate`, a 1° tilt on 4 of the 384 shapes, is ignored and kept as it is). It reads the files
//! through its client sources, so the copy the editor writes into the server's `ClientOverlay` wins; the client
//! reads the pak overlay's copy. Untouched shapes are written back byte for byte (their original lines), edited
//! ones in the same layout with the original text of every unchanged number.
//!
//! **Rules** (game database): `housing_areas` (id = `value1`, `housing_group_id`, `name` = the zone group,
//! `comments` = the shape name) → `housing_groups` (a rule set: `houseless` = only for players who own no building,
//! `existing_category_id` = not for players who already own a building of that category, `can_extend` = false: the
//! plot expires instead of paying more tax; `doodad_id`, `allowed_tax_delay_week` are client data the server does not
//! use) → `housing_group_categories` (the building categories allowed, `housings.category_id`; `max_construct_count`
//! is not checked by the server). The server's `placement.rs` accepts a building when one rule set of an area under it
//! allows its category and the player passes its conditions.
//!
//! * Click a zone to select it, drag a corner to move it, drag inside to move the zone, Shift+click an edge (or the
//!   ground) to add a corner to the selected zone, Delete removes the selected corner. Ctrl+Z / Ctrl+Y.
//! * "Draw new zone": click the corners on the ground, Enter (or Finish) closes it, Esc cancels, Backspace removes
//!   the last one. The zone file is the one of the world zone under the first corner; a new `housing_areas` row is
//!   made for it with the chosen rule set.
//! * Saving (Ctrl+S or the window) writes the changed zone files to the pak overlay and the server's `ClientOverlay`
//!   (registered as its first client source, like the terrain export) and the rule changes to the database (one
//!   transaction). The server reads both when it starts.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use aa_world::{TerrainQuery, bevy_to_cry, cry_to_bevy};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, params};

use crate::camera::EditorCamera;
use crate::db::GameData;
use crate::server_terrain::ServerTerrain;
use crate::terrain_edit::Heights;
use crate::tools::Cursor;
use crate::{Editor, Mode, Request};

const CELL: f32 = 1024.0;
const SECTOR: f32 = 64.0;
/// Corners drawn and lines sampled this far above the ground.
const LIFT: f32 = 0.4;

// ---------------------------------------------------------------------------------------------------------------
// shapes and files

/// One housing zone, in cry space.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub name: String,
    /// `value1`: the `housing_areas` id.
    pub area: u32,
    /// The entity's position (world).
    pub pos: Vec3,
    /// Corners relative to `pos`.
    pub points: Vec<Vec3>,
}

impl Shape {
    pub fn corners(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.points.iter().map(|p| self.pos + *p)
    }

    /// The server's test: is `(x, y)` inside the polygon (crossing number).
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let c: Vec<Vec3> = self.corners().collect();
        let mut inside = false;
        let mut j = c.len().wrapping_sub(1);
        for i in 0..c.len() {
            let (a, b) = (c[i], c[j]);
            if (a.y > y) != (b.y > y) && x < (b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// Area of the polygon, m².
    pub fn size(&self) -> f32 {
        let c: Vec<Vec3> = self.corners().collect();
        let n = c.len();
        (0..n).map(|i| c[i].x * c[(i + 1) % n].y - c[(i + 1) % n].x * c[i].y).sum::<f32>().abs() / 2.0
    }

    pub fn centre(&self) -> Vec3 {
        if self.points.is_empty() {
            return self.pos;
        }
        self.corners().sum::<Vec3>() / self.points.len() as f32
    }

    /// Horizontal distance from `p` to the zone (0 inside).
    pub fn distance_to(&self, p: Vec3) -> f32 {
        if self.contains(p.x, p.y) {
            return 0.0;
        }
        let c: Vec<Vec2> = self.corners().map(|c| c.truncate()).collect();
        (0..c.len()).map(|i| segment_distance(p.truncate(), c[i], c[(i + 1) % c.len()])).fold(f32::INFINITY, f32::min)
    }

    /// A request framing the whole zone.
    pub fn view(&self) -> Request {
        let c = self.centre();
        let r = self.corners().map(|p| p.truncate().distance(c.truncate())).fold(0.0, f32::max);
        Request::View(c, r * 1.6 + 20.0)
    }

    /// The edge (from corner `k` to `k + 1`) nearest to `p`.
    fn nearest_edge(&self, p: Vec3) -> Option<usize> {
        let c: Vec<Vec2> = self.corners().map(|c| c.truncate()).collect();
        (0..c.len()).min_by(|&a, &b| {
            segment_distance(p.truncate(), c[a], c[(a + 1) % c.len()]).total_cmp(&segment_distance(p.truncate(), c[b], c[(b + 1) % c.len()]))
        })
    }
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(p)
}

/// How a shape is written: its original lines and attributes, so untouched text stays as it was.
#[derive(Debug, Default)]
struct Template {
    /// The entity's lines in the file as read (`None` for a new shape).
    block: Option<String>,
    entity: Vec<(String, String)>,
    area: Vec<(String, String)>,
    /// The entity position read: world value, `Pos` text, cell.
    pos: Option<(Vec3, String, (i32, i32))>,
    /// The corners read: relative value and text.
    points: Vec<(Vec3, String)>,
}

/// A housing zone of the world as loaded and edited.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The world zone (`level_design/zone/<key>`) whose file holds it.
    pub zone: u16,
    pub now: Shape,
    pub deleted: bool,
    /// As in the file (`None`: not saved yet).
    saved: Option<Shape>,
    tmpl: Arc<Template>,
}

impl Entry {
    pub fn changed(&self) -> bool {
        match &self.saved {
            None => !self.deleted,
            Some(s) => self.deleted || *s != self.now,
        }
    }

    fn entity_id(&self) -> u32 {
        attr(&self.tmpl.entity, "EntityId").and_then(|v| v.parse().ok()).unwrap_or(0)
    }
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// The `name="value"` pairs of a tag line, in order.
fn attrs(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(eq) = rest.find("=\"") {
        let name = rest[..eq].rsplit(|c: char| c.is_whitespace() || c == '<').next().unwrap_or("").to_string();
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else { break };
        out.push((name, unescape(&after[..end])));
        rest = &after[end + 1..];
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

fn vec3(text: &str) -> Option<Vec3> {
    let v: Vec<f32> = text.split(',').map(|s| s.trim().parse().ok()).collect::<Option<_>>()?;
    (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
}

/// A float as the editor that made these files wrote it (`%.8g`).
pub fn num(v: f32) -> String {
    if v == 0.0 || !v.is_finite() {
        return "0".into();
    }
    let e = format!("{v:.7e}");
    let (m, x) = e.split_once('e').unwrap_or((&e, "0"));
    let x: i32 = x.parse().unwrap_or(0);
    if (-5..8).contains(&x) {
        let s = format!("{:.*}", (7 - x).max(0) as usize, v);
        let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
        if s == "-0" { "0".into() } else { s }
    } else {
        let m = if m.contains('.') { m.trim_end_matches('0').trim_end_matches('.') } else { m };
        format!("{m}e{x:+03}")
    }
}

fn cell_base(origin: (i32, i32), cell: (i32, i32)) -> Vec3 {
    Vec3::new((origin.0 + cell.0) as f32 * CELL, (origin.1 + cell.1) as f32 * CELL, 0.0)
}

/// The shapes of one `housing_area.xml` of a zone whose origin cell is `origin`, and its line ending.
pub fn parse_file(text: &str, zone: u16, origin: (i32, i32)) -> (Vec<Entry>, &'static str) {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut out = Vec::new();
    let mut block: Option<(String, Template, Vec<Vec3>)> = None;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with("<Entity ") {
            let entity = attrs(t);
            let cell = (
                attr(&entity, "cellX").and_then(|v| v.parse().ok()).unwrap_or(0),
                attr(&entity, "cellY").and_then(|v| v.parse().ok()).unwrap_or(0),
            );
            let pos_text = attr(&entity, "Pos").unwrap_or("0,0,0").to_string();
            let pos = cell_base(origin, cell) + vec3(&pos_text).unwrap_or_default();
            let tmpl = Template { block: None, entity, area: Vec::new(), pos: Some((pos, pos_text, cell)), points: Vec::new() };
            block = Some((String::new(), tmpl, Vec::new()));
        }
        let Some((lines, tmpl, points)) = block.as_mut() else { continue };
        lines.push_str(line);
        if t.starts_with("<Area ") {
            tmpl.area = attrs(t);
        } else if t.starts_with("<Point ") {
            let text = attrs(t).into_iter().find(|(k, _)| k == "Pos").map(|(_, v)| v).unwrap_or_default();
            let p = vec3(&text).unwrap_or_default();
            tmpl.points.push((p, text));
            points.push(p);
        } else if t == "</Entity>" {
            let (lines, mut tmpl, points) = block.take().unwrap_or_default();
            let area = attr(&tmpl.area, "value1").or(attr(&tmpl.area, "Id")).and_then(|v| v.parse().ok()).unwrap_or(0);
            let shape = Shape {
                name: attr(&tmpl.entity, "Name").unwrap_or("").to_string(),
                area,
                pos: tmpl.pos.as_ref().map_or(Vec3::ZERO, |p| p.0),
                points,
            };
            tmpl.block = Some(lines);
            out.push(Entry { zone, now: shape.clone(), deleted: false, saved: Some(shape), tmpl: Arc::new(tmpl) });
        }
    }
    (out, eol)
}

/// One shape's lines. Unchanged (and not `force`d) shapes are their original text.
fn write_block(e: &Entry, origin: (i32, i32), eol: &str, force: bool) -> String {
    let t = &e.tmpl;
    if !force
        && let (Some(block), Some(saved)) = (&t.block, &e.saved)
        && *saved == e.now
    {
        return block.clone();
    }
    let s = &e.now;
    let (pos_text, cell) = match &t.pos {
        Some((p, text, cell)) if *p == s.pos => (text.clone(), *cell),
        _ => {
            let cell = ((s.pos.x / CELL).floor() as i32 - origin.0, (s.pos.y / CELL).floor() as i32 - origin.1);
            let local = s.pos - cell_base(origin, cell);
            (format!("{},{},{}", num(local.x), num(local.y), num(local.z)), cell)
        }
    };
    let entity: Vec<String> = t
        .entity
        .iter()
        .map(|(k, v)| {
            let v = match k.as_str() {
                "Name" => s.name.clone(),
                "Pos" => pos_text.clone(),
                "cellX" => cell.0.to_string(),
                "cellY" => cell.1.to_string(),
                _ => v.clone(),
            };
            format!("{k}=\"{}\"", escape(&v))
        })
        .collect();
    let area: Vec<String> = t
        .area
        .iter()
        .map(|(k, v)| {
            let v = if k == "value1" { s.area.to_string() } else { v.clone() };
            format!("{k}=\"{}\"", escape(&v))
        })
        .collect();
    let mut out = format!(" <Entity {}>{eol}  <Area {}>{eol}   <Points>{eol}", entity.join(" "), area.join(" "));
    for p in &s.points {
        let text = t.points.iter().find(|(v, _)| v == p).map_or_else(|| format!("{},{},{}", num(p.x), num(p.y), num(p.z)), |(_, text)| text.clone());
        out.push_str(&format!("    <Point Pos=\"{text}\"/>{eol}"));
    }
    out.push_str(&format!("   </Points>{eol}   <TimeOfDay />{eol}  </Area>{eol} </Entity>{eol}"));
    out
}

/// A zone file with `entries` (deleted ones are left out).
pub fn write_file<'a>(entries: impl IntoIterator<Item = &'a Entry>, origin: (i32, i32), eol: &str, force: bool) -> String {
    let blocks: Vec<String> = entries.into_iter().filter(|e| !e.deleted).map(|e| write_block(e, origin, eol, force)).collect();
    if blocks.is_empty() {
        return format!("<Objects />{eol}");
    }
    format!("<Objects>{eol}{}</Objects>{eol}", blocks.concat())
}

/// The zone file inside a client tree (pak overlay layout, no `game/`).
pub fn file_rel(world: &str, zone: u16) -> String {
    format!("worlds/{world}/level_design/zone/{zone}/client/housing_area.xml")
}

/// A new shape's entity and area attributes, as the original editor wrote them.
fn new_template(name: &str, entity_id: u32, area: u32) -> Template {
    let pair = |k: &str, v: String| (k.to_string(), v);
    let mut h = std::hash::DefaultHasher::new();
    std::hash::Hash::hash(&(name, entity_id, std::time::SystemTime::now()), &mut h);
    let guid = format!("{:016x}", std::hash::Hasher::finish(&h));
    Template {
        block: None,
        entity: vec![
            pair("Name", name.into()),
            pair("Pos", "0,0,0".into()),
            pair("EntityClass", "AreaShape".into()),
            pair("EntityId", entity_id.to_string()),
            pair("EntityGuid", guid),
            pair("Layer", "Main".into()),
            pair("cellX", "0".into()),
            pair("cellY", "0".into()),
        ],
        area: vec![
            pair("Id", "0".into()),
            pair("Group", "1".into()),
            pair("Proximity", "0".into()),
            pair("Priority", "0".into()),
            pair("value1", area.to_string()),
            pair("value2", "0".into()),
            pair("flags", "0".into()),
            pair("Height", "0".into()),
        ],
        pos: None,
        points: Vec::new(),
    }
}

/// The zones of `world.xml`: origin cells and the sector owners (`<Zone id originX originY>/cellList/cell/sector`).
#[derive(Debug, Default)]
pub struct WorldZones {
    pub origins: BTreeMap<u16, (i32, i32)>,
    sectors: HashMap<(i32, i32), Box<[u16; 256]>>,
}

impl WorldZones {
    pub fn parse(xml: &str) -> WorldZones {
        let mut w = WorldZones::default();
        let (mut zone, mut cell) = (0u16, (0i32, 0i32));
        let get = |a: &[(String, String)], k: &str| attr(a, k).and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
        for line in xml.lines() {
            let t = line.trim_start();
            if t.starts_with("<Zone ") {
                let a = attrs(t);
                zone = get(&a, "id") as u16;
                w.origins.insert(zone, (get(&a, "originX"), get(&a, "originY")));
            } else if t.starts_with("<cell ") {
                let a = attrs(t);
                cell = (get(&a, "x"), get(&a, "y"));
            } else if t.starts_with("<sector ") {
                let a = attrs(t);
                let (sx, sy) = (get(&a, "x"), get(&a, "y"));
                if (0..16).contains(&sx) && (0..16).contains(&sy) {
                    w.sectors.entry(cell).or_insert_with(|| Box::new([0; 256]))[(sx * 16 + sy) as usize] = zone;
                }
            }
        }
        w
    }

    /// The zone owning the 64 m sector under `(x, y)`.
    pub fn zone_at(&self, x: f32, y: f32) -> Option<u16> {
        let c = ((x / CELL).floor() as i32, (y / CELL).floor() as i32);
        let sx = (((x - c.0 as f32 * CELL) / SECTOR) as usize).min(15);
        let sy = (((y - c.1 as f32 * CELL) / SECTOR) as usize).min(15);
        self.sectors.get(&c).map(|o| o[sx * 16 + sy]).filter(|&z| z != 0)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// rules (game database)

/// A `housing_groups` row and its allowed categories.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Group {
    pub id: u32,
    /// The table's own (Korean) name and description, shown when there is no English text.
    pub name_ko: String,
    pub desc_ko: String,
    /// English (`localized_texts.en_us`).
    pub name: String,
    pub desc: String,
    pub doodad_id: Option<i64>,
    pub houseless: bool,
    pub existing_category_id: Option<i64>,
    pub tax_delay_weeks: i64,
    pub can_extend: bool,
    pub categories: Vec<Category>,
}

impl Group {
    pub fn label(&self) -> String {
        let name = if self.name.is_empty() { &self.name_ko } else { &self.name };
        format!("{} {name}", self.id)
    }
}

/// A `housing_group_categories` row (`id` 0: not saved yet).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Category {
    pub id: u32,
    pub category: u32,
    pub max: Option<i64>,
}

/// A `housing_areas` row.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AreaRow {
    pub id: u32,
    /// The zone group (`zone_groups.name`) the area lies in.
    pub name: String,
    pub group: u32,
    /// The shape's name.
    pub comments: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Rules {
    pub groups: Vec<Group>,
    pub areas: BTreeMap<u32, AreaRow>,
}

impl Rules {
    pub fn group(&self, id: u32) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Rows that differ from `before` (rule sets, counted once each, and areas).
    pub fn changes(&self, before: &Rules) -> usize {
        let groups = self.groups.iter().filter(|g| before.group(g.id) != Some(*g)).count()
            + before.groups.iter().filter(|g| self.group(g.id).is_none()).count();
        let areas = self.areas.values().filter(|a| before.areas.get(&a.id) != Some(*a)).count()
            + before.areas.keys().filter(|id| !self.areas.contains_key(id)).count();
        groups + areas
    }
}

/// Names for the rule editor (not edited).
#[derive(Clone, Debug, Default)]
pub struct Labels {
    /// Building categories (`housings.category_id`): id, "designs: some English names".
    pub categories: Vec<(u32, String)>,
    /// World zone key -> zone group name (`zones.zone_key` -> `zone_groups.name`).
    pub zone_groups: HashMap<u16, String>,
}

impl Labels {
    pub fn category(&self, id: u32) -> String {
        self.categories.iter().find(|c| c.0 == id).map_or_else(|| format!("{id}"), |c| format!("{id} {}", c.1))
    }
}

fn truthy(v: SqlValue) -> bool {
    match v {
        SqlValue::Integer(i) => i != 0,
        SqlValue::Text(t) => matches!(t.trim().to_ascii_lowercase().as_str(), "t" | "true" | "1"),
        _ => false,
    }
}

fn tf(v: bool) -> &'static str {
    if v { "t" } else { "f" }
}

pub fn read_rules(conn: &Connection) -> rusqlite::Result<Rules> {
    let mut en: HashMap<(String, u32), String> = HashMap::new();
    let mut q = conn.prepare("SELECT tbl_column_name, idx, COALESCE(en_us, '') FROM localized_texts WHERE tbl_name = 'housing_groups'")?;
    for r in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32, r.get::<_, String>(2)?)))? {
        let (c, i, t) = r?;
        en.insert((c, i), t);
    }
    let mut groups = Vec::new();
    let mut q = conn.prepare(
        "SELECT id, COALESCE(name, ''), COALESCE(\"desc\", ''), doodad_id, houseless, existing_category_id, \
         COALESCE(allowed_tax_delay_week, 0), can_extend FROM housing_groups ORDER BY id",
    )?;
    for r in q.query_map([], |r| {
        let id = r.get::<_, i64>(0)? as u32;
        Ok(Group {
            id,
            name_ko: r.get(1)?,
            desc_ko: r.get(2)?,
            name: String::new(),
            desc: String::new(),
            doodad_id: r.get(3)?,
            houseless: truthy(r.get(4)?),
            existing_category_id: r.get(5)?,
            tax_delay_weeks: r.get(6)?,
            can_extend: truthy(r.get(7)?),
            categories: Vec::new(),
        })
    })? {
        let mut g = r?;
        g.name = en.get(&("name".into(), g.id)).cloned().unwrap_or_default();
        g.desc = en.get(&("desc".into(), g.id)).cloned().unwrap_or_default();
        groups.push(g);
    }
    let mut q = conn.prepare("SELECT id, housing_group_id, category_id, max_construct_count FROM housing_group_categories ORDER BY id")?;
    for r in q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32, r.get::<_, i64>(2)? as u32, r.get::<_, Option<i64>>(3)?)))? {
        let (id, group, category, max) = r?;
        if let Some(g) = groups.iter_mut().find(|g| g.id == group) {
            g.categories.push(Category { id, category, max });
        }
    }
    let mut areas = BTreeMap::new();
    let mut q = conn.prepare("SELECT id, COALESCE(name, ''), COALESCE(housing_group_id, 0), COALESCE(comments, '') FROM housing_areas ORDER BY id")?;
    for r in q.query_map([], |r| Ok(AreaRow { id: r.get::<_, i64>(0)? as u32, name: r.get(1)?, group: r.get::<_, i64>(2)? as u32, comments: r.get(3)? }))? {
        let a = r?;
        areas.insert(a.id, a);
    }
    Ok(Rules { groups, areas })
}

pub fn read_labels(conn: &Connection) -> Labels {
    let mut labels = Labels::default();
    let mut by_cat: BTreeMap<u32, (usize, Vec<String>)> = BTreeMap::new();
    if let Ok(mut q) = conn.prepare(
        "SELECT h.category_id, COALESCE(l.en_us, '') FROM housings h LEFT JOIN localized_texts l \
         ON l.tbl_name = 'housings' AND l.tbl_column_name = 'name' AND l.idx = h.id ORDER BY h.id",
    ) && let Ok(rows) = q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)))
    {
        for (c, name) in rows.flatten() {
            let e = by_cat.entry(c).or_default();
            e.0 += 1;
            if !name.is_empty() && !e.1.contains(&name) {
                e.1.push(name);
            }
        }
    }
    labels.categories = by_cat
        .into_iter()
        .map(|(c, (n, names))| {
            let shown: Vec<&str> = names.iter().take(2).map(String::as_str).collect();
            let more = if names.len() > 2 { ", …" } else { "" };
            (c, format!("{} ({n} design{}){more}", shown.join(", "), if n == 1 { "" } else { "s" }).trim().to_string())
        })
        .collect();
    if let Ok(mut q) = conn.prepare("SELECT z.zone_key, COALESCE(g.name, '') FROM zones z JOIN zone_groups g ON g.id = z.group_id")
        && let Ok(rows) = q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u16, r.get::<_, String>(1)?)))
    {
        labels.zone_groups.extend(rows.flatten());
    }
    labels
}

fn set_localized(tx: &rusqlite::Transaction, column: &str, id: u32, text: &str) -> rusqlite::Result<()> {
    let n = tx.execute(
        "UPDATE localized_texts SET en_us = ?1 WHERE tbl_name = 'housing_groups' AND tbl_column_name = ?2 AND idx = ?3",
        params![text, column, id],
    )?;
    if n == 0 {
        tx.execute(
            "INSERT INTO localized_texts (id, tbl_name, tbl_column_name, idx, en_us) \
             VALUES ((SELECT COALESCE(MAX(id), 0) + 1 FROM localized_texts), 'housing_groups', ?1, ?2, ?3)",
            params![column, id, text],
        )?;
    }
    Ok(())
}

/// Writes the differences between `before` (as read) and `now` in one transaction; returns the rows written.
pub fn write_rules(conn: &mut Connection, before: &Rules, now: &Rules) -> anyhow::Result<usize> {
    let tx = conn.transaction()?;
    let mut n = 0;
    for g in &now.groups {
        let old = before.group(g.id);
        if old == Some(g) {
            continue;
        }
        match old {
            None => {
                tx.execute(
                    "INSERT INTO housing_groups (id, name, \"desc\", doodad_id, houseless, existing_category_id, allowed_tax_delay_week, can_extend) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        g.id,
                        if g.name_ko.is_empty() { &g.name } else { &g.name_ko },
                        if g.desc_ko.is_empty() { &g.desc } else { &g.desc_ko },
                        g.doodad_id,
                        tf(g.houseless),
                        g.existing_category_id,
                        g.tax_delay_weeks,
                        tf(g.can_extend)
                    ],
                )?;
                set_localized(&tx, "name", g.id, &g.name)?;
                set_localized(&tx, "desc", g.id, &g.desc)?;
            }
            Some(o) => {
                if (o.doodad_id, o.houseless, o.existing_category_id, o.tax_delay_weeks, o.can_extend)
                    != (g.doodad_id, g.houseless, g.existing_category_id, g.tax_delay_weeks, g.can_extend)
                {
                    tx.execute(
                        "UPDATE housing_groups SET doodad_id = ?1, houseless = ?2, existing_category_id = ?3, \
                         allowed_tax_delay_week = ?4, can_extend = ?5 WHERE id = ?6",
                        params![g.doodad_id, tf(g.houseless), g.existing_category_id, g.tax_delay_weeks, tf(g.can_extend), g.id],
                    )?;
                }
                if o.name != g.name {
                    set_localized(&tx, "name", g.id, &g.name)?;
                }
                if o.desc != g.desc {
                    set_localized(&tx, "desc", g.id, &g.desc)?;
                }
            }
        }
        n += 1;
        let old_cats = old.map_or(&[][..], |o| o.categories.as_slice());
        for c in &g.categories {
            if c.id == 0 {
                tx.execute(
                    "INSERT INTO housing_group_categories (id, housing_group_id, category_id, max_construct_count) \
                     VALUES ((SELECT COALESCE(MAX(id), 0) + 1 FROM housing_group_categories), ?1, ?2, ?3)",
                    params![g.id, c.category, c.max],
                )?;
                n += 1;
            } else if old_cats.iter().find(|o| o.id == c.id) != Some(c) {
                tx.execute(
                    "UPDATE housing_group_categories SET category_id = ?1, max_construct_count = ?2 WHERE id = ?3",
                    params![c.category, c.max, c.id],
                )?;
                n += 1;
            }
        }
        for o in old_cats.iter().filter(|o| !g.categories.iter().any(|c| c.id == o.id)) {
            tx.execute("DELETE FROM housing_group_categories WHERE id = ?1", params![o.id])?;
            n += 1;
        }
    }
    for o in before.groups.iter().filter(|o| now.group(o.id).is_none()) {
        tx.execute("DELETE FROM housing_groups WHERE id = ?1", params![o.id])?;
        tx.execute("DELETE FROM housing_group_categories WHERE housing_group_id = ?1", params![o.id])?;
        tx.execute("DELETE FROM localized_texts WHERE tbl_name = 'housing_groups' AND idx = ?1", params![o.id])?;
        n += 1;
    }
    for a in now.areas.values() {
        match before.areas.get(&a.id) {
            Some(o) if o == a => {}
            Some(_) => {
                tx.execute(
                    "UPDATE housing_areas SET name = ?1, housing_group_id = ?2, comments = ?3 WHERE id = ?4",
                    params![a.name, a.group, a.comments, a.id],
                )?;
                n += 1;
            }
            None => {
                tx.execute(
                    "INSERT INTO housing_areas (id, name, housing_group_id, comments) VALUES (?1, ?2, ?3, ?4)",
                    params![a.id, a.name, a.group, a.comments],
                )?;
                n += 1;
            }
        }
    }
    for id in before.areas.keys().filter(|id| !now.areas.contains_key(id)) {
        tx.execute("DELETE FROM housing_areas WHERE id = ?1", params![id])?;
        n += 1;
    }
    tx.commit()?;
    Ok(n)
}

// ---------------------------------------------------------------------------------------------------------------
// the editor state

#[derive(Clone, Copy, Debug, PartialEq)]
enum Hover {
    Corner(usize, usize),
    /// Near edge `k` (corner `k` to `k + 1`).
    Edge(usize, usize),
    Inside(usize),
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    shape: usize,
    /// A corner, or the whole shape.
    corner: Option<usize>,
    /// Ground point and shape position at the press.
    from: Vec3,
    pos: Vec3,
    press: Vec2,
    started: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Zones,
    Rules,
}

#[derive(Clone)]
struct Snap {
    shapes: Vec<Entry>,
    rules: Option<Rules>,
}

#[derive(Resource)]
pub struct Housing {
    pub world: String,
    pub loaded: bool,
    pub zones: WorldZones,
    /// Line ending per zone file read.
    eols: HashMap<u16, &'static str>,
    pub shapes: Vec<Entry>,
    /// Area id -> shapes in the other worlds (a `housing_areas` row they use is never deleted).
    elsewhere: HashMap<u32, usize>,
    /// The database rows as now edited, and as read.
    pub rules: Option<Rules>,
    rules_saved: Rules,
    pub labels: Labels,
    pub selected: Option<usize>,
    pub corner: Option<usize>,
    hover: Option<Hover>,
    drag: Option<Drag>,
    /// The corners of a zone being drawn.
    pub drawing: Option<Vec<Vec3>>,
    /// The rule set of new zones.
    pub new_group: u32,
    pub rule: Option<u32>,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// The window field being edited: one undo step per edit of a field (typing, dragging a number).
    burst: Option<String>,
    pub open: bool,
    pub tab: Tab,
    pub search: String,
    pub near_only: bool,
    pub radius: f32,
}

impl Housing {
    pub fn new(world: &str) -> Housing {
        Housing {
            world: world.to_string(),
            loaded: false,
            zones: WorldZones::default(),
            eols: HashMap::new(),
            shapes: Vec::new(),
            elsewhere: HashMap::new(),
            rules: None,
            rules_saved: Rules::default(),
            labels: Labels::default(),
            selected: None,
            corner: None,
            hover: None,
            drag: None,
            drawing: None,
            new_group: 1,
            rule: None,
            undo: Vec::new(),
            redo: Vec::new(),
            burst: None,
            open: false,
            tab: Tab::Zones,
            search: String::new(),
            near_only: true,
            radius: 800.0,
        }
    }

    /// Reads the world's zones and housing files (through the pak overlay) and the other worlds' area ids.
    pub fn load(&mut self) {
        self.loaded = true;
        let pak = aa_pak::Pak::global();
        let started = std::time::Instant::now();
        match pak.read_string(&format!("worlds/{}/world.xml", self.world)) {
            Ok(xml) => self.zones = WorldZones::parse(&xml),
            Err(e) => warn!("housing: no world.xml for {}: {e}", self.world),
        }
        self.shapes.clear();
        for (&zone, &origin) in &self.zones.origins {
            let rel = file_rel(&self.world, zone);
            if !pak.exists(&rel) {
                continue;
            }
            match pak.read_string(&rel) {
                Ok(text) => {
                    let (entries, eol) = parse_file(&text, zone, origin);
                    self.eols.insert(zone, eol);
                    self.shapes.extend(entries);
                }
                Err(e) => warn!("housing: cannot read {rel}: {e}"),
            }
        }
        let own = format!("game/worlds/{}/", self.world.to_ascii_lowercase());
        let others: Vec<String> =
            pak.names().filter(|n| n.ends_with("/housing_area.xml") && !aa_pak::normalize(n).starts_with(&own)).map(str::to_string).collect();
        for name in others {
            if let Ok(text) = pak.read_string(&name) {
                for e in parse_file(&text, 0, (0, 0)).0 {
                    *self.elsewhere.entry(e.now.area).or_default() += 1;
                }
            }
        }
        info!(
            "housing: {} zone(s) in {} file(s) of {} ({} ms)",
            self.shapes.len(),
            self.eols.len(),
            self.world,
            started.elapsed().as_millis()
        );
    }

    pub fn load_rules(&mut self, game: &GameData) {
        match game.housing_rules() {
            Ok(r) => {
                self.rules_saved = r.clone();
                self.new_group = r.groups.first().map_or(1, |g| g.id);
                self.rules = Some(r);
            }
            Err(e) => warn!("housing: cannot read the rules from {}: {e:#}", game.path),
        }
        self.labels = game.housing_labels();
    }

    pub fn changed_count(&self) -> usize {
        self.shapes.iter().filter(|e| e.changed()).count() + self.rules.as_ref().map_or(0, |r| r.changes(&self.rules_saved))
    }

    fn snapshot(&mut self) {
        self.undo.push(Snap { shapes: self.shapes.clone(), rules: self.rules.clone() });
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.burst = None;
    }

    /// A snapshot before the first change of an edit of window field `key`.
    fn snapshot_for(&mut self, key: &str) {
        if self.burst.as_deref() != Some(key) {
            self.snapshot();
            self.burst = Some(key.to_string());
        }
    }

    fn restore(&mut self, from_undo: bool) -> bool {
        let Some(s) = (if from_undo { self.undo.pop() } else { self.redo.pop() }) else { return false };
        let now = Snap { shapes: std::mem::replace(&mut self.shapes, s.shapes), rules: std::mem::replace(&mut self.rules, s.rules) };
        if from_undo { self.redo.push(now) } else { self.undo.push(now) }
        self.selected = self.selected.filter(|&i| i < self.shapes.len() && !self.shapes[i].deleted);
        self.corner = None;
        true
    }

    pub fn undo(&mut self) -> bool {
        self.restore(true)
    }

    pub fn redo(&mut self) -> bool {
        self.restore(false)
    }

    fn live(&self, i: usize) -> Option<&Entry> {
        self.shapes.get(i).filter(|e| !e.deleted)
    }

    /// Shapes in this world (live ones) using `area`, besides `except`.
    fn area_users(&self, area: u32, except: Option<usize>) -> usize {
        self.shapes.iter().enumerate().filter(|(i, e)| !e.deleted && e.now.area == area && Some(*i) != except).count()
    }

    fn next_area_id(&self) -> u32 {
        let rules = self.rules.iter().flat_map(|r| r.areas.keys()).chain(self.rules_saved.areas.keys());
        let shapes = self.shapes.iter().map(|e| &e.now.area).chain(self.elsewhere.keys());
        rules.chain(shapes).copied().max().unwrap_or(0) + 1
    }

    /// A new zone with corners `points` (world) in the world zone under the first one, with a new area row of rule
    /// set `group`. Returns its index.
    pub fn create(&mut self, points: &[Vec3], group: u32) -> Result<usize, String> {
        let first = *points.first().ok_or("no corners")?;
        if points.len() < 3 {
            return Err("a zone needs at least 3 corners".into());
        }
        let zone = self.zones.zone_at(first.x, first.y).ok_or("no world zone owns that spot")?;
        let name = (1..10000)
            .map(|n| format!("LevelDesignShape_{zone}_editor_{n}"))
            .find(|n| !self.shapes.iter().any(|e| e.now.name == *n))
            .ok_or("no free name")?;
        let entity_id = self.shapes.iter().map(Entry::entity_id).max().unwrap_or(0) + 1;
        let area = self.next_area_id();
        self.snapshot();
        let shape = Shape { name: name.clone(), area, pos: first, points: points.iter().map(|p| *p - first).collect() };
        let tmpl = Arc::new(new_template(&name, entity_id, area));
        self.shapes.push(Entry { zone, now: shape, deleted: false, saved: None, tmpl });
        if let Some(r) = &mut self.rules {
            let zone_group = self.labels.zone_groups.get(&zone).cloned().unwrap_or_default();
            r.areas.insert(area, AreaRow { id: area, name: zone_group, group, comments: name });
        }
        self.eols.entry(zone).or_insert("\n");
        Ok(self.shapes.len() - 1)
    }

    /// Deletes zone `i`, and its area row when nothing else (in any world) uses it.
    pub fn delete(&mut self, i: usize) {
        self.snapshot();
        self.shapes[i].deleted = true;
        let area = self.shapes[i].now.area;
        if self.area_users(area, None) == 0
            && !self.elsewhere.contains_key(&area)
            && let Some(r) = &mut self.rules
        {
            r.areas.remove(&area);
        }
        self.selected = None;
        self.corner = None;
    }

    /// Writes the changed zone files under `overlay` (`<overlay>/worlds/...`) and `server` (`<server>/game/worlds/...`,
    /// the first time with a `.editor-orig` backup). Returns the files written.
    pub fn save_files(&mut self, overlay: &Path, server: Option<&Path>) -> anyhow::Result<usize> {
        let mut zones: Vec<u16> = self.shapes.iter().filter(|e| e.changed()).map(|e| e.zone).collect();
        zones.sort_unstable();
        zones.dedup();
        for &zone in &zones {
            let origin = self.zones.origins.get(&zone).copied().ok_or_else(|| anyhow::anyhow!("zone {zone} is not in world.xml"))?;
            let eol = self.eols.get(&zone).copied().unwrap_or("\n");
            let text = write_file(self.shapes.iter().filter(|e| e.zone == zone), origin, eol, false);
            let rel = file_rel(&self.world, zone);
            write(&overlay.join(&rel), &text, false)?;
            if let Some(server) = server {
                write(&server.join("game").join(&rel), &text, true)?;
            }
            // what is on disk now is the new original (its lines are written back as they are)
            let (fresh, _) = parse_file(&text, zone, origin);
            let mut fresh = fresh.into_iter();
            let mut kept = Vec::with_capacity(self.shapes.len());
            let mut moved_to = Vec::with_capacity(self.shapes.len());
            for e in std::mem::take(&mut self.shapes) {
                if e.zone != zone {
                    moved_to.push(Some(kept.len()));
                    kept.push(e);
                } else if !e.deleted
                    && let Some(f) = fresh.next()
                {
                    moved_to.push(Some(kept.len()));
                    kept.push(f);
                } else {
                    moved_to.push(None);
                }
            }
            self.selected = self.selected.and_then(|s| moved_to.get(s).copied().flatten());
            self.shapes = kept;
        }
        if !zones.is_empty() {
            self.undo.clear();
            self.redo.clear();
            self.corner = None;
        }
        Ok(zones.len())
    }
}

/// Writes `text` to `file` (making its folder), with a `.editor-orig` backup of the file there first if `backup`.
fn write(file: &Path, text: &str, backup: bool) -> anyhow::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if backup {
        crate::server_terrain::backup(file)?;
    }
    std::fs::write(file, text).map_err(|e| anyhow::anyhow!("write {}: {e}", file.display()))
}

// ---------------------------------------------------------------------------------------------------------------
// systems

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, setup)
        .add_systems(Update, (keys, load, interact, save, nudge, draw).chain())
        .add_systems(EguiPrimaryContextPass, (window, labels).after(crate::ui::panels));
}

fn setup(mut commands: Commands, editor: Res<Editor>) {
    let mut h = Housing::new(&editor.world);
    h.open = editor.mode == Mode::Housing;
    // `--housing-tab rules`: start on the rule sets (screenshots)
    if crate::arg("--housing-tab").as_deref() == Some("rules") {
        h.tab = Tab::Rules;
    }
    commands.insert_resource(h);
}

/// Loads the files and rules the first time Housing mode is used (or with `--housing <area id>`).
fn load(mut editor: ResMut<Editor>, mut housing: ResMut<Housing>, game: Res<GameData>, mut picked: Local<bool>) {
    if housing.loaded || (editor.mode != Mode::Housing && crate::arg("--housing").is_none()) {
        return;
    }
    housing.load();
    housing.load_rules(&game);
    if !*picked
        && let Some(area) = crate::arg("--housing").and_then(|v| v.parse::<u32>().ok())
    {
        *picked = true;
        editor.mode = Mode::Housing;
        housing.open = true;
        housing.selected = housing.shapes.iter().position(|e| e.now.area == area);
        match housing.selected {
            Some(i) => {
                let v = housing.shapes[i].now.view();
                editor.requests.push(v);
                housing.rule = housing.rules.as_ref().and_then(|r| r.areas.get(&area)).map(|a| a.group);
            }
            None => {
                let world = editor.world.clone();
                editor.status(format!("housing: no zone with area id {area} in {world}"));
            }
        }
    }
}

/// `--housing-nudge dx,dy`: moves the selected zone once loaded and saves (a check of the whole save path).
fn nudge(mut editor: ResMut<Editor>, mut housing: ResMut<Housing>, mut done: Local<bool>) {
    if *done || !housing.loaded {
        return;
    }
    *done = true;
    let Some(v) = crate::arg("--housing-nudge") else { return };
    let d: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if let (Some(i), [dx, dy, ..]) = (housing.selected, d.as_slice()) {
        housing.snapshot();
        housing.shapes[i].now.pos += Vec3::new(*dx, *dy, 0.0);
        editor.status(format!("housing: moved {} by {dx}, {dy}", housing.shapes[i].now.name));
        editor.requests.push(Request::SaveHousing);
    }
}

fn ground(heights: &Heights, query: &TerrainQuery, p: Vec3) -> f32 {
    heights.height_at(query, p.x, p.y).unwrap_or(p.z)
}

/// The zones drawn: near the camera, plus the selected one.
fn shown(housing: &Housing, camera: Vec3) -> Vec<usize> {
    let mut out: Vec<usize> =
        housing.shapes.iter().enumerate().filter(|(_, e)| !e.deleted && e.now.distance_to(camera) < housing.radius).map(|(i, _)| i).collect();
    if let Some(s) = housing.selected.filter(|&s| housing.live(s).is_some() && !out.contains(&s)) {
        out.push(s);
    }
    out
}

fn keys(keys: Res<ButtonInput<KeyCode>>, egui: Res<EguiWantsInput>, mut editor: ResMut<Editor>, mut housing: ResMut<Housing>) {
    let typing = egui.wants_any_keyboard_input();
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if keys.just_pressed(KeyCode::Digit6) && !typing && !ctrl {
        editor.mode = Mode::Housing;
        housing.open = true;
    }
    if editor.mode != Mode::Housing {
        housing.drawing = None;
        return;
    }
    if editor.playing || typing {
        return;
    }
    if ctrl && keys.just_pressed(KeyCode::KeyZ) && housing.undo() {
        editor.status("housing: undo");
    }
    if ctrl && keys.just_pressed(KeyCode::KeyY) && housing.redo() {
        editor.status("housing: redo");
    }
    if let Some(points) = housing.drawing.as_mut() {
        if keys.just_pressed(KeyCode::Escape) {
            housing.drawing = None;
            editor.status("housing: drawing cancelled");
        } else if keys.just_pressed(KeyCode::Backspace) || keys.just_pressed(KeyCode::Delete) {
            points.pop();
        } else if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
            finish(&mut editor, &mut housing);
        }
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        if housing.corner.is_some() {
            housing.corner = None;
        } else {
            housing.selected = None;
        }
    }
    if (keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace))
        && let (Some(i), Some(k)) = (housing.selected, housing.corner)
    {
        if housing.shapes[i].now.points.len() <= 3 {
            editor.status("housing: a zone keeps at least 3 corners (Delete zone in the window removes it)");
        } else {
            housing.snapshot();
            let s = &mut housing.shapes[i].now;
            s.points.remove(k);
            // corner 0 is the entity position: keep the others where they are
            if k == 0 {
                let d = s.points[0];
                s.pos += d;
                for p in &mut s.points {
                    *p -= d;
                }
            }
            housing.corner = None;
            editor.status(format!("housing: corner {} removed", k + 1));
        }
    }
    if keys.just_pressed(KeyCode::KeyF)
        && !ctrl
        && let Some(e) = housing.selected.and_then(|i| housing.live(i))
    {
        let r = match housing.corner.and_then(|k| e.now.points.get(k)) {
            Some(p) => Request::Focus(e.now.pos + *p),
            None => e.now.view(),
        };
        editor.requests.push(r);
    }
}

/// Closes the zone being drawn.
fn finish(editor: &mut Editor, housing: &mut Housing) {
    let points = housing.drawing.take().unwrap_or_default();
    let group = housing.new_group;
    match housing.create(&points, group) {
        Ok(i) => {
            housing.selected = Some(i);
            let e = &housing.shapes[i];
            editor.status(format!("housing: new zone {} (area {}, zone file {}); save to write it", e.now.name, e.now.area, e.zone));
        }
        Err(e) => {
            editor.status(format!("housing: {e}"));
            if points.len() < 3 {
                housing.drawing = Some(points);
            }
        }
    }
}

fn interact(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Res<Cursor>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    window: Query<&Window, With<PrimaryWindow>>,
    cam: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
    mut editor: ResMut<Editor>,
    mut housing: ResMut<Housing>,
) {
    if editor.mode != Mode::Housing || editor.playing || !housing.loaded {
        housing.hover = None;
        housing.drag = None;
        return;
    }
    let (Ok(window), Ok((camera, ct))) = (window.single(), cam.single()) else { return };
    let mouse_pos = window.cursor_position();
    let clicked = mouse.just_pressed(MouseButton::Left) && !cursor.over_ui;
    if housing.drawing.is_some() {
        housing.hover = None;
        if clicked
            && let Some(g) = cursor.ground
        {
            let z = ground(&heights, &query, g);
            if let Some(d) = housing.drawing.as_mut() {
                d.push(Vec3::new(g.x, g.y, z));
            }
        }
        return;
    }

    // hover: a corner within 12 px, else an edge within 8 px, else inside a zone (the smallest)
    housing.hover = None;
    if let Some(m) = mouse_pos.filter(|_| !cursor.over_ui && housing.drag.is_none()) {
        let screen = |p: Vec3| camera.world_to_viewport(ct, cry_to_bevy(p)).ok();
        let mut best: Option<(f32, Hover)> = None;
        for i in shown(&housing, bevy_to_cry(ct.translation())) {
            let s = &housing.shapes[i].now;
            let bonus = if housing.selected == Some(i) { 3.0 } else { 0.0 };
            let pts: Vec<Option<Vec2>> = s.corners().map(|c| screen(Vec3::new(c.x, c.y, ground(&heights, &query, c) + LIFT))).collect();
            for (k, p) in pts.iter().enumerate() {
                if let Some(p) = p {
                    let d = p.distance(m) - bonus;
                    if d < 12.0 && best.is_none_or(|b| d < b.0) {
                        best = Some((d, Hover::Corner(i, k)));
                    }
                }
            }
            for k in 0..pts.len() {
                let (Some(a), Some(b)) = (pts[k], pts[(k + 1) % pts.len()]) else { continue };
                let d = segment_distance(m, a, b) + 4.0 - bonus;
                if d < 12.0 && best.is_none_or(|b| d < b.0) {
                    best = Some((d, Hover::Edge(i, k)));
                }
            }
        }
        housing.hover = best.map(|b| b.1);
        if housing.hover.is_none()
            && let Some(g) = cursor.ground
        {
            housing.hover = shown(&housing, bevy_to_cry(ct.translation()))
                .into_iter()
                .filter(|&i| housing.shapes[i].now.contains(g.x, g.y))
                .min_by(|&a, &b| housing.shapes[a].now.size().total_cmp(&housing.shapes[b].now.size()))
                .map(Hover::Inside);
        }
    }

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if clicked {
        if shift {
            let Some(g) = cursor.ground else { return };
            let target = match housing.hover {
                Some(Hover::Edge(i, k)) => Some((i, k)),
                _ => housing.selected.and_then(|i| housing.live(i).and_then(|e| e.now.nearest_edge(g)).map(|k| (i, k))),
            };
            let Some((i, k)) = target else {
                editor.status("housing: select a zone before adding corners");
                return;
            };
            housing.snapshot();
            let z = ground(&heights, &query, g);
            let s = &mut housing.shapes[i].now;
            s.points.insert(k + 1, Vec3::new(g.x, g.y, z) - s.pos);
            housing.selected = Some(i);
            housing.corner = Some(k + 1);
            editor.status(format!("housing: corner {} added", k + 2));
            return;
        }
        let g = cursor.ground.unwrap_or_default();
        let press = mouse_pos.unwrap_or_default();
        match housing.hover {
            Some(Hover::Corner(i, k)) => {
                housing.selected = Some(i);
                housing.corner = Some(k);
                housing.drag = Some(Drag { shape: i, corner: Some(k), from: g, pos: housing.shapes[i].now.pos, press, started: false });
            }
            Some(Hover::Edge(i, _) | Hover::Inside(i)) => {
                if housing.selected != Some(i) {
                    housing.rule = housing.rules.as_ref().and_then(|r| r.areas.get(&housing.shapes[i].now.area)).map(|a| a.group);
                }
                housing.selected = Some(i);
                housing.corner = None;
                housing.drag = Some(Drag { shape: i, corner: None, from: g, pos: housing.shapes[i].now.pos, press, started: false });
            }
            None => {
                housing.selected = None;
                housing.corner = None;
            }
        }
    }
    if !mouse.pressed(MouseButton::Left) {
        housing.drag = None;
        return;
    }
    let (Some(g), Some(m)) = (cursor.ground, mouse_pos) else { return };
    let Some(d) = housing.drag.as_mut() else { return };
    if !d.started && d.press.distance(m) < 4.0 {
        return;
    }
    let first = !d.started;
    d.started = true;
    let d = *d;
    if first {
        housing.snapshot();
    }
    let z = ground(&heights, &query, g);
    let Some(s) = housing.shapes.get_mut(d.shape).map(|e| &mut e.now) else { return };
    match d.corner {
        Some(k) if k < s.points.len() => s.points[k] = Vec3::new(g.x, g.y, z) - s.pos,
        Some(_) => {}
        None => s.pos = d.pos + Vec3::new(g.x - d.from.x, g.y - d.from.y, 0.0),
    }
}

fn save(mut editor: ResMut<Editor>, mut housing: ResMut<Housing>, mut game: ResMut<GameData>, st: Res<ServerTerrain>) {
    let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::SaveHousing)) else { return };
    editor.requests.remove(i);
    if !housing.loaded {
        return;
    }
    let mut said = Vec::new();
    let source = st.source_dir();
    let overlay = editor.paths.overlay.clone();
    match housing.save_files(&overlay, Some(&source)) {
        Ok(0) => {}
        Ok(n) => {
            said.push(format!("{n} zone file(s) written to the pak overlay and {}", source.display()));
            if st.register {
                match crate::server_terrain::register_source(&st.config, &source) {
                    Ok(true) => said.push(format!("{} now lists it first", st.config.display())),
                    Ok(false) => {}
                    Err(e) => said.push(format!("could not register it in {}: {e:#}", st.config.display())),
                }
            }
        }
        Err(e) => said.push(format!("zone files not written: {e:#}")),
    }
    if let Some(now) = housing.rules.clone() {
        let c = now.changes(&housing.rules_saved);
        if c > 0 {
            match game.save_housing_rules(&housing.rules_saved, &now) {
                Ok(rows) => {
                    said.push(format!("{rows} rule row(s) written to {}", game.path));
                    housing.load_rules(&game);
                }
                Err(e) => said.push(format!("{c} rule change(s) not written: {e:#}")),
            }
        }
    }
    if !said.is_empty() {
        editor.status(format!("housing: {}; restart the server to load them", said.join(", ")));
    }
}

/// A colour per rule set.
fn group_color(group: u32) -> Color {
    const PALETTE: [(f32, f32, f32); 8] =
        [(0.3, 0.85, 1.0), (0.45, 1.0, 0.45), (1.0, 0.55, 0.3), (0.85, 0.5, 1.0), (1.0, 0.9, 0.4), (0.4, 1.0, 0.85), (1.0, 0.45, 0.6), (0.7, 0.75, 1.0)];
    let (r, g, b) = PALETTE[group as usize % PALETTE.len()];
    Color::srgba(r, g, b, 0.9)
}

fn draw(
    editor: Res<Editor>,
    housing: Res<Housing>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    cursor: Res<Cursor>,
    cam: Query<&GlobalTransform, With<EditorCamera>>,
    mut gizmos: Gizmos,
) {
    if editor.playing || editor.mode != Mode::Housing || !housing.loaded {
        return;
    }
    let Ok(ct) = cam.single() else { return };
    let at = |p: Vec3| cry_to_bevy(Vec3::new(p.x, p.y, ground(&heights, &query, p) + LIFT));
    // an edge hugging the ground: a point every 4 m (at most 64)
    let edge = |gizmos: &mut Gizmos, a: Vec3, b: Vec3, color: Color| {
        let n = ((a.truncate().distance(b.truncate()) / 4.0).ceil() as usize).clamp(1, 64);
        let mut prev = at(a);
        for s in 1..=n {
            let next = at(a.lerp(b, s as f32 / n as f32));
            gizmos.line(prev, next, color);
            prev = next;
        }
    };
    let group_of = |area: u32| housing.rules.as_ref().and_then(|r| r.areas.get(&area)).map_or(0, |a| a.group);
    let mut posts = Vec::new();
    for i in shown(&housing, bevy_to_cry(ct.translation())) {
        let s = &housing.shapes[i].now;
        let selected = housing.selected == Some(i);
        let hovered = matches!(housing.hover, Some(Hover::Corner(h, _) | Hover::Edge(h, _) | Hover::Inside(h)) if h == i);
        let color = if selected {
            Color::srgb(1.0, 0.8, 0.15)
        } else if hovered {
            Color::WHITE
        } else {
            group_color(group_of(s.area))
        };
        let c: Vec<Vec3> = s.corners().collect();
        for k in 0..c.len() {
            let edge_hot = housing.hover == Some(Hover::Edge(i, k));
            edge(&mut gizmos, c[k], c[(k + 1) % c.len()], if edge_hot { Color::WHITE } else { color });
        }
        for (k, p) in c.iter().enumerate() {
            let hot = selected && housing.corner == Some(k) || housing.hover == Some(Hover::Corner(i, k));
            posts.push((at(*p), if selected || hot { color } else { color.with_alpha(0.5) }, if hot { 0.9 } else if selected { 0.5 } else { 0.0 }));
        }
    }
    for (p, color, r) in posts {
        gizmos.line(p, p + Vec3::Y * 2.5, color);
        if r > 0.0 {
            gizmos.sphere(Isometry3d::from_translation(p), r, color);
        }
    }
    if let Some(points) = &housing.drawing {
        let magenta = Color::srgb(1.0, 0.4, 1.0);
        for w in points.windows(2) {
            edge(&mut gizmos, w[0], w[1], magenta);
        }
        if let (Some(last), Some(g)) = (points.last(), cursor.ground) {
            edge(&mut gizmos, *last, g, magenta.with_alpha(0.6));
            if points.len() >= 2 {
                edge(&mut gizmos, g, points[0], magenta.with_alpha(0.3));
            }
        }
        for p in points {
            gizmos.sphere(Isometry3d::from_translation(at(*p)), 0.5, magenta);
        }
    }
}

/// Zone names and rule sets in the view, corner numbers of the selected zone.
fn labels(
    mut contexts: EguiContexts,
    editor: Res<Editor>,
    housing: Res<Housing>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
    area: Res<crate::ui::UiArea>,
    cam: Query<(&Camera, &GlobalTransform), With<EditorCamera>>,
) -> Result {
    if editor.playing || editor.mode != Mode::Housing || !housing.loaded {
        return Ok(());
    }
    let Ok((camera, ct)) = cam.single() else { return Ok(()) };
    let ctx = contexts.ctx_mut()?;
    let mut painter = ctx.layer_painter(egui::LayerId::background());
    if let Some(view) = area.view {
        painter.set_clip_rect(view);
    }
    let screen = |p: Vec3| camera.world_to_viewport(ct, cry_to_bevy(Vec3::new(p.x, p.y, ground(&heights, &query, p) + 3.0))).ok().map(|s| egui::pos2(s.x, s.y));
    let text = |at: egui::Pos2, s: &str, color: egui::Color32, size: f32| {
        painter.text(at + egui::vec2(1.0, 1.0), egui::Align2::CENTER_BOTTOM, s, egui::FontId::proportional(size), egui::Color32::BLACK);
        painter.text(at, egui::Align2::CENTER_BOTTOM, s, egui::FontId::proportional(size), color);
    };
    let cam_pos = bevy_to_cry(ct.translation());
    for i in shown(&housing, cam_pos) {
        let s = &housing.shapes[i].now;
        let selected = housing.selected == Some(i);
        let centre = s.centre();
        if !selected && centre.distance(cam_pos) > 600.0 {
            continue;
        }
        let rule = housing.rules.as_ref().and_then(|r| r.areas.get(&s.area).and_then(|a| r.group(a.group))).map_or_else(|| "no rule set".into(), Group::label);
        if let Some(at) = screen(centre) {
            let color = if selected { egui::Color32::from_rgb(255, 205, 40) } else { egui::Color32::from_rgb(200, 235, 255) };
            text(at, &format!("🏠 {} · area {} · {rule}", s.name, s.area), color, 13.0);
        }
        if selected {
            for (k, c) in s.corners().enumerate() {
                if let Some(at) = screen(c) {
                    text(at, &format!("{}", k + 1), egui::Color32::WHITE, 12.0);
                }
            }
        }
    }
    if let Some(points) = &housing.drawing
        && let Some(at) = points.first().and_then(|p| screen(*p))
    {
        text(at, &format!("new zone: {} corner(s) · Enter closes, Esc cancels", points.len()), egui::Color32::from_rgb(255, 120, 255), 13.0);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// the window

fn window(mut contexts: EguiContexts, mut editor: ResMut<Editor>, mut housing: ResMut<Housing>, game: Res<GameData>, cam: Query<&GlobalTransform, With<EditorCamera>>) -> Result {
    if editor.playing || editor.mode != Mode::Housing || !housing.loaded {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    let cam_pos = cam.single().map(|t| bevy_to_cry(t.translation())).unwrap_or_default();
    // an edit burst ends when nothing is held or focused
    if !ctx.input(|i| i.pointer.any_down()) && ctx.memory(|m| m.focused().is_none()) {
        housing.burst = None;
    }
    let mut open = housing.open;
    egui::Window::new("Housing zones").open(&mut open).default_pos([330.0, 60.0]).default_size([440.0, 660.0]).resizable(true).show(&ctx, |ui| {
        let h = &mut *housing;
        ui.horizontal(|ui| {
            let n = h.changed_count();
            if ui.add_enabled(n > 0, egui::Button::new(format!("Save {n} change(s)"))).on_hover_text("Ctrl+S: zone files to the pak overlay and the server, rules to the database").clicked() {
                editor.requests.push(Request::SaveHousing);
            }
            if ui.add_enabled(!h.undo.is_empty(), egui::Button::new("Undo")).clicked() {
                h.undo();
            }
            if ui.add_enabled(!h.redo.is_empty(), egui::Button::new("Redo")).clicked() {
                h.redo();
            }
            ui.add(egui::DragValue::new(&mut h.radius).range(100.0..=6000.0).speed(10.0).suffix(" m")).on_hover_text("Zones within this distance of the camera are drawn");
        });
        if !game.writable {
            ui.colored_label(egui::Color32::from_rgb(255, 170, 60), "The database is read-only: rule changes are not saved.");
        }
        ui.horizontal(|ui| {
            ui.selectable_value(&mut h.tab, Tab::Zones, "Zones");
            ui.selectable_value(&mut h.tab, Tab::Rules, "Rule sets");
        });
        ui.separator();
        match h.tab {
            Tab::Zones => zones_tab(ui, &mut editor, h, cam_pos),
            Tab::Rules => rules_tab(ui, h),
        }
    });
    housing.open = open;
    if !open {
        housing.open = true;
        housing.drawing = None;
        editor.mode = Mode::Select;
    }
    Ok(())
}

/// A rule set picker.
fn group_combo(ui: &mut egui::Ui, salt: egui::Id, rules: &Rules, value: &mut u32) -> bool {
    let shown = rules.group(*value).map_or_else(|| format!("{value} (missing)"), Group::label);
    let mut changed = false;
    egui::ComboBox::from_id_salt(salt).width(220.0).selected_text(shown).show_ui(ui, |ui| {
        for g in &rules.groups {
            changed |= ui.selectable_value(value, g.id, g.label()).changed();
        }
    });
    changed
}

fn zones_tab(ui: &mut egui::Ui, editor: &mut Editor, h: &mut Housing, cam_pos: Vec3) {
    ui.small("Click a zone to select · drag a corner or the inside · Shift+click: add a corner · Del: remove the corner · Ctrl+Z/Y · F: focus");
    // new zone
    ui.horizontal(|ui| {
        if let Some(points) = &h.drawing {
            ui.colored_label(egui::Color32::from_rgb(255, 120, 255), format!("Drawing: {} corner(s)", points.len()));
            let n = points.len();
            if ui.add_enabled(n >= 3, egui::Button::new("Finish (Enter)")).clicked() {
                finish(editor, h);
            }
            if ui.button("Cancel (Esc)").clicked() {
                h.drawing = None;
            }
        } else if ui.button("✏ Draw new zone").on_hover_text("Click the corners on the ground, Enter closes the zone").clicked() {
            h.drawing = Some(Vec::new());
            h.selected = None;
            editor.status("housing: click the corners of the new zone on the ground; Enter closes it, Esc cancels");
        }
        if let Some(r) = h.rules.clone() {
            ui.label("rule set");
            let mut g = h.new_group;
            if group_combo(ui, egui::Id::new("new zone group"), &r, &mut g) {
                h.new_group = g;
            }
        }
    });
    ui.separator();

    ui.horizontal(|ui| {
        ui.label("Search");
        ui.add(egui::TextEdit::singleline(&mut h.search).desired_width(150.0).hint_text("name or area id"));
        ui.checkbox(&mut h.near_only, "near the camera");
    });
    let needle = h.search.trim().to_lowercase();
    let mut list: Vec<(usize, f32)> = h
        .shapes
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.deleted && (needle.is_empty() || e.now.name.to_lowercase().contains(&needle) || e.now.area.to_string() == needle))
        .map(|(i, e)| (i, e.now.distance_to(cam_pos)))
        .filter(|(_, d)| !h.near_only || !needle.is_empty() || *d < h.radius * 3.0)
        .collect();
    list.sort_by(|a, b| a.1.total_cmp(&b.1));
    let live = h.shapes.iter().filter(|e| !e.deleted).count();
    ui.small(format!("{live} zone(s) in {}, {} listed", h.world, list.len()));
    let mut pick = None;
    egui::ScrollArea::vertical().id_salt("housing zones").max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
        if list.is_empty() {
            ui.weak(if h.near_only { "No zone near the camera (untick \"near the camera\" or search)." } else { "No zone." });
        }
        for (i, d) in list.iter().take(500) {
            let e = &h.shapes[*i];
            let rule = h.rules.as_ref().and_then(|r| r.areas.get(&e.now.area).and_then(|a| r.group(a.group))).map_or_else(|| "–".into(), Group::label);
            let mark = if e.changed() { "● " } else { "" };
            let label = format!("{mark}{}  · area {} · {rule} · {:.0} m", e.now.name, e.now.area, d);
            if ui.selectable_label(h.selected == Some(*i), label).clicked() {
                pick = Some(*i);
            }
        }
    });
    if let Some(i) = pick {
        h.selected = Some(i);
        h.corner = None;
        h.rule = h.rules.as_ref().and_then(|r| r.areas.get(&h.shapes[i].now.area)).map(|a| a.group);
        editor.requests.push(h.shapes[i].now.view());
    }
    ui.separator();

    let Some(i) = h.selected.filter(|&i| h.live(i).is_some()) else {
        ui.weak("Select a zone in the list or the view.");
        return;
    };
    let e = h.shapes[i].clone();
    ui.horizontal(|ui| {
        ui.label("Name");
        let mut name = e.now.name.clone();
        let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0));
        if r.changed() && !name.trim().is_empty() {
            h.snapshot_for(&format!("name {i}"));
            let old = std::mem::replace(&mut h.shapes[i].now.name, name.clone());
            if let Some(a) = h.rules.as_mut().and_then(|r| r.areas.get_mut(&e.now.area)).filter(|a| a.comments == old) {
                a.comments = name;
            }
        }
    });
    let s = &e.now;
    ui.small(format!(
        "{} corners · {:.0} m² · zone file {} ({}){}",
        s.points.len(),
        s.size(),
        e.zone,
        h.labels.zone_groups.get(&e.zone).map(String::as_str).unwrap_or("?"),
        if e.saved.is_none() { " · new" } else { "" }
    ));

    // the area row and its rule set
    let users = h.area_users(s.area, Some(i)) + h.elsewhere.get(&s.area).copied().unwrap_or(0);
    let mut area_id = s.area;
    ui.horizontal(|ui| {
        ui.label("Area id");
        if ui.add(egui::DragValue::new(&mut area_id).range(1..=u32::MAX)).on_hover_text("value1: the housing_areas row this zone uses").changed() {
            h.snapshot();
            h.shapes[i].now.area = area_id;
        }
        if users > 0 {
            ui.small(format!("shared with {users} other zone(s)"));
        }
    });
    let rules_ok = h.rules.is_some();
    if rules_ok {
        let row = h.rules.as_ref().and_then(|r| r.areas.get(&area_id).cloned());
        match row {
            Some(mut a) => {
                ui.horizontal(|ui| {
                    ui.label("Rule set");
                    let r = h.rules.clone().unwrap_or_default();
                    if group_combo(ui, egui::Id::new(("zone group", i)), &r, &mut a.group) {
                        h.snapshot();
                        h.rule = Some(a.group);
                        if let Some(r) = &mut h.rules {
                            r.areas.insert(a.id, a.clone());
                        }
                    }
                    if ui.small_button("edit").on_hover_text("Open this rule set in the Rule sets tab").clicked() {
                        h.rule = Some(a.group);
                        h.tab = Tab::Rules;
                    }
                });
                if users > 0 {
                    ui.small(format!("(the rule set belongs to area {}: it applies to the {users} other zone(s) too)", a.id));
                }
                if let Some(g) = h.rules.as_ref().and_then(|r| r.group(a.group)) {
                    let cats: Vec<String> = g.categories.iter().map(|c| h.labels.category(c.category)).collect();
                    ui.small(format!("Allows: {}", if cats.is_empty() { "nothing".into() } else { cats.join("; ") }));
                    if !g.desc.is_empty() {
                        ui.small(&g.desc);
                    }
                }
                ui.small(format!("housing_areas {}: zone group \"{}\", comment \"{}\"", a.id, a.name, a.comments));
            }
            None => {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(255, 170, 60), format!("No housing_areas row {area_id}: nobody can build here."));
                    if ui.button("Create the row").clicked() {
                        h.snapshot();
                        let zone_group = h.labels.zone_groups.get(&e.zone).cloned().unwrap_or_default();
                        let group = h.new_group;
                        if let Some(r) = &mut h.rules {
                            r.areas.insert(area_id, AreaRow { id: area_id, name: zone_group, group, comments: e.now.name.clone() });
                        }
                    }
                });
            }
        }
    }

    ui.horizontal_wrapped(|ui| {
        if ui.button("Focus").clicked() {
            editor.requests.push(e.now.view());
        }
        if ui.button("Duplicate").on_hover_text("A copy 20 m east with its own area row and the same rule set").clicked() {
            let pts: Vec<Vec3> = e.now.corners().map(|c| c + Vec3::new(20.0, 0.0, 0.0)).collect();
            let group = h.rules.as_ref().and_then(|r| r.areas.get(&e.now.area)).map_or(h.new_group, |a| a.group);
            match h.create(&pts, group) {
                Ok(j) => h.selected = Some(j),
                Err(err) => editor.status(format!("housing: {err}")),
            }
        }
        if ui.button("🗑 Delete zone").clicked() {
            h.delete(i);
            editor.status(format!("housing: {} deleted (written when you save)", e.now.name));
        }
    });

    // the corners
    ui.label("Corners");
    let n = e.now.points.len();
    let mut edit: Option<(usize, Vec3)> = None;
    let mut remove = None;
    egui::ScrollArea::vertical().id_salt("housing corners").auto_shrink([false, true]).show(ui, |ui| {
        egui::Grid::new("housing corners grid").num_columns(4).striped(true).show(ui, |ui| {
            ui.strong("#");
            ui.strong("X");
            ui.strong("Y");
            ui.label("");
            ui.end_row();
            for (k, c) in e.now.corners().enumerate() {
                if ui.selectable_label(h.corner == Some(k), format!("{}", k + 1)).clicked() {
                    h.corner = Some(k);
                    editor.requests.push(Request::Focus(c));
                }
                let mut v = c;
                let a = ui.add(egui::DragValue::new(&mut v.x).speed(0.1).max_decimals(2));
                let b = ui.add(egui::DragValue::new(&mut v.y).speed(0.1).max_decimals(2));
                if (a.changed() || b.changed()) && v != c {
                    edit = Some((k, v));
                }
                if n > 3 && ui.small_button("✖").clicked() {
                    remove = Some(k);
                }
                ui.end_row();
            }
        });
    });
    if let Some((k, v)) = edit {
        h.snapshot_for(&format!("corner {i} {k}"));
        let s = &mut h.shapes[i].now;
        s.points[k] = v - s.pos;
    }
    if let Some(k) = remove {
        h.selected = Some(i);
        h.corner = Some(k);
        h.snapshot();
        let s = &mut h.shapes[i].now;
        s.points.remove(k);
        if k == 0 {
            let d = s.points[0];
            s.pos += d;
            for p in &mut s.points {
                *p -= d;
            }
        }
        h.corner = None;
    }
}

fn rules_tab(ui: &mut egui::Ui, h: &mut Housing) {
    let Some(mut rules) = h.rules.clone() else {
        ui.weak("The rules could not be read from the database (see the log).");
        return;
    };
    let used = |rules: &Rules, g: u32| rules.areas.values().filter(|a| a.group == g).count();
    ui.small("A zone's area row names a rule set; the server lets a building stand where a rule set of an area under it allows the building's category and the player passes its conditions.");
    egui::ScrollArea::vertical().id_salt("housing rule list").max_height(170.0).auto_shrink([false, true]).show(ui, |ui| {
        for g in &rules.groups {
            let label = format!("{}  · {} area(s) · {} categor{}", g.label(), used(&rules, g.id), g.categories.len(), if g.categories.len() == 1 { "y" } else { "ies" });
            if ui.selectable_label(h.rule == Some(g.id), label).clicked() {
                h.rule = Some(g.id);
            }
        }
    });
    let selected = h.rule.and_then(|id| rules.groups.iter().position(|g| g.id == id));
    ui.horizontal(|ui| {
        if ui.button("New rule set").on_hover_text("A copy of the selected one (or an empty one) with the next id").clicked() {
            let id = rules.groups.iter().map(|g| g.id).chain(h.rules_saved.groups.iter().map(|g| g.id)).max().unwrap_or(0) + 1;
            let mut g = selected.map(|k| rules.groups[k].clone()).unwrap_or_default();
            g.id = id;
            g.name = if g.name.is_empty() { format!("Rule set {id}") } else { format!("{} (copy)", g.name) };
            if !g.name_ko.is_empty() {
                g.name_ko = format!("{} (copy)", g.name_ko);
            }
            for c in &mut g.categories {
                c.id = 0;
            }
            rules.groups.push(g);
            h.rule = Some(id);
        }
        if let Some(k) = selected {
            let n = used(&rules, rules.groups[k].id);
            if ui.add_enabled(n == 0, egui::Button::new("Delete rule set")).on_disabled_hover_text(format!("{n} area(s) use it: give them another rule set first")).clicked() {
                rules.groups.remove(k);
                h.rule = None;
            }
        }
    });
    ui.separator();
    if let Some(k) = h.rule.and_then(|id| rules.groups.iter().position(|g| g.id == id)) {
        let categories = h.labels.categories.clone();
        let label = |c: u32| h.labels.category(c);
        let id = rules.groups[k].id;
        let users = used(&rules, id);
        let here = h.shapes.iter().filter(|e| !e.deleted && rules.areas.get(&e.now.area).is_some_and(|a| a.group == id)).count();
        let g = &mut rules.groups[k];
        ui.heading(g.label());
        if !g.name_ko.is_empty() {
            ui.small(format!("{} — {}", g.name_ko, g.desc_ko));
        }
        egui::Grid::new("housing rule fields").num_columns(2).show(ui, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(250.0));
            ui.end_row();
            ui.label("Description");
            ui.add(egui::TextEdit::multiline(&mut g.desc).desired_width(250.0).desired_rows(2));
            ui.end_row();
            ui.label("Houseless only");
            ui.checkbox(&mut g.houseless, "only players who own no building").on_hover_text("houseless: the server refuses players who already own a building");
            ui.end_row();
            ui.label("Not if owning");
            let shown = match g.existing_category_id {
                None | Some(0) => "–".to_string(),
                Some(c) => label(c as u32),
            };
            egui::ComboBox::from_id_salt("existing category").width(250.0).selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut g.existing_category_id, Some(0), "–");
                for (c, name) in &categories {
                    ui.selectable_value(&mut g.existing_category_id, Some(*c as i64), format!("{c} {name}"));
                }
            })
            .response
            .on_hover_text("existing_category_id: refuse players who already own a building of this category");
            ui.end_row();
            ui.label("Can extend");
            ui.checkbox(&mut g.can_extend, "pay more tax to keep the building").on_hover_text("can_extend off: the building expires instead (the beginner plots)");
            ui.end_row();
            ui.label("Tax delay");
            ui.add(egui::DragValue::new(&mut g.tax_delay_weeks).range(0..=52).suffix(" week(s)")).on_hover_text("allowed_tax_delay_week (client data; the server does not use it)");
            ui.end_row();
            ui.label("Doodad");
            ui.horizontal(|ui| {
                let mut on = g.doodad_id.is_some();
                if ui.checkbox(&mut on, "").changed() {
                    g.doodad_id = on.then_some(g.doodad_id.unwrap_or(0));
                }
                if let Some(d) = &mut g.doodad_id {
                    ui.add(egui::DragValue::new(d).range(0..=i64::MAX));
                }
            })
            .response
            .on_hover_text("doodad_id (client data; the server does not use it)");
            ui.end_row();
        });
        ui.label("Allowed building categories");
        let mut remove = None;
        egui::Grid::new("housing rule categories").num_columns(3).striped(true).show(ui, |ui| {
            ui.strong("category");
            ui.strong("max");
            ui.label("");
            ui.end_row();
            for (j, c) in g.categories.iter_mut().enumerate() {
                egui::ComboBox::from_id_salt(("rule category", j)).width(260.0).selected_text(label(c.category)).show_ui(ui, |ui| {
                    for (id, name) in &categories {
                        ui.selectable_value(&mut c.category, *id, format!("{id} {name}"));
                    }
                });
                let mut on = c.max.is_some();
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut on, "").on_hover_text("max_construct_count (client data; the server does not check it)").changed() {
                        c.max = on.then_some(c.max.unwrap_or(1));
                    }
                    if let Some(m) = &mut c.max {
                        ui.add(egui::DragValue::new(m).range(0..=99));
                    }
                });
                if ui.small_button("✖").clicked() {
                    remove = Some(j);
                }
                ui.end_row();
            }
        });
        if let Some(j) = remove {
            g.categories.remove(j);
        }
        let free = categories.iter().map(|c| c.0).find(|c| !g.categories.iter().any(|x| x.category == *c));
        if ui.add_enabled(free.is_some(), egui::Button::new("Add category")).clicked()
            && let Some(c) = free
        {
            g.categories.push(Category { id: 0, category: c, max: None });
        }
        ui.small(format!("{users} housing area(s) use this rule set ({here} zone(s) in this world)."));
    } else {
        ui.weak("Select a rule set.");
    }
    if Some(&rules) != h.rules.as_ref() {
        // typing and number drags are one step; a click (checkbox, combo, button) is its own
        if ui.ctx().input(|i| i.pointer.any_released()) {
            h.snapshot();
        } else {
            h.snapshot_for("rules");
        }
        h.rules = Some(rules);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<Objects>\n <Entity Name=\"terrorhousing1\" Pos=\"232.48621,782.39746,103.75372\" EntityClass=\"AreaShape\" EntityId=\"5313\" EntityGuid=\"48dce850c77cd68f\" Layer=\"Main\" cellX=\"1\" cellY=\"0\">\n  <Area Id=\"0\" Group=\"1\" Proximity=\"0\" Priority=\"0\" value1=\"212\" value2=\"0\" flags=\"0\" Height=\"0\">\n   <Points>\n    <Point Pos=\"0,0,0\"/>\n    <Point Pos=\"-4.3514404,-2.2819214,-0.16874695\"/>\n    <Point Pos=\"0.090454102,-76.547913,0.55258942\"/>\n   </Points>\n   <TimeOfDay />\n  </Area>\n </Entity>\n</Objects>\n";

    #[test]
    fn numbers_as_the_original_editor() {
        for s in ["-4.3514404", "232.48621", "0.090454102", "1", "-0.16874695", "1024", "12345.678", "0.5"] {
            assert_eq!(num(s.parse().unwrap()), s);
        }
        assert_eq!(num(0.0), "0");
        assert_eq!(num(-0.0), "0");
    }

    #[test]
    fn parse_edit_write() {
        let (mut e, eol) = parse_file(SAMPLE, 221, (21, 26));
        assert_eq!((e.len(), eol), (1, "\n"));
        let s = &e[0].now;
        assert_eq!((s.name.as_str(), s.area, s.points.len()), ("terrorhousing1", 212, 3));
        assert_eq!(s.pos, Vec3::new(22.0 * 1024.0, 26.0 * 1024.0, 0.0) + vec3("232.48621,782.39746,103.75372").unwrap());
        assert_eq!(write_file(&e, (21, 26), eol, false), SAMPLE);
        assert_eq!(write_file(&e, (21, 26), eol, true), SAMPLE);
        // a moved corner: only its line changes
        e[0].now.points[2].x += 1.0;
        let text = write_file(&e, (21, 26), eol, false);
        assert!(text.contains("<Point Pos=\"1.0904541,-76.547913,0.55258942\"/>"), "{text}");
        assert_eq!(text.lines().count(), SAMPLE.lines().count());
        assert!(e[0].changed());
        // the whole zone moved past the cell edge: new Pos and cell, same corners
        e[0].now.pos.x += 1000.0;
        let text = write_file(&e, (21, 26), eol, false);
        assert!(text.contains("cellX=\"2\""), "{text}");
        let (back, _) = parse_file(&text, 221, (21, 26));
        assert!((back[0].now.pos - e[0].now.pos).length() < 0.01);
        assert!(text.contains("<Point Pos=\"-4.3514404,-2.2819214,-0.16874695\"/>"));
        // deleting the only zone leaves the empty file
        e[0].deleted = true;
        assert_eq!(write_file(&e, (21, 26), eol, false), "<Objects />\n");
    }

    #[test]
    fn polygon_tests() {
        let s = Shape { name: "a".into(), area: 1, pos: Vec3::new(100.0, 100.0, 0.0), points: vec![Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 10.0, 0.0), Vec3::new(0.0, 10.0, 0.0)] };
        assert!(s.contains(105.0, 105.0));
        assert!(!s.contains(95.0, 105.0));
        assert_eq!(s.size(), 100.0);
        assert_eq!(s.distance_to(Vec3::new(115.0, 105.0, 0.0)), 5.0);
        assert_eq!(s.nearest_edge(Vec3::new(105.0, 112.0, 0.0)), Some(2));
    }

    #[test]
    fn world_zones() {
        let xml = "<World>\n <ZoneList>\n  <Zone name=\"a\" id=\"221\" originX=\"21\" originY=\"26\">\n   <cellList>\n    <cell x=\"22\" y=\"26\">\n     <sectorList>\n      <sector x=\"3\" y=\"12\"/>\n";
        let w = WorldZones::parse(xml);
        assert_eq!(w.origins[&221], (21, 26));
        assert_eq!(w.zone_at(22.0 * 1024.0 + 3.5 * 64.0, 26.0 * 1024.0 + 12.2 * 64.0), Some(221));
        assert_eq!(w.zone_at(22.0 * 1024.0, 26.0 * 1024.0), None);
    }

    #[test]
    fn create_delete_save() {
        let dir = std::env::temp_dir().join(format!("aa-editor-housing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut h = Housing::new("w");
        h.loaded = true;
        h.zones = WorldZones::parse("<Zone id=\"7\" originX=\"1\" originY=\"2\">\n<cell x=\"1\" y=\"2\">\n<sector x=\"0\" y=\"0\"/>\n");
        let mut rules = Rules::default();
        rules.groups.push(Group { id: 1, ..default() });
        h.rules = Some(rules.clone());
        h.rules_saved = rules;
        let at = Vec3::new(1024.0 + 10.0, 2048.0 + 10.0, 50.0);
        assert!(h.create(&[at, at + Vec3::X], 1).is_err());
        let i = h.create(&[at, at + Vec3::new(20.0, 0.0, 1.0), at + Vec3::new(20.0, 20.0, 0.0)], 1).unwrap();
        assert_eq!((h.shapes[i].zone, h.shapes[i].now.area), (7, 1));
        assert_eq!(h.changed_count(), 2, "the zone and its area row");
        let server = dir.join("srv");
        assert_eq!(h.save_files(&dir, Some(&server)).unwrap(), 1);
        let text = std::fs::read_to_string(dir.join(file_rel("w", 7))).unwrap();
        assert!(text.starts_with("<Objects>\n <Entity Name=\"LevelDesignShape_7_editor_1\" Pos=\"10,10,50\" EntityClass=\"AreaShape\" EntityId=\"1\""), "{text}");
        assert!(text.contains("<Point Pos=\"20,0,1\"/>") && text.contains("value1=\"1\""));
        assert_eq!(std::fs::read_to_string(server.join("game").join(file_rel("w", 7))).unwrap(), text);
        assert_eq!(h.shapes.iter().filter(|e| e.changed()).count(), 0);
        let (back, _) = parse_file(&text, 7, (1, 2));
        assert_eq!(back[0].now, h.shapes[0].now);
        // delete: the file empties, the area row goes, the server's first version is kept
        h.delete(0);
        assert!(h.rules.as_ref().unwrap().areas.is_empty());
        h.save_files(&dir, Some(&server)).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(file_rel("w", 7))).unwrap(), "<Objects />\n");
        assert!(h.shapes.is_empty());
        let orig = server.join("game").join(file_rel("w", 7)).with_file_name("housing_area.xml.editor-orig");
        assert_eq!(std::fs::read_to_string(orig).unwrap(), text);
        // undo was cleared by the save; redo/undo of an edit
        h.create(&[at, at + Vec3::X * 5.0, at + Vec3::Y * 5.0], 1).unwrap();
        assert!(h.undo());
        assert!(h.shapes.is_empty());
        assert!(h.redo());
        assert_eq!(h.shapes.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every housing file of the pak (all worlds), regenerated from its parsed shapes, is the same bytes.
    #[test]
    fn pak_files_round_trip() {
        let pak = aa_pak::Pak::global();
        let names: Vec<String> = pak.names().filter(|n| n.ends_with("/housing_area.xml")).map(str::to_string).collect();
        assert!(names.len() >= 100, "{} housing files", names.len());
        let mut shapes = 0;
        for name in &names {
            let bytes = crate::navmesh::original(name).unwrap();
            let text = String::from_utf8(bytes).unwrap();
            let (e, eol) = parse_file(&text, 0, (3, 4));
            shapes += e.len();
            assert_eq!(write_file(&e, (3, 4), eol, true), text, "{name}");
            for s in &e {
                assert!(s.now.points.len() >= 3, "{name}: {}", s.now.name);
            }
        }
        assert!(shapes > 350, "{shapes} shapes");
    }

    #[test]
    fn main_world_zones_and_areas() {
        let pak = aa_pak::Pak::global();
        let w = WorldZones::parse(&pak.read_string("worlds/main_world/world.xml").unwrap());
        assert!(w.origins.len() > 100 && w.origins.contains_key(&221));
        // terrorhousing1 (area 212) lies in zone 221, at its file's position
        let text = String::from_utf8(crate::navmesh::original(&format!("game/{}", file_rel("main_world", 221))).unwrap()).unwrap();
        let (e, _) = parse_file(&text, 221, w.origins[&221]);
        let c = e[0].now.centre();
        assert_eq!(w.zone_at(c.x, c.y), Some(221));
    }

    fn schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE housing_areas (id int PRIMARY KEY, name TEXT, housing_group_id INT, comments TEXT);
             CREATE TABLE housing_groups (id int PRIMARY KEY, name TEXT, \"desc\" TEXT, doodad_id INT, houseless NUM, existing_category_id INT, allowed_tax_delay_week INT, can_extend NUM);
             CREATE TABLE housing_group_categories (id int PRIMARY KEY, housing_group_id INT, category_id INT, max_construct_count INT);
             CREATE TABLE localized_texts (id INTEGER, tbl_name TEXT, tbl_column_name TEXT, idx INTEGER, ko TEXT, en_us TEXT);
             INSERT INTO housing_groups VALUES (1, '일반', '설명', 6225, 'f', 0, 1, 't'), (2, 'b', '', NULL, 't', 16, 1, 't');
             INSERT INTO housing_group_categories VALUES (1, 1, 1, 0), (2, 1, 8, 3), (3, 2, 16, 1);
             INSERT INTO housing_areas VALUES (212, 's_crescent_sea', 1, 'terrorhousing1'), (213, 's_crescent_sea', 2, 'x');
             INSERT INTO localized_texts VALUES (10, 'housing_groups', 'name', 1, '일반', 'General Housing Province');",
        )
        .unwrap();
    }

    #[test]
    fn rules_read_edit_write() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        let before = read_rules(&conn).unwrap();
        assert_eq!(before.groups.len(), 2);
        let g1 = &before.groups[0];
        assert_eq!((g1.name.as_str(), g1.doodad_id, g1.houseless, g1.can_extend, g1.categories.len()), ("General Housing Province", Some(6225), false, true, 2));
        assert_eq!(before.groups[1].doodad_id, None);
        let mut now = before.clone();
        assert_eq!(now.changes(&before), 0);
        // edit group 1: rename, houseless, drop category 8, add 11
        let g = &mut now.groups[0];
        g.name = "Plots".into();
        g.houseless = true;
        g.categories.retain(|c| c.category != 8);
        g.categories.push(Category { id: 0, category: 11, max: Some(2) });
        // a new group 3 with one category, an area moved to it, area 213 deleted, a new area
        now.groups.push(Group { id: 3, name: "New".into(), can_extend: false, categories: vec![Category { id: 0, category: 1, max: None }], ..default() });
        now.areas.get_mut(&212).unwrap().group = 3;
        now.areas.remove(&213);
        now.areas.insert(405, AreaRow { id: 405, name: "z".into(), group: 3, comments: "LevelDesignShape_1_editor_1".into() });
        // group 2 deleted
        now.groups.retain(|g| g.id != 2);
        assert_eq!(now.changes(&before), 6);
        write_rules(&mut conn, &before, &now).unwrap();
        let after = read_rules(&conn).unwrap();
        assert_eq!(after.groups.len(), 2);
        let g1 = after.group(1).unwrap();
        assert_eq!((g1.name.as_str(), g1.houseless, g1.name_ko.as_str()), ("Plots", true, "일반"));
        assert_eq!(g1.categories.iter().map(|c| (c.category, c.max)).collect::<Vec<_>>(), vec![(1, Some(0)), (11, Some(2))]);
        let g3 = after.group(3).unwrap();
        assert_eq!((g3.name.as_str(), g3.can_extend, g3.categories.len()), ("New", false, 1));
        assert_eq!(after.areas.keys().copied().collect::<Vec<_>>(), vec![212, 405]);
        assert_eq!(after.areas[&212].group, 3);
        let stored: String = conn.query_row("SELECT houseless FROM housing_groups WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, "t");
        let cats: i64 = conn.query_row("SELECT COUNT(*) FROM housing_group_categories WHERE housing_group_id = 2", [], |r| r.get(0)).unwrap();
        assert_eq!(cats, 0);
        // read back as the new "before": no changes
        assert_eq!(after.changes(&read_rules(&conn).unwrap()), 0);
    }

    #[test]
    fn shared_database_rules_read() {
        let path = Path::new("D:/aaemu-live/rs-game/Data/compact.sqlite3");
        if !path.is_file() {
            return;
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let r = read_rules(&conn).unwrap();
        assert_eq!(r.groups.len(), 15);
        assert_eq!(r.group(1).unwrap().name, "General Housing Province");
        assert!(r.areas.len() > 400);
        assert_eq!(r.areas[&212].group, 5);
        let l = read_labels(&conn);
        assert!(l.categories.len() >= 17);
        assert_eq!(l.zone_groups.get(&221).map(String::as_str), Some("s_crescent_sea"));
    }
}
