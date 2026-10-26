//! The world picker: the worlds of the pak (`worlds/<name>/world.g`) in the menu bar. Picking one restarts the
//! editor on it (every tool keeps per-world state), with the same folders mapped to the new world, after asking
//! about unsaved changes.

use std::path::{Path, PathBuf};

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::spawns::SpawnStore;
use crate::terrain_edit::Heights;
use crate::housing::Housing;
use crate::paths::AiPaths;
use crate::world_objects::WorldObjects;
use crate::{Editor, Request};

/// The bevy lane's source tree, the default home of the server data files.
pub const LANE_DATA: &str = "D:/aaemu-wt/bevy/rust/crates/aaemu-game/app/Data";

/// Flags kept when restarting on another world (`--spawn-dir` / `--mirror` are mapped to it).
const KEEP_VALUES: [&str; 12] = [
    "--db",
    "--path-dir",
    "--overlay",
    "--mode",
    "--client-exe",
    "--lane-dir",
    "--lane-name",
    "--lane-ports",
    "--play-account",
    "--play-character",
    "--play-password-file",
    "--client-arg",
];
const KEEP_FLAGS: [&str; 3] = ["--read-only-db", "--dry-run", "--no-fog"];

#[derive(Resource, Default)]
pub struct Worlds {
    pub names: Vec<String>,
    /// A switch waiting for the unsaved-changes answer, or for the saves it asked for.
    pending: Option<Pending>,
}

enum Pending {
    Ask(String),
    AfterSave(String, u32),
}

