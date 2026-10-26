//! NPC and doodad spawns: the AAEmu server's `Data/Worlds/<world>/{npc,doodad}_spawns*.json` files.
//!
//! Entry format (case-insensitive keys, `//` comments allowed, angles in degrees, Cry space: Z up):
//! `{"UnitId": 502, "Position": {"X": 23039.76, "Y": 11540.64, "Z": 252.7, "Yaw": -36.5625}}`.
//! `UnitId` is `npcs.id` / `doodad_almighties.id`. An NPC entry may name the AI route it walks: `"FollowPath": "<name>"`
//! (`Data/Path/<name>.path`, see [`crate::paths`]). The server reads every `npc_spawns*.json` /
//! `doodad_spawns*.json` of the folder, so new spawns go to `*_spawns_editor.json` and the retail
//! files are only rewritten when one of their entries was moved or deleted (untouched entries keep
//! their exact text, comments included).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde_json::{Map, Value};

use crate::jsonc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Npc,
    Doodad,
}

impl Kind {
    pub fn prefix(self) -> &'static str {
        match self {
            Kind::Npc => "npc_spawns",
            Kind::Doodad => "doodad_spawns",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Npc => "NPC",
            Kind::Doodad => "Doodad",
        }
    }
}

/// The editable part of a spawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub unit_id: u32,
    /// Cry space metres.
    pub pos: Vec3,
    /// Degrees, as in the file.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
    pub deleted: bool,
    /// `FollowPath`: index into [`SpawnStore::path_names`], 0 = none.
    pub path: u16,
}

impl Placement {
    pub fn new(unit_id: u32, pos: Vec3, yaw: f32) -> Self {
        Placement { unit_id, pos, yaw, pitch: 0.0, roll: 0.0, scale: 1.0, deleted: false, path: 0 }
    }

    /// Bevy-space rotation of a model placed with this spawn (yaw about Cry Z, counter-clockwise from north).
    pub fn rotation(&self) -> Quat {
        let cry = Quat::from_euler(EulerRot::ZYX, self.yaw.to_radians(), self.pitch.to_radians(), self.roll.to_radians());
        aa_world::coords::cry_rot_to_bevy(cry)
    }
}

pub struct Spawn {
    pub kind: Kind,
    pub file: usize,
    pub now: Placement,
    /// The entry as loaded (all fields), `None` for a new one.
    value: Option<Value>,
    /// Exact source text of an untouched loaded entry.
    source: Option<String>,
    original: Option<Placement>,
}

impl Spawn {
    pub fn changed(&self) -> bool {
        self.original != Some(self.now) && !(self.original.is_none() && self.now.deleted)
    }
}

pub struct SpawnFile {
    pub path: PathBuf,
    pub existed: bool,
}

/// One undoable step.
struct Step {
    index: usize,
    before: Placement,
    after: Placement,
}

#[derive(Resource, Default)]
pub struct SpawnStore {
    /// The folder new spawns are saved in (`*_spawns_editor.json`).
    dir: PathBuf,
    pub files: Vec<SpawnFile>,
    pub spawns: Vec<Spawn>,
    undo: Vec<Vec<Step>>,
    redo: Vec<Vec<Step>>,
    /// Incremented on every change, so views know to refresh.
    pub generation: u64,
    /// `FollowPath` names; index 0 is "no path".
    pub path_names: Vec<String>,
}

fn get_ci<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    obj.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
}

fn set_ci(obj: &mut Map<String, Value>, key: &str, value: Value) {
    let existing = obj.keys().find(|k| k.eq_ignore_ascii_case(key)).cloned();
    obj.insert(existing.unwrap_or_else(|| key.to_string()), value);
}

fn num(obj: &Map<String, Value>, key: &str) -> Option<f64> {
    get_ci(obj, key).and_then(Value::as_f64)
}

fn round(v: f32, digits: i32) -> Value {
    let p = 10f64.powi(digits);
    serde_json::Number::from_f64((v as f64 * p).round() / p).map(Value::Number).unwrap_or(Value::Null)
}

