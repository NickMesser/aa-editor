//! Terrain edits for the AAEmu Rust server.
//!
//! The server has no height files of its own. `WorldHeightmap` (`aaemu-game/src/systems/npcs/heightmap.rs`) reads the
//! client's `game/worlds/<world>/cells/XXX_YYY/client/terrain/heightmap.dat` through its `ClientFileManager`, from the
//! sources of `ClientData.Sources` in order (first source that has the file wins; a source is a `game_pak` or a plain
//! directory of unpacked client files). So "exporting to the server" means: write the edited cells' `heightmap.dat`
//! (same writer as the client overlay, [`crate::heightmap_write`]) into a directory laid out like the client's `game/`
//! tree, and list that directory before the `game_pak` in the server's `Config.Local.json`.
//!
//! The navmesh (`paths/XXX_YYY/*.bai`) is baked into the pak and is not derived from heights at run time; the server
//! has no generator for it. With `World.GeoDataMode` on, `AiGeoData::get_height` answers from the navmesh nodes first
//! and only falls back to the heightmap where there is no navmesh, so edited ground is seen by the server only
//! outside navmesh-covered areas (or with `GeoDataMode` off). See `docs/aa-editor.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use aa_world::heightmap::CellHeights;
use bevy::prelude::{App, Res, ResMut, Resource, Update, info};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::terrain_edit::Heights;
use crate::{Editor, Mode};

/// The bevy lane's runtime data folder.
pub const DEFAULT_SERVER_DATA: &str = "D:/aaemu-live-bevy/rs-game/Data";
/// Folder inside the server data folder that holds the exported client files.
pub const OVERLAY_FOLDER: &str = "ClientOverlay";

/// Where the exporter writes.
#[derive(Resource)]
pub struct ServerTerrain {
    /// The server's `Data` folder.
    pub data_dir: PathBuf,
    /// The server's `Config.Local.json` (gets the overlay as first client source).
    pub config: PathBuf,
    /// Add the overlay folder to `ClientData.Sources` while exporting.
    pub register: bool,
    pub status: String,
    requested: bool,
    done: bool,
}

impl ServerTerrain {
    /// The directory the server must list as a client source.
    pub fn source_dir(&self) -> PathBuf {
        self.data_dir.join(OVERLAY_FOLDER)
    }
}

pub fn plugin(app: &mut App) {
    let data_dir = crate::arg("--server-data").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(DEFAULT_SERVER_DATA));
    let config = crate::arg("--server-config").map(PathBuf::from).unwrap_or_else(|| data_dir.join("../Config.Local.json"));
    app.insert_resource(ServerTerrain { data_dir, config, register: true, status: String::new(), requested: false, done: false })
        .add_systems(Update, run_export)
        .add_systems(EguiPrimaryContextPass, window);
}

/// Path of a cell's heightmap inside a client tree, with (`server`) or without the `game/` prefix the pak reader adds.
pub fn heightmap_rel(world: &str, c: (i32, i32), server: bool) -> String {
    let rel = format!("{}/client/terrain/heightmap.dat", aa_world::world::cell_dir(world, c.0, c.1));
    if server { format!("game/{rel}") } else { rel }
}

/// What an export did.
#[derive(Debug, Default)]
pub struct Report {
    pub cells: usize,
    pub backed_up: usize,
    pub registered: bool,
}

/// Writes `edited` cells as the server's client files under `source_dir`. `original(rel)` gives the cell's current
/// `heightmap.dat` (the template the writer keeps its header and quadtree from).
pub fn export(
    world: &str,
    source_dir: &Path,
    edited: &HashMap<(i32, i32), Arc<CellHeights>>,
    original: impl Fn(&str) -> Result<Vec<u8>>,
) -> Result<Report> {
    let mut report = Report::default();
    let mut cells: Vec<_> = edited.keys().copied().collect();
    cells.sort();
    for c in cells {
        let template = original(&heightmap_rel(world, c, false)).with_context(|| format!("original heightmap of cell {c:?}"))?;
        let bytes = crate::heightmap_write::write(&template, &edited[&c])?;
        let path = source_dir.join(heightmap_rel(world, c, true));
        std::fs::create_dir_all(path.parent().context("no parent")?)?;
        report.backed_up += backup(&path)? as usize;
        std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
        report.cells += 1;
    }
    Ok(report)
}

