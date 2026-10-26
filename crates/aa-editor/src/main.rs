//! AABevy world editor: the real streamed world (`aa-world`) with editing tools on top.
//!
//! ```text
//! cargo run -p aa-editor -- --pos 15578,15382            # Nuia start
//! cargo run -p aa-editor -- --spawn-dir <Data/Worlds/main_world> --db <compact.sqlite3> --shot shots/aa-editor/ui.png
//! ```
//!
//! * spawns: NPCs and doodads of the AAEmu server's spawn files ([`spawns`]); select, move, rotate, delete, place new
//!   ones from the browser; previews are the real models ([`preview`]);
//! * terrain: raise / lower / smooth / flatten brushes on the streamed terrain, saved as `heightmap.dat` into a pak
//!   overlay folder the client reads through `AA_PAK_OVERLAY` ([`terrain_edit`], [`heightmap_write`]);
//! * game data: NPC, doodad, item and quest rows of the server's `compact.sqlite3` ([`db`]);
//! * Play: walk the edited world as a character ([`play`]).
//!
//! Flags (plus the `aa-dev` ones `--shot --shot-after --exit-after --size`):
//!
//! * `--world <name>`        default `main_world`
//! * `--pos x,y[,z]`         start camera position (cry / server coordinates)
//! * `--spawn-dir <dir>`     the server's `Data/Worlds/<world>` folder the spawn files are read from and saved to
//!   (default: the bevy lane's source tree `D:/aaemu-wt/bevy/rust/crates/aaemu-game/app/Data/Worlds/<world>`)
//! * `--mirror <dir>`        another folder that receives a copy of saved spawn files (repeatable; default: the lane's
//!   runtime folder `D:/aaemu-live-bevy/rs-game/Data/Worlds/<world>` when it exists)
//! * `--db <file>`           the game database to edit (default: the lane's `D:/aaemu-live-bevy/rs-game/Data/compact.sqlite3`)
//! * `--read-only-db`        never write the database
//! * `--overlay <dir>`       pak overlay folder for terrain saves (default `data/local/editor/overlay`)
//! * `--select <unit_id>`    select the first spawn of this NPC / doodad template near the start (for screenshots)
//! * `--mode <select|place|terrain>`, `--play` start in that mode
//! * `--row <npcs|doodads|items|quests>:<id>` open that database row in the row editor
//! * checks: `--sculpt x,y` holds the terrain brush there for 120 frames (`--brush-radius`, `--brush-strength`),
//!   `--sculpt-save` then saves the terrain
//! * `--preview <npcs|doodads|items|quests>:<id>` open the model preview with that row
//! * `--export-pak <game_pak>` exports the pak overlay into that pak at startup ([`pak_export`]; never `D:/AA1.2`)
//!
//! Controls: hold the right mouse button to look, WASD / Space / Ctrl to fly (Shift fast), left click to select or
//! place, drag the selection to move it, `[` `]` or Shift + wheel to turn it, Delete, Ctrl+D duplicate, Ctrl+Z / Ctrl+Y,
//! Ctrl+S save, F focus, F5 play, Esc stop.

// Bevy systems take many parameters and nested query types.
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

mod camera;
mod db;
mod heightmap_write;
mod jsonc;
mod model_preview;
mod navmesh;
mod objects_extra;
mod surface_add;
mod pak_export;
mod npc_look;
mod play;
mod play_server;
mod preview;
mod server_terrain;
mod spawns;
mod terrain_edit;
mod terrain_paint;
mod cover_paint;
mod tools;
mod ui;
mod views;
mod world_objects;

use std::path::PathBuf;

use aa_world::{AaWorldPlugin, cry_to_bevy};
use bevy::prelude::*;
use bevy_egui::{EguiGlobalSettings, EguiPlugin, PrimaryEguiContext};

use crate::db::{GameData, Table};
use crate::spawns::{Kind, SpawnStore};

pub fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn args(name: &str) -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    args.windows(2).filter(|w| w[0] == name).map(|w| w[1].clone()).collect()
}

