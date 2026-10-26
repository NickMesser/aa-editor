//! UI editor for the ArcheAge 1.2 client's original Lua UI.
//!
//! The window shows a real screen of the client (login, world select, character select / create, the in-game HUD
//! and its windows, the loading screen), built from the original scripts by `aa-ui` with the fake game API of its
//! examples. The editor panels float over it:
//!
//! * **Widgets**: the widget tree; pick a widget on screen (F2 toggles pick mode, Alt+click picks any time),
//!   see its class, rectangle, anchors, extent, text, drawables, handlers and the script lines that created it and
//!   last set each property; change them live, drag to move and resize, and write the change into the script.
//! * **Scripts**: every script of the pak, as the decompiled Lua source, in a code editor with highlighting, find
//!   and go to line. Saving compiles it to the client's `.alb` layout into the pak overlay and rebuilds the screen.
//! * **Console**: script errors (click to open the line), `print` output, and a Lua prompt in the running UI.
//! * **Export**: write the overlay's UI files into a copy of the client's `game_pak`.
//!
//! ```text
//! cargo run -p aa-ui-editor -- --screen world --open toggle_bag
//! cargo run -p aa-ui-editor -- --screen character_create --race elf --gender female --step 2
//! ```

mod code;
mod console;
mod export;
#[allow(dead_code)]
mod fake;
mod highlight;
mod inspect;
mod loader;
mod patch;
mod screens;
mod track;

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy_egui::{EguiGlobalSettings, EguiPlugin, PrimaryEguiContext};

use crate::loader::Sources;
use crate::screens::{Screen, ScreenSpec};

pub fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

pub fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

/// The editor's state shared by the panels.
#[derive(Resource)]
pub struct Editor {
    pub spec: ScreenSpec,
    pub sources: Sources,
    pub unluac: Option<PathBuf>,
    /// Rebuild the screen at the next update.
    pub reload: bool,
    /// F1: hide every editor panel (the screen as the client shows it).
    pub hidden: bool,
    /// F2: the pointer picks widgets instead of clicking them.
    pub pick: bool,
    /// The pointer or the keyboard belongs to egui this frame.
    pub egui_pointer: bool,
    pub egui_keyboard: bool,
    pub status: String,
    /// Open a script at a line (set by any panel, handled by the script editor).
    pub open_script: Option<(String, Option<usize>)>,
    pub windows: Windows,
}

/// Which editor windows are open.
#[derive(Clone)]
pub struct Windows {
    pub tree: bool,
    pub inspector: bool,
    pub scripts: bool,
    pub editor: bool,
    pub console: bool,
    pub export: bool,
}

impl Editor {
    pub fn status(&mut self, s: impl Into<String>) {
        let s = s.into();
        info!("ui-editor: {s}");
        self.status = s;
    }
}

/// The repository root (`data/local/...` defaults live there).
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn first_existing(cands: impl IntoIterator<Item = PathBuf>, ok: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    cands.into_iter().find(|p| ok(p))
}