fn placement_of(v: &Value) -> Option<Placement> {
    let obj = v.as_object()?;
    let unit_id = num(obj, "UnitId")? as u32;
    let p = get_ci(obj, "Position")?.as_object()?;
    let f = |k: &str| num(p, k).unwrap_or(0.0) as f32;
    Some(Placement {
        unit_id,
        pos: Vec3::new(f("X"), f("Y"), f("Z")),
        yaw: f("Yaw"),
        pitch: f("Pitch"),
        roll: f("Roll"),
        scale: num(obj, "Scale").map(|s| s as f32).filter(|s| *s > 0.0).unwrap_or(1.0),
        deleted: false,
        path: 0,
    })
}

impl SpawnStore {
    /// Loads every spawn file of `dir` (`.../Data/Worlds/<world>`).
    pub fn load(dir: &Path) -> anyhow::Result<SpawnStore> {
        let mut store = SpawnStore::empty(dir);
        let mut names: Vec<(Kind, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_ascii_lowercase) else { continue };
            if !name.ends_with(".json") {
                continue;
            }
            for kind in [Kind::Npc, Kind::Doodad] {
                if name.starts_with(kind.prefix()) {
                    names.push((kind, path.clone()));
                }
            }
        }
        names.sort_by(|a, b| a.1.cmp(&b.1));
        for (kind, path) in names {
            let text = std::fs::read_to_string(&path)?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            let file = store.files.len();
            store.files.push(SpawnFile { path: path.clone(), existed: true });
            let spans = jsonc::array_elements(text).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            let mut bad = 0;
            for (s, e) in spans {
                let source = &text[s..e];
                let value: Option<Value> = serde_json::from_str(&jsonc::strip_comments(source)).ok();
                match value.as_ref().and_then(placement_of) {
                    Some(mut p) => {
                        let follow = value.as_ref().and_then(Value::as_object).and_then(|o| get_ci(o, "FollowPath")).and_then(Value::as_str);
                        p.path = follow.map_or(0, |n| store.path_id(n));
                        store.spawns.push(Spawn { kind, file, now: p, value, source: Some(source.to_string()), original: Some(p) });
                    }
                    None => bad += 1,
                }
            }
            if bad > 0 {
                warn!("{}: {bad} entries without UnitId / Position are kept out of the editor", path.display());
            }
        }
        info!("spawns: {} from {} files in {}", store.spawns.len(), store.files.iter().filter(|f| f.existed).count(), dir.display());
        Ok(store)
    }

    /// A store without spawns that saves new ones into `dir`.
    pub fn empty(dir: &Path) -> SpawnStore {
        SpawnStore { dir: dir.to_path_buf(), path_names: vec![String::new()], ..default() }
    }

    /// The index of a `FollowPath` name ("" is 0, no path).
    pub fn path_id(&mut self, name: &str) -> u16 {
        let name = name.trim();
        if let Some(i) = self.path_names.iter().position(|n| n == name) {
            return i as u16;
        }
        self.path_names.push(name.to_string());
        (self.path_names.len() - 1) as u16
    }

    /// The `FollowPath` of a placement.
    pub fn path_of(&self, p: &Placement) -> Option<&str> {
        self.path_names.get(p.path as usize).map(String::as_str).filter(|n| !n.is_empty())
    }

    fn editor_file(&mut self, kind: Kind) -> usize {
        let path = self.dir.join(format!("{}_editor.json", kind.prefix()));
        match self.files.iter().position(|f| f.path == path) {
            Some(i) => i,
            None => {
                self.files.push(SpawnFile { path, existed: false });
                self.files.len() - 1
            }
        }
    }

    pub fn add(&mut self, kind: Kind, placement: Placement) -> usize {
        let file = self.editor_file(kind);
        let index = self.spawns.len();
        self.spawns.push(Spawn { kind, file, now: Placement { deleted: true, ..placement }, value: None, source: None, original: None });
        self.apply(vec![(index, placement)]);
        index
    }

    /// Changes spawns, as one undo step.
    pub fn apply(&mut self, changes: Vec<(usize, Placement)>) {
        let mut steps = Vec::new();
        for (index, after) in changes {
            let Some(s) = self.spawns.get_mut(index) else { continue };
            if s.now == after {
                continue;
            }
            steps.push(Step { index, before: s.now, after });
            s.now = after;
        }
        if !steps.is_empty() {
            self.undo.push(steps);
            self.redo.clear();
            self.generation += 1;
        }
    }

    /// Like [`SpawnStore::apply`], merged into the previous undo step (a drag in progress).
    pub fn apply_continued(&mut self, changes: Vec<(usize, Placement)>) {
        let Some(mut last) = self.undo.pop() else { return self.apply(changes) };
        for (index, after) in changes {
            let Some(s) = self.spawns.get_mut(index) else { continue };
            match last.iter_mut().find(|st| st.index == index) {
                Some(st) => st.after = after,
                None => last.push(Step { index, before: s.now, after }),
            }
            s.now = after;
        }
        self.undo.push(last);
        self.generation += 1;
    }

    pub fn undo(&mut self) -> bool {
        let Some(steps) = self.undo.pop() else { return false };
        for st in steps.iter().rev() {
            self.spawns[st.index].now = st.before;
        }
        self.redo.push(steps);
        self.generation += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(steps) = self.redo.pop() else { return false };
        for st in &steps {
            self.spawns[st.index].now = st.after;
        }
        self.undo.push(steps);
        self.generation += 1;
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn changed_count(&self) -> usize {
        self.spawns.iter().filter(|s| s.changed()).count()
    }

    /// Live spawns within `radius` metres (horizontal) of `center` (cry space).
    pub fn near(&self, center: Vec3, radius: f32) -> impl Iterator<Item = usize> + '_ {
        let r2 = radius * radius;
        self.spawns
            .iter()
            .enumerate()
            .filter(move |(_, s)| !s.now.deleted && s.now.pos.truncate().distance_squared(center.truncate()) <= r2)
            .map(|(i, _)| i)
    }

    /// The JSON text of one entry.
    fn entry_text(spawn: &Spawn, name: Option<&str>, path_names: &[String]) -> String {
        if spawn.original == Some(spawn.now)
            && let Some(src) = &spawn.source {
                return src.clone();
            }
        let mut obj = spawn.value.as_ref().and_then(Value::as_object).cloned().unwrap_or_default();
        let p = spawn.now;
        set_ci(&mut obj, "UnitId", Value::from(p.unit_id));
        let mut pos = get_ci(&obj, "Position").and_then(Value::as_object).cloned().unwrap_or_default();
        set_ci(&mut pos, "X", round(p.pos.x, 3));
        set_ci(&mut pos, "Y", round(p.pos.y, 3));
        set_ci(&mut pos, "Z", round(p.pos.z, 3));
        for (key, v) in [("Yaw", p.yaw), ("Pitch", p.pitch), ("Roll", p.roll)] {
            if v != 0.0 || get_ci(&pos, key).is_some() {
                set_ci(&mut pos, key, round(v, 4));
            }
        }
        set_ci(&mut obj, "Position", Value::Object(pos));
        if spawn.kind == Kind::Doodad && (p.scale != 1.0 || get_ci(&obj, "Scale").is_some()) {
            set_ci(&mut obj, "Scale", round(p.scale, 4));
        }
        match path_names.get(p.path as usize).filter(|n| !n.is_empty()) {
            Some(name) => set_ci(&mut obj, "FollowPath", Value::from(name.as_str())),
            None => {
                if let Some(k) = obj.keys().find(|k| k.eq_ignore_ascii_case("FollowPath")).cloned() {
                    obj.remove(&k);
                }
            }
        }
        let mut text = Vec::new();
        let mut ser = serde_json::Serializer::with_formatter(&mut text, serde_json::ser::PrettyFormatter::with_indent(b"    "));
        serde::Serialize::serialize(&Value::Object(obj), &mut ser).expect("json value serialises");
        let text = String::from_utf8(text).unwrap_or_default();
        let mut out = String::new();
        for (i, line) in text.lines().enumerate() {
            if i > 0 {
                out.push_str("\n    ");
            }
            out.push_str(line);
            if let Some(name) = name.filter(|_| line.trim_start().to_ascii_lowercase().starts_with("\"unitid\"")) {
                out.push_str(&format!(" // {}", name.replace(['\n', '\r'], " ")));
            }
        }
        out
    }

    /// Writes every file that has changes into `dir` (normally [`SpawnStore::dir`]) and copies the written files to
    /// each of `mirrors`. A retail file overwritten for the first time is kept as `<name>.editor-orig`.
    /// Returns the written paths.
    pub fn save(&mut self, mirrors: &[PathBuf], name_of: impl Fn(Kind, u32) -> Option<String>) -> anyhow::Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        for (fi, file) in self.files.iter().enumerate() {
            let entries: Vec<&Spawn> = self.spawns.iter().filter(|s| s.file == fi).collect();
            if !entries.iter().any(|s| s.changed()) {
                continue;
            }
            let live: Vec<String> = entries
                .iter()
                .filter(|s| !s.now.deleted)
                .map(|s| Self::entry_text(s, name_of(s.kind, s.now.unit_id).as_deref(), &self.path_names))
                .collect();
            let text = if live.is_empty() { "[\n]\n".to_string() } else { format!("[\n    {}\n]\n", live.join(",\n    ")) };
            if file.path.exists() {
                let backup = file.path.with_extension("json.editor-orig");
                if !backup.exists() {
                    std::fs::copy(&file.path, &backup)?;
                }
            }
            let tmp = file.path.with_extension("json.editor-tmp");
            std::fs::write(&tmp, &text)?;
            std::fs::rename(&tmp, &file.path)?;
            for mirror in mirrors {
                if mirror.is_dir() {
                    let target = mirror.join(file.path.file_name().unwrap());
                    if target.exists() {
                        let backup = target.with_extension("json.editor-orig");
                        if !backup.exists() {
                            std::fs::copy(&target, &backup)?;
                        }
                    }
                    std::fs::write(&target, &text)?;
                    written.push(target);
                } else {
                    warn!("spawn mirror {} is not a folder; skipped", mirror.display());
                }
            }
            written.push(file.path.clone());
        }
        // the saved state is the new baseline
        for s in &mut self.spawns {
            if s.changed() {
                s.source = None;
                s.original = (!s.now.deleted).then_some(s.now);
            }
        }
        for f in &mut self.files {
            if f.path.exists() {
                f.existed = true;
            }
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_path_is_read_set_and_cleared() {
        let dir = std::env::temp_dir().join(format!("aa-editor-follow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let retail = r#"[
    { "UnitId": 1, "FollowPath": "walk_a", "Position": { "X": 1, "Y": 2, "Z": 3 } },
    { "UnitId": 2, "Position": { "X": 4, "Y": 5, "Z": 6 } }
]"#;
        std::fs::write(dir.join("npc_spawns.json"), retail).unwrap();
        let mut store = SpawnStore::load(&dir).unwrap();
        assert_eq!(store.path_of(&store.spawns[0].now), Some("walk_a"));
        assert_eq!(store.path_of(&store.spawns[1].now), None);
        let (mut a, mut b) = (store.spawns[0].now, store.spawns[1].now);
        a.path = 0;
        b.path = store.path_id("walk_b");
        store.apply(vec![(0, a), (1, b)]);
        assert_eq!(store.changed_count(), 2);
        store.save(&[], |_, _| None).unwrap();
        let text = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        assert!(!text.contains("walk_a") && text.contains("\"FollowPath\": \"walk_b\""), "{text}");
        let again = SpawnStore::load(&dir).unwrap();
        assert_eq!(again.path_of(&again.spawns[1].now), Some("walk_b"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn untouched_entries_keep_their_text_and_edits_round_trip() {
        let dir = std::env::temp_dir().join(format!("aa-editor-spawns-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let retail = r#"[
    {
        "UnitId": 3111, // Gorgon
        "Position": { "X": 10.5, "Y": 20, "Z": 3, "Yaw": -45.0 }
    },
    {
        "UnitId": 502,
        "RespawnTime": 15,
        "Position": { "X": 1, "Y": 2, "Z": 3 }
    }
]"#;
        std::fs::write(dir.join("npc_spawns.json"), retail).unwrap();
        let mut store = SpawnStore::load(&dir).unwrap();
        assert_eq!(store.spawns.len(), 2);
        assert_eq!(store.spawns[0].now.yaw, -45.0);
        // move the second, add a doodad
        let mut p = store.spawns[1].now;
        p.pos.x = 100.25;
        store.apply(vec![(1, p)]);
        store.add(Kind::Doodad, Placement::new(1373, Vec3::new(5.0, 6.0, 7.0), 90.0));
        let written = store.save(&[], |_, id| (id == 1373).then(|| "Dead body".to_string())).unwrap();
        assert_eq!(written.len(), 2);
        let text = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        assert!(text.contains("// Gorgon"), "{text}");
        assert!(text.contains("\"RespawnTime\": 15") && text.contains("100.25"), "{text}");
        assert!(dir.join("npc_spawns.json.editor-orig").exists());
        let doodads = std::fs::read_to_string(dir.join("doodad_spawns_editor.json")).unwrap();
        assert!(doodads.contains("\"UnitId\": 1373, // Dead body"), "{doodads}");
        // reload: same data; undo of the delete etc.
        let mut again = SpawnStore::load(&dir).unwrap();
        assert_eq!(again.spawns.len(), 3);
        let moved = again.spawns.iter().find(|s| s.now.unit_id == 502).unwrap();
        assert_eq!(moved.now.pos.x, 100.25);
        let gorgon = again.spawns.iter().position(|s| s.now.unit_id == 3111).unwrap();
        let mut del = again.spawns[gorgon].now;
        del.deleted = true;
        again.apply(vec![(gorgon, del)]);
        assert_eq!(again.changed_count(), 1);
        assert!(again.undo());
        assert_eq!(again.changed_count(), 0);
        assert!(again.redo());
        again.save(&[], |_, _| None).unwrap();
        let text = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        assert!(!text.contains("3111"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retail_files_load_and_a_single_edit_changes_only_its_entry() {
        let source = Path::new("D:/aaemu-wt/bevy/rust/crates/aaemu-game/app/Data/Worlds/main_world");
        if !source.join("npc_spawns.json").exists() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("aa-editor-retail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["npc_spawns.json", "doodad_spawns.json"] {
            std::fs::copy(source.join(name), dir.join(name)).unwrap();
        }
        let mut store = SpawnStore::load(&dir).unwrap();
        let npcs = store.spawns.iter().filter(|s| s.kind == Kind::Npc).count();
        let doodads = store.spawns.len() - npcs;
        assert!(npcs > 10_000 && doodads > 10_000, "{npcs} npcs, {doodads} doodads");
        let original = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        let mut p = store.spawns[0].now;
        p.pos.z += 1.0;
        store.apply(vec![(0, p)]);
        store.save(&[], |_, _| None).unwrap();
        let saved = std::fs::read_to_string(dir.join("npc_spawns.json")).unwrap();
        let (a, b): (Vec<&str>, Vec<&str>) = (original.lines().collect(), saved.lines().collect());
        let differing = a.iter().zip(&b).filter(|(x, y)| x.trim_end() != y.trim_end()).count();
        assert!(differing <= 12, "{differing} lines differ");
        assert_eq!(SpawnStore::load(&dir).unwrap().spawns.len(), store.spawns.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
