//! Painting a surface type a cell does not list yet.
//!
//! A heightmap sample stores a slot into its cell's `leveldata.xml` `<SurfaceTypes>` (at most 31; 31 itself is the
//! hole). The world's layer library (`aa_world::terrain_layers::LayerLibrary`) is built from the surface types of
//! *every* cell, so any material some cell of the world uses already has its textures in the layer arrays. Adding one
//! to another cell therefore needs no new textures:
//!
//! 1. the catalogue is every distinct `DetailMaterial` of the world's cells, each with a donor cell, slot and its
//!    `<SurfaceType .../>` element text (scales and the other attributes come with it);
//! 2. when the paint brush (with "add missing surface types" on) reaches a cell without the material, the donor's
//!    element is appended to that cell's `<SurfaceTypes>` (new slot = the old count) and the `leveldata.xml` is
//!    written to the pak overlay at once (an unused extra surface type is harmless if the stroke is undone);
//! 3. the cell's loaded terrain material gets the donor's shader table entry for the new slot
//!    ([`copy_slot`]: the same projections, with the uv offsets recomputed for this cell's origin), so the new
//!    material shows immediately; after a restart the library reads the overlay's `leveldata.xml` itself.

use std::collections::HashMap;

use aa_world::terrain_layers::{SURFACE_TYPES, TerrainLayers, TerrainMaterial, TerrainParams, parse_surface_types};
use aa_world::stream::{TerrainPatch, WorldConfig, WorldInfo};
use aa_world::CELL_SIZE;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::terrain_edit::{Brush, BrushKind};
use crate::terrain_paint::SurfaceTables;
use crate::{Editor, Mode};

/// `aa_world::terrain_layers` layout: projections per surface type and vectors per projection entry.
const PROJECTIONS: usize = 3;
const ENTRY: usize = 4;

/// A material some cell uses: where to copy it from.
#[derive(Clone, Debug)]
pub struct Donor {
    pub material: String,
    pub cell: (i32, i32),
    pub slot: u8,
    /// The `<SurfaceType .../>` element as written in the donor's `leveldata.xml`.
    pub element: String,
}

/// State of the "add missing surface types" option (kept inside [`SurfaceTables`] so the brush sees it).
#[derive(Default)]
pub struct AddState {
    pub enabled: bool,
    /// Every material of the world, sorted by name (`None` until first needed).
    pub catalogue: Option<Vec<Donor>>,
    /// Slots added to cells: (cell, slot, donor). The shader tables of their loaded materials follow these.
    pub added: Vec<((i32, i32), u8, Donor)>,
    /// Entries of `added` whose material has not been updated yet.
    pending: Vec<usize>,
    pub status: String,
    filter: String,
    /// The editor's pak overlay (where `leveldata.xml` goes).
    pub overlay: std::path::PathBuf,
}

pub fn plugin(app: &mut App) {
    let overlay = app.world().get_resource::<Editor>().map(|e| e.paths.overlay.clone());
    app.init_resource::<SurfaceTables>();
    if let Some(o) = overlay {
        app.world_mut().resource_mut::<SurfaceTables>().add.overlay = o;
    }
    app.add_systems(Update, (build_catalogue, apply_added).chain().after(crate::tools::update_cursor))
        .add_systems(EguiPrimaryContextPass, window);
}

fn leveldata_path(world: &str, c: (i32, i32)) -> String {
    format!("{}/client/leveldata.xml", aa_world::world::cell_dir(world, c.0, c.1))
}

/// `(material key, element text)` of every `<SurfaceType>` of a `leveldata.xml`, in slot order.
pub fn elements(xml: &str) -> Vec<(String, String)> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    doc.descendants()
        .filter(|n| n.has_tag_name("SurfaceType"))
        .map(|n| (key(n.attribute("DetailMaterial").unwrap_or("")), xml[n.range()].to_string()))
        .collect()
}

