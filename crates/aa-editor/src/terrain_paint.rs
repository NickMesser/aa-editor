//! Terrain texture painting: the surface types of the cells (`leveldata.xml`), the brush palette with diffuse
//! thumbnails, and the in-place update of a loaded cell's surface map.
//!
//! The five surface bits of a heightmap sample are a **slot** into the cell's own `<SurfaceTypes>` list, so the
//! same slot means different materials in different cells. The brush therefore paints a material (its
//! `DetailMaterial` path): each cell under the brush gets the slot that names it, and cells without that material
//! are left alone (adding a surface type to a cell would change its `leveldata.xml` and the layer library).
//!
//! The terrain shader reads the cell's surface map, an `R8Uint` 513 x 513 image (`terrain_layers::surface_image`)
//! owned by the cell's [`TerrainMaterial`]. Painting overwrites that image's bytes (263 KB, one upload) and touches
//! the material so its bind group is rebuilt: no new material, mesh or texture array per stroke.

use std::collections::HashMap;
use std::sync::Arc;

use aa_cry::material::{Material as CryMaterial, TextureMap};
use aa_cry::texture::{Dds, rgba8_image};
use aa_world::coords::CELL_SIZE;
use aa_world::heightmap::{CellHeights, GRID};
use aa_world::terrain_layers::{SURFACE_TYPES, TerrainMaterial, parse_surface_types};
use bevy::prelude::*;
use bevy_egui::{EguiTextureHandle, EguiUserTextures, egui};

use crate::terrain_edit::Brush;
use crate::tools::Cursor;
use crate::{Editor, Mode};

/// Thumbnails loaded per frame (each reads one `.mtl` and one `.dds`).
const THUMBS_PER_FRAME: usize = 2;
/// Thumbnail side in pixels (the diffuse mip closest to it).
const THUMB_PX: u32 = 64;

/// One surface type of the palette.
#[derive(Clone, Debug)]
pub struct PaletteEntry {
    /// Slot in the palette cell.
    pub slot: u8,
    /// `DetailMaterial` path, lower case with `/`, without `game/` (what the brush paints).
    pub material: String,
    /// Short name: the material's file name.
    pub name: String,
    pub thumb: Option<egui::TextureId>,
}

/// `DetailMaterial` of every slot of a cell (`None`: unused slot), read from `leveldata.xml` once per cell.
#[derive(Resource, Default)]
pub struct SurfaceTables {
    cells: HashMap<(i32, i32), Arc<Vec<Option<String>>>>,
}

impl SurfaceTables {
    pub fn get(&mut self, world: &str, c: (i32, i32)) -> Arc<Vec<Option<String>>> {
        self.cells
            .entry(c)
            .or_insert_with(|| {
                let path = format!("{}/client/leveldata.xml", aa_world::world::cell_dir(world, c.0, c.1));
                let xml = aa_pak::Pak::global().read_string(&path).unwrap_or_default();
                let mut slots: Vec<Option<String>> = parse_surface_types(&xml)
                    .into_iter()
                    .take(SURFACE_TYPES - 1)
                    .map(|e| (!e.material.is_empty()).then(|| material_key(&e.material)))
                    .collect();
                slots.resize(SURFACE_TYPES - 1, None);
                Arc::new(slots)
            })
            .clone()
    }

    /// The slot of `material` in cell `c` (the first one that names it).
    pub fn slot_of(&mut self, world: &str, c: (i32, i32), material: &str) -> Option<u8> {
        self.get(world, c).iter().position(|m| m.as_deref() == Some(material)).map(|i| i as u8)
    }
}

/// A `DetailMaterial` as the cells write it (with or without `game/`, any case, `\` or `/`) as one key.
fn material_key(path: &str) -> String {
    let p = path.trim().replace('\\', "/").to_ascii_lowercase();
    p.strip_prefix("game/").map(str::to_string).unwrap_or(p)
}

/// Loaded thumbnails by material (`None`: no diffuse map).
#[derive(Resource, Default)]
struct Thumbs(HashMap<String, Option<egui::TextureId>>);

pub fn plugin(app: &mut App) {
    app.init_resource::<SurfaceTables>().init_resource::<Thumbs>().add_systems(Update, palette.after(crate::tools::update_cursor));
}

