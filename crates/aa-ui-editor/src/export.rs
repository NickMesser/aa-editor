//! Export the overlay's UI files (`scriptsbin/`, `ui/`) into a `game_pak` the original client reads, with an undo
//! file next to it (`aa_pak::write`, the same writer as the world editor's Pak export). The 1.2 client folder
//! `D:/AA1.2` is refused: export into a copy of the client.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::Editor;

pub fn plugin(app: &mut App) {
    app.init_resource::<Export>().add_systems(EguiPrimaryContextPass, window);
}

#[derive(Resource, Default)]
struct Export {
    target: String,
    last_undo: Option<(PathBuf, PathBuf)>,
    result: String,
}

/// Every overlay file under `scriptsbin/` and `ui/`, as (pak name, bytes).
pub fn ui_files(overlay: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for top in ["scriptsbin", "ui"] {
        let mut stack = vec![overlay.join(top)];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(rel) = p.strip_prefix(overlay)
                    && let Ok(bytes) = std::fs::read(&p)
                {
                    let name = format!("game/{}", rel.to_string_lossy().replace('\\', "/").to_ascii_lowercase());
                    out.push((name, bytes));
                }
            }
        }
    }
    out.sort();
    out
}

/// The original client folder is read-only for this project.
pub fn refused(target: &Path) -> bool {
    let t = target.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    t == "d:/aa1.2" || t.starts_with("d:/aa1.2/")
}

fn window(mut contexts: EguiContexts, mut editor: ResMut<Editor>, mut ex: ResMut<Export>) -> Result {
    if editor.hidden || !editor.windows.export {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    let mut open = true;
    let files = ui_files(&editor.sources.overlay);
    egui::Window::new("Export to game_pak").open(&mut open).default_pos([700.0, 300.0]).show(&ctx, |ui| {
        ui.label(format!("{} UI file(s) in the overlay {}", files.len(), editor.sources.overlay.display()));
        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
            for (n, b) in &files {
                ui.monospace(format!("{n}  ({} bytes)", b.len()));
            }
        });
        ui.horizontal(|ui| {
            ui.label("game_pak");
            ui.add(egui::TextEdit::singleline(&mut ex.target).hint_text("C:/AA1.2-copy/game_pak").desired_width(320.0));
        });
        ui.weak("Export into a copy of the client. An undo file is written next to the pak.");
        let target = PathBuf::from(ex.target.trim());
        let ok = !files.is_empty() && target.is_file() && !refused(&target);
        if refused(&target) {
            ui.colored_label(egui::Color32::from_rgb(255, 120, 100), "refused: the 1.2 client folder is read-only; use a copy");
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(ok, egui::Button::new("Export")).clicked() {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let undo = target.with_extension(format!("undo-{secs}"));
                ex.result = match aa_pak::write::patch_with_undo(&target, &files, Some(&undo)) {
                    Ok(r) => {
                        ex.last_undo = Some((target.clone(), undo));
                        format!("exported: {} in place, {} reused, {} appended, {} new names", r.in_place, r.reused, r.appended, r.new_names)
                    }
                    Err(e) => format!("export failed: {e}"),
                };
                editor.status(ex.result.clone());
            }
            if let Some((pak, undo)) = ex.last_undo.clone()
                && ui.button("Undo last export").clicked()
            {
                ex.result = match aa_pak::write::undo(&pak, &undo) {
                    Ok(()) => {
                        ex.last_undo = None;
                        "undone".into()
                    }
                    Err(e) => format!("undo failed: {e}"),
                };
            }
        });
        if !ex.result.is_empty() {
            ui.label(&ex.result);
        }
    });
    if !open {
        editor.windows.export = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_the_client_folder() {
        assert!(refused(Path::new("D:/AA1.2/game_pak")));
        assert!(refused(Path::new("d:\\aa1.2\\game_pak")));
        assert!(!refused(Path::new("C:/AA1.2-copy/game_pak")));
    }

    #[test]
    fn collects_overlay_ui_files() {
        let dir = std::env::temp_dir().join(format!("ui-editor-export-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("scriptsbin/x2ui/hud")).unwrap();
        std::fs::create_dir_all(dir.join("worlds")).unwrap();
        std::fs::write(dir.join("scriptsbin/x2ui/hud/Clock.alb"), b"x").unwrap();
        std::fs::write(dir.join("worlds/a.dat"), b"y").unwrap();
        let f = ui_files(&dir);
        assert_eq!(f, vec![("game/scriptsbin/x2ui/hud/clock.alb".to_string(), b"x".to_vec())]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
