//! "Pak export": writes the pak overlay folder (the editor's saved client files, e.g. terrain `heightmap.dat`) into
//! a real `game_pak` the original client reads, with `aa_pak::write::patch_with_undo`.
//!
//! The target must be a copy of the client: `D:/AA1.2` is read-only for this project. Every export first writes an
//! undo file next to the pak (`<pak>.undo-<unix time>`: the old table + header and the old bytes of every slot that is
//! overwritten), so `aa_pak::write::undo` can put the pak back exactly.
//!
//! `--export-pak <path>` exports once at startup (log line `pak export: ...`), for unattended checks.

use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::Editor;

/// The client folder of this project: never written.
const READ_ONLY_CLIENT: &str = "d:/aa1.2";

#[derive(Resource, Default)]
struct PakExport {
    target: String,
    status: String,
    job: Option<JoinHandle<Result<Done, String>>>,
    last_undo: Option<(PathBuf, PathBuf)>,
    /// (files, bytes) in the overlay, refreshed every few seconds while the window is open.
    overlay: Option<(usize, u64)>,
    counted: Option<f64>,
    /// Start with the window open (`--export-pak`).
    open: bool,
}

pub struct Done {
    message: String,
    undo: Option<(PathBuf, PathBuf)>,
}

pub fn plugin(app: &mut App) {
    let mut st = PakExport::default();
    if let Some(target) = crate::arg("--export-pak") {
        st.target = target;
        st.open = true;
    }
    let unattended = crate::arg("--export-pak").is_some();
    app.insert_resource(st).add_systems(Update, poll).add_systems(EguiPrimaryContextPass, window);
    if unattended {
        app.add_systems(Startup, |mut st: ResMut<PakExport>, editor: Res<Editor>| {
            let (target, overlay) = (st.target.clone(), editor.paths.overlay.clone());
            start(&mut st, target, overlay);
        });
    }
}

/// Why `target` must not be written, if it must not.
pub fn refuse(target: &Path) -> Option<String> {
    if target.as_os_str().is_empty() {
        return Some("pick a target pak first (a copy of the client's game_pak)".into());
    }
    let mut forms = vec![std::path::absolute(target).unwrap_or_else(|_| target.to_path_buf())];
    if let Ok(real) = target.canonicalize() {
        forms.push(real);
    }
    for p in forms {
        let s = p.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        let s = s.trim_start_matches("//?/");
        if s == READ_ONLY_CLIENT || s.starts_with(&format!("{READ_ONLY_CLIENT}/")) {
            return Some(format!(
                "refused: {} is inside D:/AA1.2. The 1.2 client folder is read-only for this project; \
                 copy the client and export to the copy's game_pak.",
                target.display()
            ));
        }
    }
    if !target.is_file() {
        return Some(format!("{} does not exist (export patches an existing game_pak)", target.display()));
    }
    None
}

