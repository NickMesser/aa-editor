//! The "Model preview" window: a turntable render of the browser row under the mouse, the open row editor's row or
//! the selected spawn.
//!
//! * NPCs: the model the world shows (a humanoid dressed in its own gear with its weapons, a creature with its idle
//!   animation, a static model or prefab);
//! * doodads: `doodad_almighties.model` (a `.cgf` or a `prefab://` element);
//! * items: weapons (`item_weapons` -> `item_assets.path`), armour worn by a default body of the race / gender picked
//!   in the window (the first one with meshes for the item when the picked one has none); every item shows its icon,
//!   and items without a model only the icon;
//! * quests: the NPC who gives the quest (`QuestActConAcceptNpc`), else the one it is turned in to, else the first step
//!   that names an NPC.
//!
//! The models are spawned at the origin on their own render layer ([`LAYER`]), which only the preview camera and its
//! two lights see; the camera draws into an image that the window shows, and only while the window is open.

use aa_anim::{ClipLibrary, Gender, Race};
use aa_cry::scene::CryAssets;
use aa_cry::texture::{Dds, rgba8_image};
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, EguiTextureHandle, EguiUserTextures, egui};

use crate::db::{GameData, Table};
use crate::preview::Previewer;
use crate::spawns::{Kind, SpawnStore};
use crate::{AnimDb, Editor};

/// The render layer of the preview models, camera and lights.
pub const LAYER: usize = 7;
/// Side of the square render target, in pixels.
const SIZE: u32 = 640;
/// How long the pointer has to rest on a browser row before the preview switches to it (seconds).
const HOVER_DELAY: f64 = 0.3;
/// Frames the model's bounds are measured after it spawned (parts and skins appear over a few frames).
const FIT_FRAMES: u32 = 90;

/// What the preview shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    Npc(u32),
    Doodad(u32),
    Item(u32),
    Quest(u32),
    /// A static world object's model (Objects mode selection): index into [`ModelPreview::models`].
    Model(u32),
}

impl Subject {
    pub fn of_row(table: Table, id: u32) -> Subject {
        match table {
            Table::Npcs => Subject::Npc(id),
            Table::Doodads => Subject::Doodad(id),
            Table::Items => Subject::Item(id),
            Table::Quests => Subject::Quest(id),
        }
    }

    fn row(self) -> (Table, u32) {
        match self {
            Subject::Npc(id) => (Table::Npcs, id),
            Subject::Doodad(id) => (Table::Doodads, id),
            Subject::Item(id) => (Table::Items, id),
            Subject::Quest(id) => (Table::Quests, id),
            Subject::Model(_) => (Table::Doodads, 0),
        }
    }

    /// `npcs:502`, `items:18395`, `doodads:300`, `quests:19`.
    pub fn parse(text: &str) -> Option<Subject> {
        let (t, id) = text.split_once(':')?;
        let table = Table::ALL.into_iter().find(|x| x.label().eq_ignore_ascii_case(t))?;
        Some(Subject::of_row(table, id.trim().parse().ok()?))
    }
}

/// Turntable camera around the model's centre.
#[derive(Clone, Copy, Debug)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    center: Vec3,
    /// Distance that frames the whole model (`reset` returns to it).
    fit_distance: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Orbit { yaw: 0.6, pitch: -0.18, distance: 4.0, center: Vec3::new(0.0, 1.0, 0.0), fit_distance: 4.0 }
    }
}

#[derive(Resource)]
pub struct ModelPreview {
    pub open: bool,
    /// Follow the browser / row editor / selection (off: keep the current subject).
    pub follow: bool,
    /// Set by the browser each frame: the row under the pointer.
    pub hover: Option<Subject>,
    /// Asked for explicitly (row editor button, `--preview`); taken by the next update.
    pub request: Option<Subject>,
    hover_since: Option<(Subject, f64)>,
    last_row: Option<(Table, u32)>,
    last_selected: Option<(Kind, u32)>,
    /// What should be shown.
    subject: Option<Subject>,
    /// What is spawned (subject, race, gender).
    shown: Option<(Subject, Race, Gender)>,
    root: Option<Entity>,
    image: Handle<Image>,
    texture: Option<egui::TextureId>,
    icon: Option<(u32, Option<egui::TextureId>)>,
    /// Item icon to load (item, pak path).
    icon_pending: Option<(u32, String)>,
    /// Turn the model onto its longest axis at the first fit (weapons).
    upright: bool,
    /// The subject has a model (else only its note and icon show).
    has_model: bool,
    note: String,
    orbit: Orbit,
    fit: u32,
    auto_rotate: bool,
    race: Race,
    gender: Gender,
    /// Model files of [`Subject::Model`].
    models: Vec<String>,
    last_object: Option<String>,
}