pub fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Select,
    Place,
    Terrain,
    /// Static world objects of the cell files (brushes, vegetation, entities).
    Objects,
}

/// Where things are saved.
#[derive(Clone, Debug)]
pub struct Paths {
    pub spawn_dir: PathBuf,
    pub mirrors: Vec<PathBuf>,
    pub db: PathBuf,
    pub overlay: PathBuf,
}

/// The editor's state shared by the tools and the panels.
#[derive(Resource)]
pub struct Editor {
    pub world: String,
    pub paths: Paths,
    pub mode: Mode,
    /// Index into [`SpawnStore::spawns`].
    pub selected: Option<usize>,
    /// The template the Place tool puts down.
    pub place: Option<(Kind, u32)>,
    pub browser: Table,
    pub search: String,
    pub view_radius: f32,
    pub show_npcs: bool,
    pub show_doodads: bool,
    pub snap_to_ground: bool,
    pub status: String,
    /// Open row editor.
    pub row: Option<ui::RowEdit>,
    pub playing: bool,
    /// Requested by the panels, done by the systems that own the data.
    pub requests: Vec<Request>,
}

#[derive(Clone, Debug)]
pub enum Request {
    SaveSpawns,
    SaveTerrain,
    Focus(Vec3),
    Play,
    Stop,
    UndoTerrain,
    /// Open / close the "Play on server" window.
    PlayOnServer,
    /// Save the edited static world objects ([`world_objects`]).
    SaveObjects,
}

impl Editor {
    pub fn status(&mut self, text: impl Into<String>) {
        let text = text.into();
        info!("{text}");
        self.status = text;
    }
}

/// The game DB's character tables for humanoid previews (aa-anim reads the shared copy read-only).
#[derive(Resource)]
pub struct AnimDb(pub Option<std::sync::Mutex<aa_anim::GameDb>>);

fn vec_arg(name: &str) -> Option<Vec<f32>> {
    let v: Vec<f32> = arg(name)?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    (v.len() >= 2).then_some(v)
}