/// Every file under the overlay folder as `(path relative to the overlay, with '/', on disk)`.
pub fn overlay_files(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let path = e.path();
            if e.file_type()?.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(dir) {
                out.push((rel.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The pak name of an overlay file: the name itself when the pak has it (`bin32/...`), else under `game/` like
/// almost every name of the real pak (`game/worlds/main_world/cells/...`). Lowercase, as the pak stores names.
pub fn pak_name(rel: &str, existing: &std::collections::HashSet<String>) -> String {
    let n = aa_pak::normalize(rel);
    if existing.contains(&n) || n.starts_with("game/") { n } else { format!("game/{n}") }
}

/// Exports the overlay into `target`. Returns the status line.
pub fn export(overlay: &Path, target: &Path) -> Result<Done, String> {
    if let Some(why) = refuse(target) {
        return Err(why);
    }
    let found = overlay_files(overlay).map_err(|e| format!("cannot read the overlay {}: {e}", overlay.display()))?;
    if found.is_empty() {
        return Err(format!("the overlay {} is empty: save terrain first", overlay.display()));
    }
    let table = aa_pak::write::read_table(target).map_err(|e| format!("cannot read {}: {e}", target.display()))?;
    let existing: std::collections::HashSet<String> = table.files.iter().map(|f| aa_pak::normalize(&f.name)).collect();
    drop(table);
    let mut files = Vec::with_capacity(found.len());
    for (rel, path) in &found {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        files.push((pak_name(rel, &existing), bytes));
    }
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut undo = target.as_os_str().to_owned();
    undo.push(format!(".undo-{secs}"));
    let undo = PathBuf::from(undo);
    let r = aa_pak::write::patch_with_undo(target, &files, Some(&undo)).map_err(|e| format!("pak export failed: {e}"))?;
    // read back through the normal reader: every exported file must come back byte for byte
    let pak = aa_pak::Pak::open(target).map_err(|e| format!("exported, but the pak does not open again: {e}"))?;
    let bad = files.iter().filter(|(n, b)| pak.entry(n).is_none_or(|e| e.size != b.len() as u64)).count();
    if bad > 0 {
        return Err(format!("exported, but {bad} file(s) do not read back; undo with {}", undo.display()));
    }
    let message = format!(
        "exported {} file(s), {:.2} MB to {} ({} replaced in place, {} into free blocks, {} appended, {} new names); \
         undo file {}",
        files.len(),
        r.bytes as f64 / 1e6,
        target.display(),
        r.in_place,
        r.reused,
        r.appended,
        r.new_names,
        undo.display()
    );
    Ok(Done { message, undo: Some((target.to_path_buf(), undo)) })
}

fn start(st: &mut PakExport, target: String, overlay: PathBuf) {
    if st.job.is_some() {
        return;
    }
    st.status = format!("exporting to {target} ...");
    st.job = Some(std::thread::spawn(move || export(&overlay, Path::new(target.trim()))));
}

fn poll(mut st: ResMut<PakExport>, mut editor: ResMut<Editor>) {
    if !st.job.as_ref().is_some_and(|j| j.is_finished()) {
        return;
    }
    let result = st.job.take().unwrap().join().unwrap_or_else(|_| Err("pak export panicked".into()));
    let text = match result {
        Ok(done) => {
            if done.undo.is_some() {
                st.last_undo = done.undo;
            }
            done.message
        }
        Err(e) => e,
    };
    st.status = text.clone();
    editor.status(format!("pak export: {text}"));
}

fn window(mut contexts: EguiContexts, mut st: ResMut<PakExport>, editor: Res<Editor>, time: Res<Time>) -> Result {
    let ctx = contexts.ctx_mut()?;
    let now = time.elapsed_secs_f64();
    egui::Window::new("Pak export").default_open(st.open).default_pos([1180.0, 40.0]).default_width(380.0).show(ctx, |ui| {
        if st.counted.is_none_or(|t| now - t > 2.0) {
            st.counted = Some(now);
            st.overlay = overlay_files(&editor.paths.overlay)
                .ok()
                .map(|f| (f.len(), f.iter().filter_map(|(_, p)| p.metadata().ok()).map(|m| m.len()).sum()));
        }
        ui.small(format!("Overlay: {}", editor.paths.overlay.display()));
        match st.overlay {
            Some((n, bytes)) => ui.label(format!("{n} file(s), {:.2} MB to export", bytes as f64 / 1e6)),
            None => ui.label("overlay folder not readable"),
        };
        ui.label("Target game_pak (a copy of the client's):");
        ui.text_edit_singleline(&mut st.target);
        let refused = refuse(Path::new(st.target.trim()));
        if let Some(why) = &refused {
            ui.colored_label(egui::Color32::from_rgb(230, 170, 60), why);
        }
        let busy = st.job.is_some();
        if ui.add_enabled(refused.is_none() && !busy, egui::Button::new("Export overlay to pak")).clicked() {
            let (target, overlay) = (st.target.trim().to_string(), editor.paths.overlay.clone());
            start(&mut st, target, overlay);
        }
        if let Some((pak, undo)) = st.last_undo.clone()
            && ui.add_enabled(!busy, egui::Button::new("Undo last export")).clicked()
        {
            st.status = match aa_pak::write::undo(&pak, &undo) {
                Ok(()) => format!("restored {} from {}", pak.display(), undo.display()),
                Err(e) => format!("undo failed: {e}"),
            };
            st.last_undo = None;
        }
        ui.small("Untouched files keep their bytes. An undo file is written next to the pak before each export.");
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

    #[test]
    fn refuses_the_client_folder() {
        for p in ["D:/AA1.2/game_pak", "d:\\aa1.2\\game_pak", "D:/AA1.2", "D:/AA1.2/sub/x"] {
            assert!(refuse(Path::new(p)).is_some_and(|w| w.contains("read-only")), "{p}");
        }
        assert!(refuse(Path::new("")).is_some());
        assert!(refuse(Path::new("C:/no/such/game_pak")).is_some_and(|w| w.contains("does not exist")));
    }

    #[test]
    fn maps_overlay_names() {
        let existing: std::collections::HashSet<String> = ["bin32/zlib1.dll".to_string()].into();
        assert_eq!(
            pak_name("worlds/main_world/cells/015_015/client/terrain/heightmap.dat", &existing),
            "game/worlds/main_world/cells/015_015/client/terrain/heightmap.dat"
        );
        assert_eq!(pak_name("Bin32/zlib1.dll", &existing), "bin32/zlib1.dll");
        assert_eq!(pak_name("game/ui/a.dds", &existing), "game/ui/a.dds");
    }

    /// Overlay -> scratch pak -> read back, with undo.
    #[test]
    fn exports_overlay_into_a_pak() {
        let dir = std::env::temp_dir().join(format!("aa-editor-pak-export-{}", std::process::id()));
        let overlay = dir.join("overlay");
        let rel = "worlds/main_world/cells/015_015/client/terrain/heightmap.dat";
        std::fs::create_dir_all(overlay.join(rel).parent().unwrap()).unwrap();
        std::fs::write(overlay.join(rel), b"new heights").unwrap();
        let pak = dir.join("game_pak");
        let mut w = aa_pak::write::PakWriter::new();
        w.add("game/ui/a.dds", vec![7; 100]).add(&format!("game/{rel}"), vec![1; 5000]);
        w.write(&pak).unwrap();
        let original = std::fs::read(&pak).unwrap();

        let done = export(&overlay, &pak).unwrap();
        let p = aa_pak::Pak::open(&pak).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p.read(rel).unwrap(), b"new heights");
        assert_eq!(p.read("ui/a.dds").unwrap(), vec![7; 100]);
        let (_, undo) = done.undo.unwrap();
        aa_pak::write::undo(&pak, &undo).unwrap();
        assert!(std::fs::read(&pak).unwrap() == original);
        let _ = std::fs::remove_dir_all(dir);
    }
}