impl ModelPreview {
    /// The subject showing a model file.
    pub fn model(&mut self, path: &str) -> Subject {
        let i = self.models.iter().position(|m| m == path).unwrap_or_else(|| {
            self.models.push(path.to_string());
            self.models.len() - 1
        });
        Subject::Model(i as u32)
    }
}

#[derive(Component)]
struct PreviewCamera;

#[derive(Component)]
struct PreviewLight(Vec3);

#[derive(Component)]
struct PreviewModel;

pub fn plugin(app: &mut App) {
    let subject = crate::arg("--preview").and_then(|s| Subject::parse(&s));
    app.insert_resource(ModelPreview {
        // `--preview-open`: the window open at start, following the selection
        open: subject.is_some() || crate::flag("--preview-open"),
        follow: true,
        hover: None,
        request: subject,
        hover_since: None,
        last_row: None,
        last_selected: None,
        subject: None,
        shown: None,
        root: None,
        image: Handle::default(),
        texture: None,
        icon: None,
        icon_pending: None,
        upright: false,
        has_model: false,
        note: String::new(),
        orbit: Orbit::default(),
        fit: 0,
        auto_rotate: true,
        race: Race::Nuian,
        gender: Gender::Female,
        models: Vec::new(),
        last_object: None,
    })
    .add_systems(Startup, setup)
    .add_systems(Update, (choose, spawn, load_icon, layers, fit, place_camera).chain())
    .add_systems(EguiPrimaryContextPass, window.after(crate::ui::panels));
}

fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut textures: ResMut<EguiUserTextures>, mut preview: ResMut<ModelPreview>) {
    let mut image = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = RenderAssetUsages::default();
    let image = images.add(image);
    preview.texture = Some(textures.add_image(EguiTextureHandle::Strong(image.clone())));
    preview.image = image.clone();
    commands.spawn((
        Name::new("model preview camera"),
        Camera3d::default(),
        Camera { order: -5, is_active: false, clear_color: ClearColorConfig::Custom(Color::srgb(0.13, 0.14, 0.16)), ..default() },
        RenderTarget::Image(image.into()),
        Projection::Perspective(PerspectiveProjection { fov: 30f32.to_radians(), near: 0.05, far: 2000.0, ..default() }),
        AmbientLight { color: Color::WHITE, brightness: 450.0, ..default() },
        RenderLayers::layer(LAYER),
        Transform::default(),
        PreviewCamera,
    ));
    // key light from the upper front left, fill from the back right (both follow the camera)
    for (dir, lux) in [(Vec3::new(-0.6, -0.8, -0.5), 9000.0), (Vec3::new(0.7, -0.2, 0.6), 2500.0)] {
        commands.spawn((
            Name::new("model preview light"),
            DirectionalLight { illuminance: lux, shadow_maps_enabled: false, contact_shadows_enabled: false, ..default() },
            RenderLayers::layer(LAYER),
            Transform::default(),
            PreviewLight(dir),
        ));
    }
}

/// Picks the subject: an explicit request, else a row the pointer rests on, a newly opened row editor row or a newly
/// selected spawn.
fn choose(
    mut preview: ResMut<ModelPreview>,
    editor: Res<Editor>,
    store: Res<SpawnStore>,
    objs: Res<crate::world_objects::WorldObjects>,
    state: Res<aa_world::objects::ObjectState>,
    time: Res<Time>,
) {
    // the model of the selected world object (Objects mode)
    let object = objs.selected.and_then(|part| {
        let cell = objs.cells.get(&part.cell)?;
        let m = cell.model(part.kind, part.index, state.vegetation_groups().map(|g| &**g));
        let m = m.split(" (").next().unwrap_or(&m).trim().to_string();
        (!m.is_empty() && !m.starts_with("vegetation group")).then_some(m)
    });
    let new_object = object.clone().filter(|o| preview.last_object.as_ref() != Some(o));
    preview.last_object = object;
    if preview.follow
        && let Some(m) = new_object
    {
        let s = preview.model(&m);
        preview.subject = Some(s);
        return;
    }
    let p = &mut *preview;
    if let Some(s) = p.request.take() {
        p.subject = Some(s);
        p.open = true;
        return;
    }
    let now = time.elapsed_secs_f64();
    let hover = p.hover.take();
    p.hover_since = match (hover, p.hover_since) {
        (Some(h), Some((s, t))) if h == s => Some((s, t)),
        (Some(h), _) => Some((h, now)),
        (None, _) => None,
    };
    let row = editor.row.as_ref().map(|r| (r.table, r.id));
    let selected = editor.selected.and_then(|i| store.spawns.get(i)).filter(|s| !s.now.deleted).map(|s| (s.kind, s.now.unit_id));
    let new_row = row.filter(|r| p.last_row != Some(*r));
    let new_selected = selected.filter(|s| p.last_selected != Some(*s));
    p.last_row = row;
    p.last_selected = selected;
    if !p.follow {
        return;
    }
    if let Some((h, t)) = p.hover_since
        && now - t >= HOVER_DELAY
    {
        p.subject = Some(h);
    } else if let Some((table, id)) = new_row {
        p.subject = Some(Subject::of_row(table, id));
    } else if let Some((kind, id)) = new_selected {
        p.subject = Some(match kind {
            Kind::Npc => Subject::Npc(id),
            Kind::Doodad => Subject::Doodad(id),
        });
    }
}