/// Copies an existing file to `<name>.editor-orig` the first time it is overwritten. True when it made the copy.
pub(crate) fn backup(path: &Path) -> Result<bool> {
    if !path.is_file() {
        return Ok(false);
    }
    let mut name = path.file_name().context("no file name")?.to_os_string();
    name.push(".editor-orig");
    let orig = path.with_file_name(name);
    if orig.exists() {
        return Ok(false);
    }
    std::fs::copy(path, orig)?;
    Ok(true)
}

/// Inserts `source_dir` as the first entry of `ClientData.Sources` of the JSON text `config` (text edit, so the
/// file's formatting stays). `None` when it is already listed.
pub fn with_source(config: &str, source_dir: &Path) -> Result<Option<String>> {
    let entry = source_dir.to_string_lossy().replace('\\', "/");
    let quoted = format!("\"{entry}\"");
    if config.contains(&quoted) {
        return Ok(None);
    }
    let key = config.find("\"Sources\"").context("no \"Sources\" in the server config")?;
    let open = key + config[key..].find('[').context("no Sources array")? + 1;
    let rest = config[open..].trim_start();
    let sep = if rest.starts_with(']') { "" } else { ", " };
    Ok(Some(format!("{} {quoted}{sep}{}", &config[..open], config[open..].trim_start())))
}

/// [`with_source`] on the file, with a `.editor-orig` backup the first time. True when the file changed.
pub fn register_source(config: &Path, source_dir: &Path) -> Result<bool> {
    let text = std::fs::read_to_string(config).with_context(|| format!("read {}", config.display()))?;
    let (bom, body) = match text.strip_prefix('\u{feff}') {
        Some(b) => ("\u{feff}", b),
        None => ("", text.as_str()),
    };
    let Some(new) = with_source(body, source_dir)? else { return Ok(false) };
    backup(config)?;
    std::fs::write(config, format!("{bom}{new}"))?;
    Ok(true)
}

fn run_export(editor: Res<Editor>, heights: Res<Heights>, mut st: ResMut<ServerTerrain>) {
    // `--export-server-terrain`: once the scripted sculpt has been saved
    if crate::flag("--export-server-terrain") && !st.done && !heights.edited.is_empty() && heights.unsaved.is_empty() {
        st.requested = true;
    }
    if !st.requested {
        return;
    }
    st.requested = false;
    st.done = true;
    if heights.edited.is_empty() {
        st.status = "nothing edited".into();
        return;
    }
    let source = st.source_dir();
    let result = export(&editor.world, &source, &heights.edited, |rel| Ok(aa_pak::Pak::global().read(rel)?)).and_then(|mut r| {
        if st.register {
            match register_source(&st.config, &source) {
                Ok(changed) => r.registered = changed,
                Err(e) => bail!("exported {} cell(s) but the server config was not updated: {e:#}", r.cells),
            }
        }
        Ok(r)
    });
    let nav = result.is_ok().then(|| crate::navmesh::export(&editor.world, &heights.edited, &source, Some(&editor.paths.overlay)));
    st.status = match result {
        Ok(r) => format!(
            "server terrain: {} cell(s) -> {} ({} backed up{}). {}Restart the server.",
            r.cells,
            source.display(),
            r.backed_up,
            if r.registered { ", client source added to the server config" } else { "" },
            match nav {
                Some(Ok(n)) => format!("Navmesh: {} file(s), {} point(s) moved, {} off the ground kept, {} link(s) blocked over holes. ", n.files, n.moved, n.kept, n.blocked_links),
                Some(Err(e)) => format!("Navmesh update failed: {e:#}. "),
                None => String::new(),
            }
        ),
        Err(e) => format!("server terrain export failed: {e:#}"),
    };
    info!("{}", st.status);
}