pub fn plugin(app: &mut App) {
    let mut names: Vec<String> = aa_pak::Pak::global()
        .list("worlds/")
        .filter_map(|n| {
            let n = aa_pak::normalize(n);
            let rest = n.strip_prefix("game/").unwrap_or(&n).strip_prefix("worlds/")?;
            let (name, file) = rest.split_once('/')?;
            (file == "world.g" && name != "login2").then(|| name.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    // main_world first
    if let Some(i) = names.iter().position(|n| n == "main_world") {
        let m = names.remove(i);
        names.insert(0, m);
    }
    app.insert_resource(Worlds { names, pending: None }).add_systems(EguiPrimaryContextPass, switch.after(crate::ui::panels));
    // check: `--switch-world <name>` asks for the switch once the editor runs
    if let Some(world) = crate::arg("--switch-world") {
        app.add_systems(Update, move |mut editor: ResMut<Editor>, time: Res<Time>, mut done: Local<bool>| {
            if !*done && time.elapsed_secs() > 8.0 {
                *done = true;
                editor.requests.push(Request::SwitchWorld(world.clone()));
            }
        });
    }
}

/// The server `Data` folders to read shared files from (`world_spawns.json`, `Portal/*.json`): the one above the
/// spawn folder, then the lane's source tree.
pub fn data_dirs(spawn_dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = spawn_dir.parent().and_then(Path::parent).map(Path::to_path_buf).into_iter().collect();
    v.push(PathBuf::from(LANE_DATA));
    v
}

/// The server's start point of `world` (`Worlds/world_spawns.json`), cry space.
pub fn server_start(world: &str, data: &[PathBuf]) -> Option<Vec3> {
    for d in data {
        let Ok(text) = std::fs::read_to_string(d.join("Worlds/world_spawns.json")) else { continue };
        let text = crate::jsonc::strip_comments(text.trim_start_matches('\u{feff}'));
        let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for e in list {
            if e["Name"].as_str() == Some(world) {
                let p = &e["SpawnPosition"];
                return Some(Vec3::new(p["X"].as_f64()? as f32, p["Y"].as_f64()? as f32, p["Z"].as_f64()? as f32));
            }
        }
    }
    None
}

/// Where the camera starts in `world` without `--pos`: Nuia's new-character spawn in main_world, else the server's
/// start point, else the middle of the world (cry space, camera position).
pub fn start_camera(world: &str, data: &[PathBuf]) -> Vec3 {
    if world == "main_world" {
        return Vec3::new(15560.0, 15350.0, 160.0);
    }
    if let Some(p) = server_start(world, data) {
        return p + Vec3::new(0.0, -30.0, 30.0);
    }
    let w = aa_world::World::load(world).unwrap_or_default();
    let (x, y) = (w.cells_x.max(1) as f32 * 512.0, w.cells_y.max(1) as f32 * 512.0);
    let ground = aa_world::TerrainQuery::new(world).height_at(x, y).unwrap_or(100.0);
    Vec3::new(x, y - 60.0, ground + 60.0)
}

/// `dir` for another world: `<parent>/<world>` (the server's `Data/Worlds/<world>` layout).
fn map_dir(dir: &Path, world: &str) -> PathBuf {
    dir.parent().map(|p| p.join(world)).unwrap_or_else(|| PathBuf::from(world))
}

/// The command line for the editor on `world`, from this one's.
pub fn relaunch_args(args: &[String], world: &str, size: Option<(f32, f32)>) -> Vec<String> {
    let mut out = vec!["--world".to_string(), world.to_string()];
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        let value = args.get(i + 1);
        if KEEP_FLAGS.contains(&a) {
            out.push(a.to_string());
        } else if let Some(v) = value.filter(|_| KEEP_VALUES.contains(&a)) {
            out.extend([a.to_string(), v.clone()]);
            i += 1;
        } else if let Some(v) = value.filter(|_| a == "--spawn-dir" || a == "--mirror") {
            let mapped = map_dir(Path::new(v), world);
            if a == "--spawn-dir" || mapped.is_dir() {
                out.extend([a.to_string(), mapped.display().to_string()]);
            }
            i += 1;
        } else if value.is_some_and(|v| !v.starts_with("--")) && a.starts_with("--") {
            i += 1; // a dropped flag's value
        }
        i += 1;
    }
    if let Some((w, h)) = size {
        out.extend(["--size".to_string(), format!("{}x{}", w.round(), h.round())]);
    }
    out
}

/// Copies a folder's files (one level, the spawn files) into a new folder.
fn copy_files(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        if e.file_type()?.is_file() {
            std::fs::copy(e.path(), to.join(e.file_name()))?;
        }
    }
    Ok(())
}

fn relaunch(editor: &mut Editor, world: &str, size: Option<(f32, f32)>, exit: &mut MessageWriter<AppExit>) {
    let args: Vec<String> = std::env::args().collect();
    let new = relaunch_args(&args, world, size);
    // a spawn folder of our own (a scratch copy) that has no copy of this world yet: start it from the lane's files
    if let Some(i) = new.iter().position(|a| a == "--spawn-dir") {
        let dir = PathBuf::from(&new[i + 1]);
        let source = Path::new(LANE_DATA).join("Worlds").join(world);
        if !dir.exists()
            && source.is_dir()
            && let Err(e) = copy_files(&source, &dir)
        {
            editor.status(format!("cannot copy the spawn files of {world} to {}: {e}", dir.display()));
            return;
        }
    }
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return editor.status(format!("cannot restart: {e}")),
    };
    match std::process::Command::new(exe).args(&new).spawn() {
        Ok(_) => {
            info!("switching to {world}: {}", new.join(" "));
            exit.write(AppExit::Success);
        }
        Err(e) => editor.status(format!("cannot restart on {world}: {e}")),
    }
}

fn unsaved(store: &SpawnStore, heights: &Heights, objects: &WorldObjects, paths: Option<&AiPaths>, housing: Option<&Housing>) -> usize {
    store.changed_count()
        + heights.unsaved.len()
        + objects.cells.values().filter(|c| !c.unsaved.is_empty()).count()
        + paths.map_or(0, AiPaths::changed_count)
        + housing.map_or(0, Housing::changed_count)
}

/// The world menu of the menu bar.
pub fn menu(ui: &mut egui::Ui, editor: &mut Editor, worlds: &Worlds) {
    ui.menu_button(format!("🌐 {}", editor.world), |ui| {
        ui.label("Restart the editor on:");
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for name in &worlds.names {
                if ui.add_enabled(*name != editor.world, egui::Button::selectable(*name == editor.world, name.as_str())).clicked() {
                    editor.requests.push(Request::SwitchWorld(name.clone()));
                    ui.close();
                }
            }
        });
    });
}