/// (Re)spawns the model when the subject (or the armour body) changed and the window is open.
fn spawn(
    mut commands: Commands,
    mut preview: ResMut<ModelPreview>,
    mut game: ResMut<GameData>,
    anim_db: Res<AnimDb>,
    clips: Res<ClipLibrary>,
    mut cry: CryAssets,
) {
    let p = &mut *preview;
    if !p.open {
        return;
    }
    let Some(subject) = p.subject else { return };
    let key = (subject, p.race, p.gender);
    if p.shown == Some(key) {
        return;
    }
    // the body only matters for armour
    if let Some((s, ..)) = p.shown
        && s == subject
        && !matches!(subject, Subject::Item(id) if game.item_is_armor(id))
    {
        p.shown = Some(key);
        return;
    }
    p.shown = Some(key);
    if let Some(root) = p.root.take() {
        commands.entity(root).despawn();
    }
    let anim_db = anim_db.0.as_ref().map(|m| m.lock().unwrap_or_else(|e| e.into_inner()));
    let mut previewer = Previewer { cry: &mut cry, clips: &clips, game: anim_db.as_deref() };
    let (table, id) = subject.row();
    let name = game.name(table, id).unwrap_or_default();
    let kind = match subject {
        Subject::Npc(_) => "NPC",
        Subject::Doodad(_) => "Doodad",
        Subject::Item(_) => "Item",
        Subject::Quest(_) => "Quest",
        Subject::Model(_) => "World object",
    };
    let mut note = match subject {
        Subject::Model(i) => format!("{kind}\n{}", p.models.get(i as usize).cloned().unwrap_or_default()),
        _ => format!("{kind} {id} {name}"),
    };
    let mut scale = 1.0;
    let model = match subject {
        Subject::Npc(id) => spawn_npc(&mut commands, &mut previewer, &mut game, id, &mut note, &mut scale),
        Subject::Model(i) => match p.models.get(i as usize).cloned() {
            Some(path) => previewer.spawn_npc(&mut commands, &path, None),
            None => None,
        },
        Subject::Doodad(id) => match game.doodad_model(id) {
            Some(reference) => {
                note.push_str(&format!("\n{reference}"));
                previewer.spawn_npc(&mut commands, &reference, None)
            }
            None => {
                note.push_str("\nno model");
                None
            }
        },
        Subject::Quest(id) => match game.quest_npc(id) {
            Some((npc, role)) => {
                note.push_str(&format!("\nNPC {npc} {} {role}", game.name(Table::Npcs, npc).unwrap_or_default()));
                spawn_npc(&mut commands, &mut previewer, &mut game, npc, &mut note, &mut scale)
            }
            None => {
                note.push_str("\nno NPC gives or takes this quest");
                None
            }
        },
        Subject::Item(id) => {
            if p.icon.as_ref().is_none_or(|(i, _)| *i != id) {
                p.icon = None;
                p.icon_pending = game.item_icon(id).map(|path| (id, path));
            }
            if let Some(path) = game.item_weapon(id) {
                note.push_str(&format!("\n{path}"));
                p.upright = true;
                previewer.spawn_weapon(&mut commands, &path)
            } else if game.item_is_armor(id) {
                let bodies = std::iter::once((p.race, p.gender)).chain(Race::ALL.into_iter().flat_map(|r| Gender::ALL.map(|g| (r, g))));
                match bodies.into_iter().find(|&(r, g)| previewer.wears(r, g, id)) {
                    Some((r, g)) => {
                        if (r, g) != (p.race, p.gender) {
                            note.push_str(&format!("\nno meshes for {:?} {:?}: worn by {r:?} {g:?}", p.race, p.gender));
                        } else {
                            note.push_str(&format!("\nworn by {r:?} {g:?}"));
                        }
                        previewer.spawn_dressed(&mut commands, r, g, vec![id])
                    }
                    None => {
                        note.push_str("\narmour without meshes in the pak");
                        None
                    }
                }
            } else {
                note.push_str("\nno model (icon only)");
                None
            }
        }
    };
    if !matches!(subject, Subject::Item(_)) {
        p.icon = None;
    }
    if model.is_none() && !note.contains("no model") && !note.contains("without meshes") && !note.contains("no NPC gives") {
        note.push_str("\nthe model could not be loaded");
    }
    p.note = note;
    let root = commands.spawn((Name::new("model preview"), Transform::from_scale(Vec3::splat(scale)), Visibility::default(), PreviewModel, RenderLayers::layer(LAYER))).id();
    if let Some(model) = model {
        commands.entity(model).insert(ChildOf(root));
    }
    p.root = Some(root);
    p.has_model = model.is_some();
    p.fit = FIT_FRAMES;
    p.upright &= matches!(subject, Subject::Item(_));
    p.orbit = Orbit::default();
}