/// Keeps the palette on the surface types of the cell under the cursor (else the camera's).
fn palette(
    editor: Res<Editor>,
    cursor: Res<Cursor>,
    camera: Query<&Transform, With<crate::camera::EditorCamera>>,
    mut brush: ResMut<Brush>,
    mut tables: ResMut<SurfaceTables>,
    mut thumbs: ResMut<Thumbs>,
    mut images: ResMut<Assets<Image>>,
    mut egui_textures: ResMut<EguiUserTextures>,
    mut current: Local<Option<(i32, i32)>>,
) {
    if editor.mode != Mode::Terrain || editor.playing {
        return;
    }
    let at = cursor.ground.or_else(|| camera.single().ok().map(|t| aa_world::bevy_to_cry(t.translation)));
    if let Some(hit) = at {
        let c = ((hit.x / CELL_SIZE).floor() as i32, (hit.y / CELL_SIZE).floor() as i32);
        if *current != Some(c) {
            *current = Some(c);
            let table = tables.get(&editor.world, c);
            brush.palette_cell = Some(c);
            brush.palette = table
                .iter()
                .enumerate()
                .filter_map(|(slot, m)| {
                    let m = m.as_ref()?;
                    // the same material may sit in two slots: list it once
                    (table.iter().position(|o| o.as_ref() == Some(m)) == Some(slot)).then(|| PaletteEntry {
                        slot: slot as u8,
                        material: m.clone(),
                        name: m.rsplit('/').next().unwrap_or(m).to_string(),
                        thumb: None,
                    })
                })
                .collect();
            if brush.paint.is_none() {
                brush.paint = brush.palette.first().map(|e| e.material.clone());
            }
        }
    }
    let mut loaded = 0;
    for entry in brush.palette.iter_mut().filter(|e| e.thumb.is_none()) {
        if let Some(t) = thumbs.0.get(&entry.material) {
            entry.thumb = *t;
            continue;
        }
        if loaded == THUMBS_PER_FRAME {
            break;
        }
        loaded += 1;
        let id = thumbnail(&entry.material).map(|image| egui_textures.add_image(EguiTextureHandle::Strong(images.add(image))));
        thumbs.0.insert(entry.material.clone(), id);
        entry.thumb = id;
    }
}

/// The diffuse map of a terrain layer material (the ground projection of a three-axis one), about 64 px.
fn thumbnail(material: &str) -> Option<Image> {
    let mtl = CryMaterial::load(&format!("{material}.mtl")).ok()?;
    let pick = |m: &CryMaterial| m.texture(&TextureMap::Diffuse).map(|t| t.pak_path());
    // sub-materials x, y, z: the last is the ground
    let path = mtl.sub_materials.iter().rev().find_map(pick).or_else(|| pick(&mtl))?;
    let bytes = aa_pak::Pak::global().read(&path).ok()?;
    let dds = Dds::parse(&bytes).ok()?;
    let mut mip = 0;
    while mip + 1 < dds.mips && (dds.width >> mip) > THUMB_PX {
        mip += 1;
    }
    let (w, h) = ((dds.width >> mip).max(1), (dds.height >> mip).max(1));
    Some(rgba8_image(w, h, &[dds.decode_rgba8(0, mip)], true))
}

/// Writes `cell`'s surface types into the surface map of the cell's material (the map every patch of the cell
/// shares) and marks the material changed.
pub fn update_surface_map(cell: &CellHeights, material: &Handle<TerrainMaterial>, materials: &mut Assets<TerrainMaterial>, images: &mut Assets<Image>) {
    let Some(surface) = materials.get(material).map(|m| m.extension.surface.clone()) else { return };
    let Some(mut image) = images.get_mut(&surface) else { return };
    // a one texel map: the cell has no layer table, nothing to show
    if image.texture_descriptor.size.width as usize != GRID {
        return;
    }
    image.data = Some(cell.surface.clone());
    drop(image);
    // the bind group holds the old texture view: rebuild it
    if let Some(m) = materials.get_mut(material) {
        m.into_inner();
    }
}

/// The surface to restore under an erased hole: the pak's own (when it was no hole), else a non-hole neighbour's,
/// else `fallback`.
pub fn unhole_surface(cell: &CellHeights, original: Option<&CellHeights>, ix: usize, iy: usize, fallback: u8) -> u8 {
    if let Some(o) = original.map(|o| o.surface[iy * GRID + ix]).filter(|s| *s != 31) {
        return o;
    }
    for r in 1..=4isize {
        for (dx, dy) in [(r, 0), (-r, 0), (0, r), (0, -r), (r, r), (-r, -r), (r, -r), (-r, r)] {
            let (x, y) = (ix as isize + dx, iy as isize + dy);
            if x < 0 || y < 0 || x >= GRID as isize || y >= GRID as isize {
                continue;
            }
            let s = cell.surface[y as usize * GRID + x as usize];
            if s != 31 {
                return s;
            }
        }
    }
    fallback
}