fn key(path: &str) -> String {
    let p = path.trim().replace('\\', "/").to_ascii_lowercase();
    p.strip_prefix("game/").map(str::to_string).unwrap_or(p)
}

/// `xml` with `element` appended as the last `<SurfaceType>` (same indentation as the one before). `None` when the
/// file has no `<SurfaceTypes>` list or it is full (31 types; 31 is the hole).
pub fn append_element(xml: &str, element: &str) -> Option<String> {
    let count = parse_surface_types(xml).len();
    if count >= SURFACE_TYPES - 1 {
        return None;
    }
    let doc = roxmltree::Document::parse(xml).ok()?;
    let list = doc.descendants().find(|n| n.has_tag_name("SurfaceTypes"))?;
    let last = list.children().rfind(|n| n.has_tag_name("SurfaceType"));
    let (at, indent) = match last {
        Some(n) => {
            let start = n.range().start;
            let line = xml[..start].rfind('\n').map_or(0, |i| i + 1);
            (n.range().end, format!("\n{}", &xml[line..start]))
        }
        None => {
            // `<SurfaceTypes/>` or an empty list: insert right after the opening tag
            let text = &xml[list.range()];
            if let Some(open) = text.strip_suffix("/>") {
                let new = format!("{open}>\n {element}\n</SurfaceTypes>");
                return Some(format!("{}{new}{}", &xml[..list.range().start], &xml[list.range().end..]));
            }
            (list.range().start + text.find('>')? + 1, "\n ".to_string())
        }
    };
    Some(format!("{}{indent}{element}{}", &xml[..at], &xml[at..]))
}

/// The slot of `material` in cell `c`; with the option on, a new one when the cell lacks it.
pub fn slot_or_add(tables: &mut SurfaceTables, world: &str, c: (i32, i32), material: &str) -> Option<u8> {
    if let Some(s) = tables.slot_of(world, c, material) {
        return Some(s);
    }
    if !tables.add.enabled {
        return None;
    }
    let donor = tables.add.catalogue.as_ref()?.iter().find(|d| d.material == material)?.clone();
    let path = leveldata_path(world, c);
    let xml = aa_pak::Pak::global().read_string(&path).ok()?;
    let slot = parse_surface_types(&xml).len();
    let Some(new) = append_element(&xml, &donor.element) else {
        tables.add.status = format!("cell {c:?}: no room for another surface type (31 max)");
        return None;
    };
    if tables.add.overlay.as_os_str().is_empty() {
        return None;
    }
    let file = tables.add.overlay.join(&path);
    let written = std::fs::create_dir_all(file.parent()?).and_then(|_| std::fs::write(&file, &new));
    if let Err(e) = written {
        tables.add.status = format!("cell {c:?}: cannot write {}: {e}", file.display());
        return None;
    }
    let mut table = (*tables.get(world, c)).clone();
    if slot < table.len() {
        table[slot] = Some(material.to_string());
    }
    tables.cells.insert(c, std::sync::Arc::new(table));
    tables.add.status = format!("cell {c:?}: added {} as surface type {slot} (leveldata.xml saved to the overlay)", donor.material);
    info!("{}", tables.add.status);
    tables.add.added.push((c, slot as u8, donor));
    let i = tables.add.added.len() - 1;
    tables.add.pending.push(i);
    Some(slot as u8)
}

/// The layer uv offset of projection `p` for a cell origin (`aa_world::terrain_layers::LayerLibrary::params`).
fn offset(cell: (i32, i32), p: usize, su: f32, sv: f32) -> [f32; 2] {
    let origin = [cell.0 as f64 * CELL_SIZE as f64, cell.1 as f64 * CELL_SIZE as f64];
    let fract = |v: f64| v.rem_euclid(1.0) as f32;
    match p {
        0 => [fract(origin[1] * su as f64), 0.0],
        1 => [fract(origin[0] * su as f64), 0.0],
        _ => [fract(origin[0] * su as f64), fract(origin[1] * sv as f64)],
    }
}