fn switch(
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    mut worlds: ResMut<Worlds>,
    store: Res<SpawnStore>,
    heights: Res<Heights>,
    objects: Res<WorldObjects>,
    paths: Option<Res<AiPaths>>,
    housing: Option<Res<Housing>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    let size = window.single().ok().map(|w| (w.resolution.width(), w.resolution.height()));
    if let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::SwitchWorld(_))) {
        let Request::SwitchWorld(world) = editor.requests.remove(i) else { return Ok(()) };
        if unsaved(&store, &heights, &objects, paths.as_deref(), housing.as_deref()) > 0 {
            worlds.pending = Some(Pending::Ask(world));
        } else {
            relaunch(&mut editor, &world, size, &mut exit);
        }
    }
    match worlds.pending.take() {
        Some(Pending::AfterSave(world, frames)) => {
            let saving = editor.requests.iter().any(|r| matches!(r, Request::SaveSpawns | Request::SaveTerrain | Request::SaveObjects | Request::SavePaths | Request::SaveHousing));
            if saving || frames < 2 {
                worlds.pending = Some(Pending::AfterSave(world, frames + 1));
            } else if unsaved(&store, &heights, &objects, paths.as_deref(), housing.as_deref()) > 0 {
                editor.status(format!("not switching to {world}: some changes did not save (see above)"));
            } else {
                relaunch(&mut editor, &world, size, &mut exit);
            }
        }
        Some(Pending::Ask(world)) => {
            let n = unsaved(&store, &heights, &objects, paths.as_deref(), housing.as_deref());
            let mut answer = None;
            egui::Modal::new("switch world".into()).show(contexts.ctx_mut()?, |ui| {
                ui.heading(format!("Switch to {world}?"));
                ui.label(format!("{} has {n} unsaved change(s) (spawns, terrain cells, object cells, AI paths, housing zones).", editor.world));
                ui.horizontal(|ui| {
                    if ui.button("Save all and switch").clicked() {
                        answer = Some(1);
                    }
                    if ui.button("Discard and switch").clicked() {
                        answer = Some(2);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(0);
                    }
                });
            });
            match answer {
                Some(1) => {
                    editor.requests.extend([Request::SaveSpawns, Request::SaveTerrain, Request::SaveObjects, Request::SavePaths, Request::SaveHousing]);
                    worlds.pending = Some(Pending::AfterSave(world, 0));
                }
                Some(2) => relaunch(&mut editor, &world, size, &mut exit),
                Some(_) => {}
                None => worlds.pending = Some(Pending::Ask(world)),
            }
        }
        None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn relaunch_maps_folders_and_drops_one_shot_flags() {
        let args = s(&[
            "aa-editor", "--world", "main_world", "--spawn-dir", "C:/tmp/Worlds/main_world", "--read-only-db", "--pos",
            "1,2", "--overlay", "C:/tmp/ov", "--select", "7", "--play",
        ]);
        let out: Vec<String> = relaunch_args(&args, "instance_library", Some((1600.0, 900.0))).iter().map(|a| a.replace('\\', "/")).collect();
        assert_eq!(
            out,
            s(&[
                "--world", "instance_library", "--spawn-dir", "C:/tmp/Worlds/instance_library", "--read-only-db", "--overlay",
                "C:/tmp/ov", "--size", "1600x900",
            ])
        );
    }

    #[test]
    fn worlds_come_from_the_pak() {
        if !aa_pak::Pak::global().exists("worlds/main_world/world.g") {
            return;
        }
        let mut app = App::new();
        plugin(&mut app);
        let w = app.world().resource::<Worlds>();
        assert_eq!(w.names[0], "main_world");
        assert!(w.names.iter().any(|n| n == "instance_library"));
        assert!(!w.names.iter().any(|n| n == "login2"));
    }
}