fn spawn_npc(commands: &mut Commands, previewer: &mut Previewer, game: &mut GameData, id: u32, note: &mut String, scale: &mut f32) -> Option<Entity> {
    let Some(reference) = game.npc_model(id) else {
        note.push_str("\nno model");
        return None;
    };
    note.push_str(&format!("\n{reference}"));
    *scale = game.npc_scale(id);
    let look = game.npc_look(id);
    let model = previewer.spawn_npc(commands, &reference, look.as_ref())?;
    if let Some(look) = look.as_ref() {
        Previewer::carry(commands, model, game.npc_carried(look));
    }
    Some(model)
}

fn load_icon(mut preview: ResMut<ModelPreview>, mut images: ResMut<Assets<Image>>, mut textures: ResMut<EguiUserTextures>) {
    let Some((id, path)) = preview.icon_pending.take() else { return };
    let icon = icon_image(&path).map(|image| textures.add_image(EguiTextureHandle::Strong(images.add(image))));
    preview.icon = Some((id, icon));
}

/// An item icon (`ui/icon/*.dds`) as an image.
fn icon_image(path: &str) -> Option<Image> {
    let bytes = aa_pak::Pak::global().read(path).ok()?;
    let dds = Dds::parse(&bytes).ok()?;
    Some(rgba8_image(dds.width, dds.height, &[dds.decode_rgba8(0, 0)], true))
}

/// Puts everything under the preview root on the preview layer (models spawn their parts over several frames).
fn layers(mut commands: Commands, preview: Res<ModelPreview>, children: Query<&Children>, layered: Query<(), With<RenderLayers>>) {
    let Some(root) = preview.root.filter(|_| preview.open) else { return };
    for e in children.iter_descendants(root) {
        if !layered.contains(e) {
            commands.entity(e).insert(RenderLayers::layer(LAYER));
        }
    }
}

/// Frames the model: its world-space bounds (the root sits at the origin) while its parts appear.
fn fit(
    mut preview: ResMut<ModelPreview>,
    children: Query<&Children>,
    bounds: Query<(Option<&Aabb>, &GlobalTransform)>,
    mut roots: Query<&mut Transform, With<PreviewModel>>,
) {
    let p = &mut *preview;
    let Some(root) = p.root.filter(|_| p.fit > 0) else { return };
    p.fit -= 1;
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let mut meshes = 0;
    for d in children.iter_descendants(root) {
        let Ok((aabb, t)) = bounds.get(d) else { continue };
        // skinned bodies carry no bounds: their bones stand for them
        let Some(aabb) = aabb else {
            let w = t.translation();
            if w != Vec3::ZERO {
                lo = lo.min(w);
                hi = hi.max(w);
            }
            continue;
        };
        meshes += 1;
        let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        for i in 0..8 {
            let corner = Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 });
            let w = t.transform_point(c + h * corner);
            lo = lo.min(w);
            hi = hi.max(w);
        }
    }
    if lo.x > hi.x {
        return;
    }
    // a weapon stands on its longest axis (they lie along their grip in the files), decided a few frames in: the
    // parts' own transforms propagate a frame or two after they appear
    if p.upright && meshes > 0 && p.fit < FIT_FRAMES - 10 {
        p.upright = false;
        let size = hi - lo;
        let turn = if size.z >= size.x && size.z > size.y {
            Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
        } else if size.x > size.y {
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };
        if turn != Quat::IDENTITY
            && let Ok(mut t) = roots.get_mut(root)
        {
            t.rotation = turn;
            // measure again once the turn has propagated
            p.fit = FIT_FRAMES;
            return;
        }
    }
    let radius = ((hi - lo).length() * 0.5).max(0.05);
    let distance = radius / (15f32.to_radians()).sin() * 1.15;
    let user_zoomed = (p.orbit.distance - p.orbit.fit_distance).abs() > 1e-3;
    p.orbit.center = (lo + hi) * 0.5;
    p.orbit.fit_distance = distance;
    if !user_zoomed {
        p.orbit.distance = distance;
    }
}