/// Copies surface type `ss` of `src` (built for cell `sc`) into slot `ds` of `dst` (cell `dc`): textures, scales,
/// strengths, colours and specular as they are, uv offsets recomputed for `dc`.
pub fn copy_slot(src: &TerrainParams, ss: usize, dst: &mut TerrainParams, dc: (i32, i32), ds: usize) {
    for p in 0..PROJECTIONS {
        let (i, j) = ((ss * PROJECTIONS + p) * ENTRY, (ds * PROJECTIONS + p) * ENTRY);
        dst.layers[j..j + ENTRY].copy_from_slice(&src.layers[i..i + ENTRY]);
        let (su, sv) = (src.layers[i].z, src.layers[i].w);
        let off = offset(dc, p, su, sv);
        dst.layers[j + 1].x = off[0];
        dst.layers[j + 1].y = off[1];
        let (ks, kd) = (ss * PROJECTIONS + p, ds * PROJECTIONS + p);
        let t = src.normal_uv[ks];
        dst.normal_uv[kd] = if src.layers[i].y >= 0.0 {
            let n = offset(dc, p, su * t.x, sv * t.y);
            Vec4::new(t.x, t.y, n[0] - off[0] * t.x, n[1] - off[1] * t.y)
        } else {
            t
        };
        if src.layers[i + 2].x >= 0.0 {
            let d = offset(dc, p, src.layers[i + 2].y, src.layers[i + 2].z);
            dst.layers[j + 3].x = d[0];
            dst.layers[j + 3].y = d[1];
        }
        dst.specular[kd * 2] = src.specular[ks * 2];
        dst.specular[kd * 2 + 1] = src.specular[ks * 2 + 1];
    }
}

/// Every distinct `DetailMaterial` of the world's cells with its first donor, sorted by file name.
pub fn catalogue(world: &str, cells: impl Iterator<Item = (i32, i32)>) -> Vec<Donor> {
    let started = std::time::Instant::now();
    let pak = aa_pak::Pak::global();
    let mut seen: HashMap<String, Donor> = HashMap::new();
    let mut cells: Vec<(i32, i32)> = cells.collect();
    cells.sort();
    for c in cells {
        let Ok(xml) = pak.read_string(&leveldata_path(world, c)) else { continue };
        for (slot, (material, element)) in elements(&xml).into_iter().enumerate().take(SURFACE_TYPES - 1) {
            if material.is_empty() {
                continue;
            }
            seen.entry(material.clone()).or_insert(Donor { material, cell: c, slot: slot as u8, element });
        }
    }
    let mut list: Vec<Donor> = seen.into_values().collect();
    list.sort_by(|a, b| a.material.rsplit('/').next().cmp(&b.material.rsplit('/').next()));
    info!("surface types: {} materials in the world's cells ({:.0} ms)", list.len(), started.elapsed().as_secs_f32() * 1000.0);
    list
}

/// Fills the catalogue once the option is first used.
fn build_catalogue(editor: Res<Editor>, world: Option<Res<WorldInfo>>, mut tables: ResMut<SurfaceTables>) {
    if tables.add.overlay != editor.paths.overlay {
        tables.add.overlay = editor.paths.overlay.clone();
    }
    if !tables.add.enabled || tables.add.catalogue.is_some() {
        return;
    }
    let Some(world) = world else { return };
    tables.add.catalogue = Some(catalogue(&editor.world, world.0.cells.keys().copied()));
}

/// `--paint-material <text>` (with `--add-surface`): the first catalogue material whose path contains `text`.
pub fn find_material(tables: &mut SurfaceTables, world: &str, text: &str) -> Option<String> {
    if crate::flag("--add-surface") {
        tables.add.enabled = true;
    }
    if tables.add.catalogue.is_none() {
        let w = aa_world::world::World::load(world).ok()?;
        tables.add.catalogue = Some(catalogue(world, w.cells.keys().copied()));
    }
    let text = text.to_ascii_lowercase();
    tables.add.catalogue.as_ref()?.iter().find(|d| d.material.contains(&text)).map(|d| d.material.clone())
}