fn main() {
    let world = arg("--world").unwrap_or_else(|| "main_world".into());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let overlay = arg("--overlay").map(PathBuf::from).unwrap_or_else(|| root.join("data/local/editor/overlay"));
    if let Err(e) = std::fs::create_dir_all(&overlay) {
        eprintln!("cannot create overlay folder {}: {e}", overlay.display());
    }
    // before anything opens the pak: the client data then includes our saved terrain
    // SAFETY: single threaded here, nothing reads the environment concurrently.
    unsafe { std::env::set_var("AA_PAK_OVERLAY", &overlay) };

    let spawn_dir = arg("--spawn-dir")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("D:/aaemu-wt/bevy/rust/crates/aaemu-game/app/Data/Worlds/{world}")));
    let mut mirrors: Vec<PathBuf> = args("--mirror").into_iter().map(PathBuf::from).collect();
    if mirrors.is_empty() && arg("--spawn-dir").is_none() {
        let runtime = PathBuf::from(format!("D:/aaemu-live-bevy/rs-game/Data/Worlds/{world}"));
        if runtime.is_dir() {
            mirrors.push(runtime);
        }
    }
    let db_path = arg("--db").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("D:/aaemu-live-bevy/rs-game/Data/compact.sqlite3"));

    let store = SpawnStore::load(&spawn_dir).unwrap_or_else(|e| {
        eprintln!("no spawns loaded from {}: {e}", spawn_dir.display());
        SpawnStore::empty(&spawn_dir)
    });
    let game = match GameData::open(&db_path, flag("--read-only-db")) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("cannot open the game database {}: {e}; using the shared copy read-only", db_path.display());
            GameData::open(std::path::Path::new(aa_anim::db::DEFAULT_DB), true).expect("game database")
        }
    };
    let anim_db = aa_anim::GameDb::open().map_err(|e| eprintln!("humanoid previews off: {e}")).ok().map(std::sync::Mutex::new);

    let start = match vec_arg("--pos") {
        Some(v) if v.len() >= 3 => Vec3::new(v[0], v[1], v[2]),
        Some(v) => {
            let ground = aa_world::TerrainQuery::new(&world).height_at(v[0], v[1]).unwrap_or(100.0);
            Vec3::new(v[0], v[1], ground + 25.0)
        }
        // the Nuia new-character spawn, from above
        None => Vec3::new(15560.0, 15350.0, 160.0),
    };
    let mode = match arg("--mode").as_deref() {
        Some("place") => Mode::Place,
        Some("terrain") => Mode::Terrain,
        Some("objects") => Mode::Objects,
        _ => Mode::Select,
    };
    let editor = Editor {
        world: world.clone(),
        paths: Paths { spawn_dir, mirrors, db: db_path, overlay },
        mode,
        selected: None,
        place: None,
        browser: Table::Npcs,
        search: String::new(),
        view_radius: 220.0,
        show_npcs: true,
        show_doodads: true,
        snap_to_ground: true,
        status: "ready".into(),
        row: None,
        playing: false,
        requests: if flag("--play") { vec![Request::Play] } else { Vec::new() },
    };

    let mut plugin = AaWorldPlugin::new(&world);
    plugin.config.load_radius = 1600.0;
    plugin.config.unload_radius = 2000.0;
    let mut app = App::new();
    app.add_plugins(aa_dev::default_plugins("AABevy world editor"))
        .add_plugins(aa_dev::DevPlugin)
        .insert_resource(EguiGlobalSettings { auto_create_primary_context: false, ..default() })
        .add_plugins(EguiPlugin::default())
        // (adds aa-anim's AnimPlugin too)
        .add_plugins(plugin)
        .insert_resource(editor)
        .insert_resource(store)
        .insert_resource(game)
        .insert_resource(AnimDb(anim_db))
        .add_plugins((camera::plugin, tools::plugin, views::plugin, terrain_edit::plugin, server_terrain::plugin, ui::plugin, play::plugin, play_server::plugin))
        .add_plugins(pak_export::plugin)
        .add_plugins(world_objects::plugin)
        .add_plugins(surface_add::plugin)
        .add_plugins(objects_extra::plugin)
        .add_plugins(model_preview::plugin)
        .add_systems(Startup, move |mut commands: Commands| {
            let transform = Transform::from_translation(cry_to_bevy(start))
                .with_rotation(Quat::from_rotation_y(0.0) * Quat::from_rotation_x(-0.35));
            commands.spawn((
                Camera3d::default(),
                Projection::Perspective(PerspectiveProjection { fov: 60f32.to_radians(), near: 0.3, far: 20000.0, ..default() }),
                transform,
                camera::EditorCamera,
                PrimaryEguiContext,
            ));
        });
    if let Some((table, id)) = arg("--row").and_then(|v| {
        let (t, id) = v.split_once(':')?;
        let table = Table::ALL.into_iter().find(|x| x.label().eq_ignore_ascii_case(t))?;
        Some((table, id.parse::<u32>().ok()?))
    }) {
        app.add_systems(Startup, move |mut editor: ResMut<Editor>, game: Res<GameData>| match ui::RowEdit::open(&game, table, id) {
            Ok(r) => editor.row = Some(r),
            Err(e) => editor.status(e),
        });
    }
    if let Some(id) = arg("--select").and_then(|v| v.parse::<u32>().ok()) {
        app.add_systems(Update, move |mut editor: ResMut<Editor>, store: Res<SpawnStore>, mut done: Local<bool>| {
            if *done {
                return;
            }
            *done = true;
            let pick = store.near(start, 2000.0).filter(|&i| store.spawns[i].now.unit_id == id).min_by(|&a, &b| {
                store.spawns[a].now.pos.distance(start).total_cmp(&store.spawns[b].now.pos.distance(start))
            });
            if let Some(i) = pick {
                editor.selected = Some(i);
                editor.requests.push(Request::Focus(store.spawns[i].now.pos));
            }
        });
    }
    app.run();
}