fn place_camera(
    mut preview: ResMut<ModelPreview>,
    time: Res<Time>,
    mut cam: Query<(&mut Camera, &mut Transform), With<PreviewCamera>>,
    mut lights: Query<(&PreviewLight, &mut Transform), Without<PreviewCamera>>,
) {
    let active = preview.open && preview.has_model;
    let Ok((mut camera, mut t)) = cam.single_mut() else { return };
    if camera.is_active != active {
        camera.is_active = active;
    }
    if !active {
        return;
    }
    if preview.auto_rotate {
        preview.orbit.yaw += time.delta_secs() * 0.5;
    }
    let o = preview.orbit;
    let rotation = Quat::from_euler(EulerRot::YXZ, o.yaw, o.pitch, 0.0);
    *t = Transform::from_translation(o.center + rotation * Vec3::new(0.0, 0.0, o.distance)).with_rotation(rotation);
    for (light, mut lt) in &mut lights {
        let dir = rotation * light.0;
        *lt = Transform::IDENTITY.looking_to(dir, Vec3::Y);
    }
}

fn window(mut contexts: EguiContexts, mut preview: ResMut<ModelPreview>, game: Res<GameData>) -> Result {
    if !preview.open {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let p = &mut *preview;
    let mut open = true;
    egui::Window::new("Model preview").open(&mut open).default_pos([820.0, 80.0]).default_width(380.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut p.follow, "follow").on_hover_text("Show the browser row under the pointer, the open row and the selected spawn");
            ui.checkbox(&mut p.auto_rotate, "turn");
            if ui.button("Reset view").clicked() {
                let fit = (p.orbit.center, p.orbit.fit_distance);
                p.orbit = Orbit { center: fit.0, distance: fit.1, fit_distance: fit.1, ..Orbit::default() };
            }
        });
        if let Some(Subject::Item(id)) = p.subject
            && game.item_is_armor(id)
        {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("race").selected_text(format!("{:?}", p.race)).show_ui(ui, |ui| {
                    for r in Race::ALL {
                        ui.selectable_value(&mut p.race, r, format!("{r:?}"));
                    }
                });
                for g in Gender::ALL {
                    ui.selectable_value(&mut p.gender, g, format!("{g:?}"));
                }
            });
        }
        ui.horizontal_top(|ui| {
            if let Some((_, Some(icon))) = p.icon {
                ui.add(egui::Image::new(egui::load::SizedTexture::new(icon, [40.0, 40.0])));
            }
            let text = if p.subject.is_none() { "Point at a browser row, open a row or select a spawn." } else { p.note.as_str() };
            ui.add(egui::Label::new(text).wrap());
        });
        let side = ui.available_width().clamp(160.0, 900.0);
        if let Some(texture) = p.texture
            && p.has_model
        {
            let response = ui.add(egui::Image::new(egui::load::SizedTexture::new(texture, [side, side])).sense(egui::Sense::click_and_drag()));
            if response.dragged() {
                let d = response.drag_delta();
                p.orbit.yaw -= d.x * 0.01;
                p.orbit.pitch = (p.orbit.pitch - d.y * 0.01).clamp(-1.5, 1.5);
                p.auto_rotate = false;
            }
            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    p.orbit.distance = (p.orbit.distance * (1.0 - scroll * 0.002)).clamp(0.1, 2000.0);
                }
            }
            ui.small("Drag to turn, wheel to zoom.");
        }
    });
    if !open {
        p.open = false;
    }
    Ok(())
}