fn main() {
    let root = root();
    let overlay = arg("--overlay").map(PathBuf::from).unwrap_or_else(|| root.join("data/local/editor/overlay"));
    if let Err(e) = std::fs::create_dir_all(&overlay) {
        eprintln!("cannot create overlay folder {}: {e}", overlay.display());
    }
    // before anything opens the pak: scripts saved by the editor replace the pak's
    // SAFETY: single threaded here, nothing reads the environment concurrently.
    unsafe { std::env::set_var("AA_PAK_OVERLAY", &overlay) };
    let workspace = arg("--workspace").map(PathBuf::from).unwrap_or_else(|| root.join("data/local/ui-editor/sources"));
    let decompiled = first_existing(
        arg("--decompiled").map(PathBuf::from).into_iter().chain([
            root.join("data/local/ui-editor/decompiled"),
            PathBuf::from("D:/AABevy1.2/data/extract/aa-lua/decompiled"),
        ]),
        loader::is_decompiled_dir,
    );
    let unluac = first_existing(
        arg("--unluac").map(PathBuf::from).into_iter().chain([
            root.join("data/local/ui-editor/unluac.jar"),
            PathBuf::from("D:/AABevy1.2/data/extract/aa-lua/tools/unluac.jar"),
        ]),
        |p| p.is_file(),
    );
    let source_mode = decompiled.is_some() && !flag("--alb");
    let sources = Sources { decompiled, workspace, overlay, source_mode };

    let mut spec = ScreenSpec { screen: arg("--screen").and_then(|s| Screen::parse(&s)).unwrap_or(Screen::World), ..default() };
    if let Some(r) = arg("--race") {
        spec.race = r;
    }
    if let Some(g) = arg("--gender") {
        spec.gender = g;
    }
    if let Some(s) = arg("--step").and_then(|s| s.parse().ok()) {
        spec.step = s;
    }
    spec.with_character = arg("--characters").as_deref() != Some("0");
    let db = arg("--db").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(fake::GAME_DB));
    // egui's pipeline draws into the linear (sRGB format) main texture, so the UI blends linearly too: translucent
    // panels come out slightly lighter than in the client (as in aa-ui's split mode)
    let config = aa_ui::UiConfig { ui_texts_db: db.is_file().then_some(db), gamma_blending: false, ..default() };

    let editor = Editor {
        spec: spec.clone(),
        sources: sources.clone(),
        unluac,
        reload: false,
        hidden: false,
        pick: flag("--pick"),
        egui_pointer: false,
        egui_keyboard: false,
        status: String::new(),
        open_script: arg("--script").map(|s| (s, arg("--line").and_then(|l| l.parse().ok()))),
        windows: Windows { tree: true, inspector: true, scripts: true, editor: true, console: true, export: false },
    };
    info!(
        "ui-editor: overlay {}, workspace {}, decompiled {:?}, source mode {}",
        sources.overlay.display(),
        sources.workspace.display(),
        sources.decompiled,
        sources.source_mode
    );

    App::new()
        .add_plugins(aa_dev::default_plugins("AABevy UI editor"))
        .add_plugins(aa_dev::DevPlugin)
        .insert_resource(EguiGlobalSettings { auto_create_primary_context: false, ..default() })
        .add_plugins(EguiPlugin::default())
        .add_plugins(aa_ui::UiPlugin { config: Some(config) })
        .insert_resource(screens::setup(spec, sources))
        .insert_resource(editor)
        .configure_sets(Update, aa_ui::UiInput.run_if(|e: Res<Editor>| !e.egui_keyboard))
        .add_systems(Startup, spawn_egui_camera)
        .add_systems(Update, (keys, reload).chain().before(aa_ui::UiInput))
        .add_systems(Update, (pointer, game_hotkeys).before(aa_ui::UiFrame))
        .add_plugins((inspect::plugin, code::plugin, console::plugin, export::plugin))
        .run();
}

/// egui draws with its own camera above the UI's (order 10).
fn spawn_egui_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera { order: 100, clear_color: ClearColorConfig::None, ..default() },
        bevy::camera::visibility::RenderLayers::layer(29),
        PrimaryEguiContext,
    ));
}

/// F1 hide, F2 pick, F5 rebuild (when no text field has the keys).
fn keys(keys: Res<ButtonInput<KeyCode>>, mut editor: ResMut<Editor>) {
    if editor.egui_keyboard {
        return;
    }
    if keys.just_pressed(KeyCode::F1) {
        editor.hidden = !editor.hidden;
    }
    if keys.just_pressed(KeyCode::F2) {
        editor.pick = !editor.pick;
    }
    if keys.just_pressed(KeyCode::F5) {
        editor.reload = true;
    }
}

/// Rebuild the screen (a fresh Lua state, like the client's stage change).
fn reload(world: &mut World) {
    let Some(mut editor) = world.get_resource_mut::<Editor>() else { return };
    if !std::mem::take(&mut editor.reload) {
        return;
    }
    let (spec, sources) = (editor.spec.clone(), editor.sources.clone());
    editor.status(format!("rebuilding {}", spec.screen.label()));
    world.resource_mut::<inspect::Inspect>().clear();
    aa_ui::reload_ui(world, screens::setup(spec, sources));
}

/// The screen gets the pointer only when it is not over an editor panel and not picking.
fn pointer(editor: Res<Editor>, keys: Res<ButtonInput<KeyCode>>, mut virt: ResMut<aa_ui::UiVirtualPointer>) {
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let blocked = !editor.hidden && (editor.egui_pointer || editor.pick || alt);
    let want = blocked.then_some(Vec2::new(-10000.0, -10000.0));
    if virt.pos != want {
        virt.pos = want;
    }
}

/// The client's default window keys in game (I / B bag, C character, K skills, L quests, M map, Escape).
fn game_hotkeys(
    editor: Res<Editor>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    keys: Res<ButtonInput<KeyCode>>,
    had_focus: Local<bool>,
) {
    if editor.egui_keyboard || editor.spec.screen != Screen::World {
        return;
    }
    fake::hotkeys(rt, keys, had_focus);
}