/// Gives the loaded materials of cells with added slots the donor's shader entry (again after a cell reloads).
fn apply_added(
    mut tables: ResMut<SurfaceTables>,
    layers: Res<TerrainLayers>,
    cfg: Res<WorldConfig>,
    patches: Query<(&TerrainPatch, &MeshMaterial3d<TerrainMaterial>)>,
    added_patches: Query<&TerrainPatch, Added<TerrainPatch>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    if tables.add.added.is_empty() {
        return;
    }
    let reloaded: Vec<(i32, i32)> = added_patches.iter().map(|p| p.cell).collect();
    let add = &mut tables.add;
    for (i, (c, _, _)) in add.added.iter().enumerate() {
        if reloaded.contains(c) && !add.pending.contains(&i) {
            add.pending.push(i);
        }
    }
    if tables.add.pending.is_empty() {
        return;
    }
    let Some(library) = layers.library() else { return };
    let mut keep = Vec::new();
    for i in std::mem::take(&mut tables.add.pending) {
        let (c, slot, donor) = tables.add.added[i].clone();
        let handles: Vec<Handle<TerrainMaterial>> =
            patches.iter().filter(|(p, _)| p.cell == c).map(|(_, m)| m.0.clone()).collect();
        let Some(src) = library.params(donor.cell, cfg.terrain_tint) else { continue };
        let ground = src.layers[(donor.slot as usize * PROJECTIONS + 2) * ENTRY];
        if ground.x < 0.0 {
            tables.add.status = format!("{}: the layer library has no texture for it (dropped when the arrays filled up); it paints but shows the cover colour only", donor.material);
            warn!("surface types: {}", tables.add.status);
        }
        if handles.is_empty() {
            keep.push(i);
            continue;
        }
        let mut done = std::collections::HashSet::new();
        for h in handles {
            if !done.insert(h.id()) {
                continue;
            }
            if let Some(mut m) = materials.get_mut(&h) {
                let mut params = m.extension.params;
                copy_slot(&src, donor.slot as usize, &mut params, c, slot as usize);
                m.extension.params = params;
                debug!("surface types: cell {c:?} slot {slot} <- cell {:?} slot {} (ground layer {:?})", donor.cell, donor.slot, ground);
            }
        }
    }
    tables.add.pending = keep;
}