fn window(mut contexts: EguiContexts, editor: Res<Editor>, heights: Res<Heights>, mut st: ResMut<ServerTerrain>) -> bevy::prelude::Result {
    if editor.mode != Mode::Terrain {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    egui::Window::new("Server terrain").default_pos([320.0, 760.0]).default_width(330.0).show(ctx, |ui| {
        ui.small(format!("Server data: {}", st.data_dir.display()));
        ui.small(format!("Client source folder: {}", st.source_dir().display()));
        ui.checkbox(&mut st.register, "add the folder to the server's Config.Local.json");
        let edited = heights.edited.len();
        if ui.add_enabled(edited > 0, egui::Button::new(format!("Export terrain to server ({edited} cell(s))"))).clicked() {
            st.requested = true;
        }
        ui.small("Restart the Rust game server afterwards. Navmesh (.bai) heights follow the edit; its triangles are not rebuilt.");
        if !st.status.is_empty() {
            ui.separator();
            ui.small(st.status.clone());
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_world::heightmap::GRID;

    /// Heights of a `heightmap.dat` as the server computes them (`Hmap.Read` + `load_cell_height_map` of
    /// `heightmap.rs`, before the `HeightMaxCoefficient` u16 quantisation): `[ix * 512 + iy]`. `None` for a sector
    /// whose node is not 33x33 (the server's `UpScale` quirk makes those meaningless to compare).
    fn server_view(b: &[u8]) -> Vec<Option<f32>> {
        let u32_at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let f32_at = |at: usize| f32::from_bits(u32_at(at));
        let chunk = u32_at(4) as usize;
        let mut at = if b[0] >= 24 { 32 + 128 } else { 32 };
        // (min x, min y, node size, heights)
        let mut leaves: Vec<(f32, f32, usize, Vec<f32>)> = Vec::new();
        while at < chunk {
            let version = b[at];
            let (min_x, min_y) = (f32_at(at + 4), f32_at(at + 8));
            let (offset, range) = (f32_at(at + 29), f32_at(at + 33));
            let size = u32_at(at + 37) as usize;
            let unk = u32_at(at + 41) as usize;
            let mut p = at + 45;
            let raw: Vec<u16> = (0..size * size).map(|i| u16::from_le_bytes([b[p + i * 2], b[p + i * 2 + 1]])).collect();
            p += size * size * 2 + 4 + 16 + unk + 36;
            at = p;
            if size == 0 {
                continue;
            }
            // NodeCell.init + rescale_to_int + raw_data_to_height
            let (f_min, f_max) = (offset, offset + 0xFFF0 as f32 * range);
            let i_offset = (f_min * 20.0) as i32;
            let i_range = ((f_max - f_min) * 20.0) as i32;
            let i_step = if i_range > 0 { (i_range as i64 + 4095 - 1) / 4095 } else { 1 } as i32;
            let heights = raw
                .iter()
                .map(|&h| {
                    let mut data = h as i32;
                    if version < 7 {
                        let height = f_min + (0xFFF0 & data) as f32 * range;
                        let hdec = (((height - f_min) * 20.0) as i32 / i_step.max(1)) as u16;
                        data = (data & 0xF) | ((hdec as i32) << 4);
                    }
                    0.05 * i_offset as f32 + ((data as u32 >> 4) as i64 * i_step as i64) as f32 * 0.05
                })
                .collect();
            leaves.push((min_x, min_y, size, heights));
        }
        leaves.sort_by(|a, c| a.0.partial_cmp(&c.0).unwrap().then(a.1.partial_cmp(&c.1).unwrap()));
        assert_eq!(leaves.len(), 256, "leaf nodes");
        let mut out = vec![None; 512 * 512];
        for (i, (_, _, size, h)) in leaves.iter().enumerate() {
            if *size != 33 {
                continue;
            }
            let (sx, sy) = (i / 16, i % 16);
            for ux in 0..32 {
                for uy in 0..32 {
                    out[(sx * 32 + ux) * 512 + sy * 32 + uy] = Some(h[ux * 33 + uy]);
                }
            }
        }
        out
    }

    fn scratch() -> PathBuf {
        std::env::temp_dir().join("aa-editor-test-server")
    }

    #[test]
    fn export_reaches_the_server_reader_and_unedited_cells_round_trip() {
        if !aa_world::pak_available() {
            return;
        }
        let world = "main_world";
        let cell = (15, 15);
        let rel = heightmap_rel(world, cell, false);
        let original = aa_pak::Pak::global().read(&rel).unwrap();
        let parsed = CellHeights::parse(&original).unwrap();

        let dir = scratch().join("test");
        let _ = std::fs::remove_dir_all(&dir);
        let read = |r: &str| Ok(aa_pak::Pak::global().read(r)?);

        // unedited: the server reads the same heights (within its own 5 cm * step quantisation)
        let mut edited = HashMap::new();
        edited.insert(cell, Arc::new(parsed.clone()));
        let report = export(world, &dir, &edited, read).unwrap();
        assert_eq!(report.cells, 1);
        let file = dir.join(heightmap_rel(world, cell, true));
        let exported = std::fs::read(&file).unwrap();
        let (before, after) = (server_view(&original), server_view(&exported));
        let (mut compared, mut worst) = (0, 0.0f32);
        for (a, b) in before.iter().zip(&after) {
            if let (Some(a), Some(b)) = (a, b) {
                compared += 1;
                worst = worst.max((a - b).abs());
            }
        }
        eprintln!("unedited round trip: {compared} server samples compared, worst {worst:.3} m");
        assert!(compared > 100_000);
        assert!(worst < 0.6, "worst {worst}");
        // and it agrees with what the editor sees
        let mut worst_editor = 0.0f32;
        for ix in 0..512 {
            for iy in 0..512 {
                if let Some(s) = after[ix * 512 + iy] {
                    worst_editor = worst_editor.max((s - parsed.heights[iy * GRID + ix]).abs());
                }
            }
        }
        eprintln!("server view vs editor heights: worst {worst_editor:.3} m");
        assert!(worst_editor < 0.6, "worst vs editor {worst_editor}");

        // edited: a +25 m plateau shows up in the server's view
        let mut bumped = parsed.clone();
        for iy in 100..140 {
            for ix in 200..260 {
                bumped.heights[iy * GRID + ix] += 25.0;
            }
        }
        edited.insert(cell, Arc::new(bumped.clone()));
        let report = export(world, &dir, &edited, read).unwrap();
        assert_eq!(report.backed_up, 1, "the second export backs up the first file once");
        assert!(file.with_file_name("heightmap.dat.editor-orig").is_file());
        let view = server_view(&std::fs::read(&file).unwrap());
        let (a, b) = (view[230 * 512 + 120].unwrap(), after[230 * 512 + 120].unwrap());
        eprintln!("plateau sample: server {b:.2} m -> {a:.2} m");
        assert!((a - b - 25.0).abs() < 0.6, "{b} -> {a}");
        // a second export does not overwrite the first backup
        assert_eq!(export(world, &dir, &edited, read).unwrap().backed_up, 0);
    }

    #[test]
    fn registers_the_source_once() {
        let config = "\u{feff}{\n  \"Id\": 1,\n  \"ClientData\": { \"Sources\": [ \"D:/AA1.2/game_pak\" ] }\n}\n";
        let dir = Path::new("D:\\x\\Data\\ClientOverlay");
        let new = with_source(config.trim_start_matches('\u{feff}'), dir).unwrap().unwrap();
        assert!(new.contains("\"Sources\": [ \"D:/x/Data/ClientOverlay\", \"D:/AA1.2/game_pak\" ]"), "{new}");
        assert!(with_source(&new, dir).unwrap().is_none());
        let empty = with_source("{\"Sources\": []}", dir).unwrap().unwrap();
        assert_eq!(empty, "{\"Sources\": [ \"D:/x/Data/ClientOverlay\"]}");

        let file = scratch().join("test_config/Config.Local.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, config).unwrap();
        let _ = std::fs::remove_file(file.with_file_name("Config.Local.json.editor-orig"));
        assert!(register_source(&file, dir).unwrap());
        assert!(!register_source(&file, dir).unwrap());
        assert_eq!(std::fs::read_to_string(file.with_file_name("Config.Local.json.editor-orig")).unwrap(), config);
        assert!(std::fs::read_to_string(&file).unwrap().starts_with('\u{feff}'));
    }
}