fn window(mut contexts: EguiContexts, editor: Res<Editor>, mut brush: ResMut<Brush>, mut tables: ResMut<SurfaceTables>) -> Result {
    if editor.mode != Mode::Terrain || editor.playing || brush.kind != BrushKind::Paint {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("Add surface types").default_pos([1580.0, 640.0]).default_width(320.0).default_open(false).show(ctx, |ui| {
        let add = &mut tables.add;
        ui.checkbox(&mut add.enabled, "Paint adds the material to cells that lack it");
        ui.small("The world's layer library already holds every material some cell uses; the cell's leveldata.xml gets the material as a new surface type (max 31) in the pak overlay.");
        if !add.status.is_empty() {
            ui.small(add.status.clone());
        }
        let Some(list) = add.catalogue.as_ref() else {
            if add.enabled {
                ui.label("reading the cells' surface types ...");
            }
            return;
        };
        ui.horizontal(|ui| {
            ui.label("Filter");
            ui.text_edit_singleline(&mut add.filter);
        });
        let filter = add.filter.to_ascii_lowercase();
        let mut pick = None;
        egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
            for d in list.iter().filter(|d| filter.is_empty() || d.material.contains(&filter)) {
                let name = d.material.rsplit('/').next().unwrap_or(&d.material);
                let on = brush.paint.as_deref() == Some(d.material.as_str());
                if ui.selectable_label(on, name).on_hover_text(format!("{}\nfrom cell {:03}_{:03}, slot {}", d.material, d.cell.0, d.cell.1, d.slot)).clicked() {
                    pick = Some(d.material.clone());
                }
            }
        });
        if let Some(m) = pick {
            brush.paint = Some(m);
        }
        ui.small(format!("{} cell(s) got new surface types this session", add.added.iter().map(|a| a.0).collect::<std::collections::HashSet<_>>().len()));
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = "<LevelData>\n <SurfaceTypes>\n  <SurfaceType Name=\"a\" DetailMaterial=\"Game/A\" DetailScaleX=\"2\"/>\n  <SurfaceType Name=\"b\" DetailMaterial=\"game/b\"/>\n </SurfaceTypes>\n</LevelData>\n";

    #[test]
    fn appends_with_the_same_indent() {
        let new = append_element(XML, "<SurfaceType Name=\"c\" DetailMaterial=\"game/c\"/>").unwrap();
        assert!(new.contains("  <SurfaceType Name=\"b\" DetailMaterial=\"game/b\"/>\n  <SurfaceType Name=\"c\" DetailMaterial=\"game/c\"/>\n </SurfaceTypes>"), "{new}");
        let t = parse_surface_types(&new);
        assert_eq!(t.len(), 3);
        assert_eq!(t[2].material, "game/c");
        assert_eq!(elements(&new)[0].0, "a");
        // empty list
        let e = append_element("<L><SurfaceTypes/></L>", "<SurfaceType DetailMaterial=\"x\"/>").unwrap();
        assert_eq!(parse_surface_types(&e).len(), 1, "{e}");
    }

    #[test]
    fn copies_a_slot_with_this_cells_offsets() {
        let mut src = TerrainParams::default();
        src.layers[(2 * PROJECTIONS + 2) * ENTRY] = Vec4::new(3.0, 4.0, 0.37, -0.37);
        src.normal_uv[2 * PROJECTIONS + 2] = Vec4::new(2.0, 2.0, 0.0, 0.0);
        let mut dst = TerrainParams::default();
        copy_slot(&src, 2, &mut dst, (15, 14), 7);
        let j = (7 * PROJECTIONS + 2) * ENTRY;
        assert_eq!(dst.layers[j].x, 3.0);
        let off = offset((15, 14), 2, 0.37, -0.37);
        assert_eq!([dst.layers[j + 1].x, dst.layers[j + 1].y], off);
        assert_eq!(dst.normal_uv[7 * PROJECTIONS + 2].x, 2.0);
        // a slot without textures stays "none"
        assert_eq!(dst.layers[(7 * PROJECTIONS) * ENTRY].x, -1.0);
    }
}

#[cfg(test)]
mod check {
    /// `AA_EDITOR_CHECK_HEIGHTMAP=<heightmap.dat>`: prints how many samples use each surface slot (manual check).
    #[test]
    fn surface_histogram() {
        let Some(path) = std::env::var_os("AA_EDITOR_CHECK_HEIGHTMAP") else { return };
        let cell = aa_world::heightmap::CellHeights::parse(&std::fs::read(path).unwrap()).unwrap();
        let mut n = [0usize; 32];
        for &s in &cell.surface {
            n[s as usize] += 1;
        }
        println!("surface slots: {:?}", n.iter().enumerate().filter(|(_, c)| **c > 0).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod list {
    /// `AA_EDITOR_LIST_SURFACES=<text>`: prints the catalogue materials containing `text` (manual check).
    #[test]
    fn list_catalogue() {
        let Some(text) = std::env::var("AA_EDITOR_LIST_SURFACES").ok() else { return };
        let w = aa_world::world::World::load("main_world").unwrap();
        for d in super::catalogue("main_world", w.cells.keys().copied()).iter().filter(|d| d.material.contains(&text)) {
            println!("{} {:?}", d.material, d.cell);
        }
    }
}
