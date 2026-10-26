//! "Play on server": save everything, (re)start the bevy lane's AAEmu server, and run the remade client against it
//! with the editor's pak overlay, so NPC AI, quests and the rest of the real server logic can be tried on the edits.
//!
//! * The window (menu bar button "Play on server", F6) shows whether the lane's ports listen, has the guarded
//!   **Restart lane server** button, **Launch client** (saves first) and **Stop client**.
//! * Restart runs the lane's documented commands, `Scripts/live-stack.sh stop` then `start`, from the lane worktree,
//!   with the output in a log file (not a pipe: the servers inherit it, a pipe would never close). It does not run
//!   `sync`: that copies the lane's source `app/Data` over the runtime folder and would overwrite the database
//!   the editor saved there. Spawn files are mirrored into the runtime folder by the editor's own save.
//! * The client is `aa-client --enter-world --account A --login 127.0.0.1:27237 [--character C]`, with
//!   `AA_PAK_OVERLAY` set to the editor's overlay. Teleport: the client has no start-position flag that reaches the
//!   server (`--pos` is local only), so the GM chat command `/move X Y Z` is typed through the client's scripted
//!   keyboard (`--hud-key enter`, `--hud-type`, `--hud-key enter`), to the ground under the editor camera.
//!
//! Flags: `--client-exe <path>`, `--lane-dir <dir>` (default `D:/aaemu-wt/bevy`), `--lane-name <name>` (`bevy`),
//! `--lane-ports <login,game,stream>` (`27237,27239,27250`), `--play-account <name>` (`bevylive1`),
//! `--play-character <name>`, `--play-password-file <file>`, `--client-arg "<args>"` (repeatable, split at spaces and passed to the client; a bare `--shot` would clash with the editor's own),
//! `--dry-run` (the restart only logs its command), `--play-server` (open the window at start),
//! `--play-server-auto <launch|restart>` (press that button at start; `restart` needs `--dry-run`).

use std::fs::File;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use aa_world::{TerrainQuery, bevy_to_cry};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};

use crate::camera::EditorCamera;
use crate::terrain_edit::Heights;
use crate::{Editor, Request};

/// The lane's three server ports.
const PORT_NAMES: [&str; 3] = ["login", "game", "stream"];

/// What the window needs besides the editor state.
#[derive(Resource)]
pub struct PlayServer {
    pub open: bool,
    pub client_exe: PathBuf,
    pub lane_dir: PathBuf,
    pub lane_name: String,
    pub ports: [u16; 3],
    pub account: String,
    pub character: String,
    pub password_file: String,
    pub extra_args: Vec<String>,
    pub dry_run: bool,
    pub teleport: bool,
    /// Where the logs go (restart and client output).
    pub log_dir: PathBuf,
    listening: [bool; 3],
    last_probe: f64,
    /// Time until which the restart button is armed.
    armed_until: f64,
    restart: Option<Restart>,
    client: Option<Client>,
    /// A launch waiting for the save requests to be done: frames waited.
    pending_launch: Option<u32>,
    message: String,
}

struct Restart {
    child: Option<Child>,
    log: PathBuf,
    started: Instant,
    /// Exit code of the script once it ended.
    done: Option<bool>,
}

struct Client {
    child: Child,
    log: PathBuf,
    started: Instant,
    exit: Option<String>,
}

/// The bash that runs the lane scripts (Git for Windows).
pub fn bash_path() -> PathBuf {
    for p in ["C:/Program Files/Git/bin/bash.exe", "C:/Program Files/Git/usr/bin/bash.exe"] {
        if Path::new(p).is_file() {
            return PathBuf::from(p);
        }
    }
    PathBuf::from("bash")
}

/// The lane's restart as a script line: the documented `stop`, then `start` (which returns when the game server
/// registered at the login server).
pub fn restart_script(lane_dir: &Path) -> String {
    let dir = lane_dir.to_string_lossy().replace('\\', "/");
    format!("cd \"{dir}\" && Scripts/live-stack.sh stop && Scripts/live-stack.sh start")
}

/// The whole restart command line, for display and the dry run.
pub fn restart_command_line(lane_dir: &Path, lane: &str, log: &Path) -> String {
    format!(
        "AAEMU_LANE={lane} \"{}\" -c '{}' > \"{}\" 2>&1",
        bash_path().display(),
        restart_script(lane_dir),
        log.display()
    )
}

/// The client's command line arguments (without the exe). `teleport`: the cry position to `/move` to.
pub fn client_args(s: &PlayServer, teleport: Option<Vec3>) -> Vec<String> {
    let mut a = vec![
        "--enter-world".to_string(),
        "--account".into(),
        s.account.clone(),
        "--login".into(),
        format!("127.0.0.1:{}", s.ports[0]),
    ];
    if !s.character.is_empty() {
        a.extend(["--character".to_string(), s.character.clone()]);
    }
    if !s.password_file.is_empty() {
        a.extend(["--password-file".to_string(), s.password_file.clone()]);
    }
    if let Some(p) = teleport {
        // seconds after the HUD is up: open the chat line, type the GM command, send it
        a.extend(["--hud-key".to_string(), "enter".into(), "6".into()]);
        a.extend(["--hud-type".to_string(), format!("/move {:.1} {:.1} {:.1}", p.x, p.y, p.z), "6.5".into()]);
        a.extend(["--hud-key".to_string(), "enter".into(), "7".into()]);
    }
    a.extend(s.extra_args.iter().cloned());
    a
}

/// Is something listening on this loopback port?
pub fn port_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
}

fn flag_value(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn default_client_exe(root: &Path) -> PathBuf {
    // a release build of this branch's client (it must have the pak overlay: it is newer than the editor's)
    for p in [
        "C:/Users/messe/aa-editor-target/release/aa-client.exe",
        "C:/Users/messe/aa-editor-target/debug/aa-client.exe",
    ] {
        if Path::new(p).is_file() {
            return PathBuf::from(p);
        }
    }
    root.join("target/debug/aa-client.exe")
}

pub fn plugin(app: &mut App) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ports = flag_value("--lane-ports")
        .and_then(|v| {
            let n: Vec<u16> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            <[u16; 3]>::try_from(n).ok()
        })
        .unwrap_or([27237, 27239, 27250]);
    let log_dir = absolute(&root.join("data/local/editor"));
    let state = PlayServer {
        open: std::env::args().any(|a| a == "--play-server"),
        client_exe: flag_value("--client-exe").map(PathBuf::from).unwrap_or_else(|| default_client_exe(&root)),
        lane_dir: flag_value("--lane-dir").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("D:/aaemu-wt/bevy")),
        lane_name: flag_value("--lane-name").unwrap_or_else(|| "bevy".into()),
        ports,
        account: flag_value("--play-account").unwrap_or_else(|| "bevylive1".into()),
        character: flag_value("--play-character").unwrap_or_default(),
        password_file: flag_value("--play-password-file").unwrap_or_default(),
        extra_args: {
            let a: Vec<String> = std::env::args().collect();
            a.windows(2).filter(|w| w[0] == "--client-arg").flat_map(|w| w[1].split_whitespace().map(String::from).collect::<Vec<_>>()).collect()
        },
        dry_run: std::env::args().any(|a| a == "--dry-run"),
        teleport: true,
        log_dir,
        listening: [false; 3],
        last_probe: f64::MIN,
        armed_until: 0.0,
        restart: None,
        client: None,
        pending_launch: None,
        message: String::new(),
    };
    app.insert_resource(state)
        .add_systems(Update, (requests, probe, launch_when_saved, poll_children).chain())
        .add_systems(EguiPrimaryContextPass, window);
    if let Some(what) = flag_value("--play-server-auto") {
        app.add_systems(Update, move |mut s: ResMut<PlayServer>, mut editor: ResMut<Editor>, time: Res<Time>, mut done: Local<bool>| {
            if *done || time.elapsed_secs() < 3.0 {
                return;
            }
            *done = true;
            match what.as_str() {
                "launch" => start_launch(&mut s, &mut editor),
                "restart" if s.dry_run => {
                    s.armed_until = f64::MAX;
                    start_restart(&mut s, time.elapsed_secs_f64());
                }
                _ => error!("--play-server-auto: use `launch`, or `restart` together with --dry-run"),
            }
        });
    }
}

/// The menu bar's "Play on server" button toggles the window.
fn requests(mut editor: ResMut<Editor>, mut s: ResMut<PlayServer>) {
    if let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::PlayOnServer)) {
        editor.requests.remove(i);
        s.open = !s.open;
    }
}

/// Port check every 2 s while the window is open (or a restart / launch is going on).
fn probe(mut s: ResMut<PlayServer>, time: Res<Time>) {
    let now = time.elapsed_secs_f64();
    if !(s.open || s.restart.is_some()) || now - s.last_probe < 2.0 {
        return;
    }
    s.last_probe = now;
    let ports = s.ports;
    s.listening = ports.map(port_open);
}

fn start_restart(s: &mut PlayServer, now: f64) {
    if s.restart.as_ref().is_some_and(|r| r.done.is_none()) {
        return;
    }
    if now > s.armed_until {
        return;
    }
    s.armed_until = 0.0;
    let _ = std::fs::create_dir_all(&s.log_dir);
    let log = s.log_dir.join("play_server_restart.log");
    let line = restart_command_line(&s.lane_dir, &s.lane_name, &log);
    if s.dry_run {
        info!("play on server: DRY RUN, would run: {line}");
        s.message = "dry run: the restart command was only logged".into();
        s.restart = Some(Restart { child: None, log, started: Instant::now(), done: Some(true) });
        return;
    }
    let file = match File::create(&log).and_then(|f| f.try_clone().map(|g| (f, g))) {
        Ok(f) => f,
        Err(e) => {
            s.message = format!("cannot write {}: {e}", log.display());
            return;
        }
    };
    info!("play on server: restarting the lane server: {line}");
    let mut cmd = Command::new(bash_path());
    cmd.arg("-c")
        .arg(restart_script(&s.lane_dir))
        .env("AAEMU_LANE", &s.lane_name)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file.0))
        .stderr(Stdio::from(file.1));
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // no console window
    match cmd.spawn() {
        Ok(child) => {
            s.message = "restarting: stop, then start (about a minute)".into();
            s.restart = Some(Restart { child: Some(child), log, started: Instant::now(), done: None });
        }
        Err(e) => s.message = format!("cannot run bash: {e}"),
    }
}

fn start_launch(s: &mut PlayServer, editor: &mut Editor) {
    if s.client.as_ref().is_some_and(|c| c.exit.is_none()) {
        s.message = "the client is already running".into();
        return;
    }
    // everything that exists: spawns, terrain (other agents' save requests are pushed here too)
    editor.requests.push(Request::SaveSpawns);
    editor.requests.push(Request::SaveTerrain);
    s.pending_launch = Some(0);
    s.message = "saving, then launching the client".into();
}

/// Waits for the save requests to be consumed, then starts the client at the camera's ground position.
fn launch_when_saved(
    mut s: ResMut<PlayServer>,
    mut editor: ResMut<Editor>,
    cam: Query<&Transform, With<EditorCamera>>,
    heights: Res<Heights>,
    query: Res<TerrainQuery>,
) {
    let Some(frames) = s.pending_launch else { return };
    let saving = editor.requests.iter().any(|r| matches!(r, Request::SaveSpawns | Request::SaveTerrain));
    if saving || frames < 2 {
        s.pending_launch = Some(frames + 1);
        return;
    }
    s.pending_launch = None;
    let teleport = s.teleport.then(|| cam.single().ok()).flatten().map(|t| {
        let eye = bevy_to_cry(t.translation);
        let ground = heights.height_at(&query, eye.x, eye.y).unwrap_or(eye.z);
        Vec3::new(eye.x, eye.y, ground + 1.0)
    });
    if !s.client_exe.is_file() {
        let msg = format!("client exe not found: {} (set --client-exe)", s.client_exe.display());
        editor.status(msg.clone());
        s.message = msg;
        return;
    }
    let _ = std::fs::create_dir_all(&s.log_dir);
    let log = s.log_dir.join("play_server_client.log");
    let file = match File::create(&log).and_then(|f| f.try_clone().map(|g| (f, g))) {
        Ok(f) => f,
        Err(e) => {
            s.message = format!("cannot write {}: {e}", log.display());
            return;
        }
    };
    let overlay = absolute(&editor.paths.overlay);
    let args = client_args(&s, teleport);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    info!("play on server: {} {} (AA_PAK_OVERLAY={})", s.client_exe.display(), args.join(" "), overlay.display());
    match Command::new(&s.client_exe)
        .args(&args)
        .current_dir(&root)
        .env("AA_PAK_OVERLAY", &overlay)
        .stdin(Stdio::null())
        .stdout(Stdio::from(file.0))
        .stderr(Stdio::from(file.1))
        .spawn()
    {
        Ok(child) => {
            let msg = match teleport {
                Some(p) => format!("client started (pid {}), will /move to {:.0} {:.0} {:.0}", child.id(), p.x, p.y, p.z),
                None => format!("client started (pid {})", child.id()),
            };
            editor.status(msg.clone());
            s.message = msg;
            s.client = Some(Client { child, log, started: Instant::now(), exit: None });
        }
        Err(e) => {
            let msg = format!("cannot start {}: {e}", s.client_exe.display());
            editor.status(msg.clone());
            s.message = msg;
        }
    }
}

fn absolute(p: &Path) -> PathBuf {
    let p = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
    // drop `dir/../..` parts without the `\\?\` form of canonicalize
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

fn poll_children(mut s: ResMut<PlayServer>) {
    if let Some(r) = s.restart.as_mut()
        && r.done.is_none()
        && let Some(child) = r.child.as_mut()
        && let Ok(Some(status)) = child.try_wait()
    {
        r.done = Some(status.success());
    }
    if let Some(c) = s.client.as_mut()
        && c.exit.is_none()
        && let Ok(Some(status)) = c.child.try_wait()
    {
        c.exit = Some(format!("exited: {status}"));
    }
}

fn tail(path: &Path, n: usize) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| l.chars().take(140).collect()).collect()
}

fn window(mut contexts: EguiContexts, mut s: ResMut<PlayServer>, mut editor: ResMut<Editor>, time: Res<Time>) -> Result {
    if !s.open || editor.playing {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let now = time.elapsed_secs_f64();
    let mut open = true;
    egui::Window::new("Play on server").open(&mut open).default_pos([700.0, 60.0]).default_width(470.0).show(ctx, |ui| {
        ui.small("Runs the remade client against the bevy lane's AAEmu server: NPC AI, quests and the rest are real.");
        ui.small("The server reads spawns and the game database only at startup: restart it after saving those.");
        ui.separator();
        ui.horizontal(|ui| {
            for (i, name) in PORT_NAMES.iter().enumerate() {
                let up = s.listening[i];
                let color = if up { egui::Color32::from_rgb(90, 200, 90) } else { egui::Color32::from_rgb(220, 90, 90) };
                ui.colored_label(color, format!("● {name} :{} {}", s.ports[i], if up { "listening" } else { "down" }));
            }
        });
        let server_up = s.listening[0] && s.listening[1];
        let restarting = s.restart.as_ref().is_some_and(|r| r.done.is_none());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let armed = now < s.armed_until;
            let label = if armed { "Click again to restart the bevy lane server" } else { "Restart lane server" };
            let button = egui::Button::new(label).fill(if armed { egui::Color32::from_rgb(150, 60, 40) } else { ui.visuals().widgets.inactive.bg_fill });
            if ui.add_enabled(!restarting, button).clicked() {
                if armed {
                    start_restart(&mut s, now);
                } else {
                    s.armed_until = now + 6.0;
                }
            }
            if armed {
                ui.small("(stops the lane's servers and its real client, then starts them)");
            }
        });
        if let Some(r) = &s.restart {
            let secs = r.started.elapsed().as_secs();
            match r.done {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        let phase = match (s.listening[0] || s.listening[1], secs) {
                            (true, _) => "stopping / starting",
                            (false, _) => "starting (loading data, about a minute)",
                        };
                        ui.label(format!("restart running {secs}s: {phase}"));
                    });
                }
                Some(ok) => {
                    let text = if r.child.is_none() { "dry run (nothing was run)" } else if ok { "restart finished" } else { "restart FAILED" };
                    ui.colored_label(
                        if ok { egui::Color32::from_rgb(90, 200, 90) } else { egui::Color32::from_rgb(220, 90, 90) },
                        format!("{text} after {secs}s"),
                    );
                }
            }
            for l in tail(&r.log, 3) {
                ui.small(l);
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Account");
            ui.add(egui::TextEdit::singleline(&mut s.account).desired_width(110.0));
            ui.label("Character");
            ui.add(egui::TextEdit::singleline(&mut s.character).hint_text("first").desired_width(110.0));
        });
        ui.checkbox(&mut s.teleport, "Move the character to the camera's ground position (GM /move)");
        let running = s.client.as_ref().is_some_and(|c| c.exit.is_none());
        ui.horizontal(|ui| {
            let can = !running && !restarting && s.pending_launch.is_none();
            if ui.add_enabled(can, egui::Button::new("▶ Save all and launch client")).clicked() {
                start_launch(&mut s, &mut editor);
            }
            if ui.add_enabled(running, egui::Button::new("■ Stop client")).clicked()
                && let Some(c) = s.client.as_mut()
            {
                let _ = c.child.kill();
                let _ = c.child.wait();
                c.exit = Some("stopped from the editor".into());
                s.message = "client stopped".into();
            }
        });
        if !server_up {
            ui.colored_label(egui::Color32::from_rgb(230, 170, 60), "The lane server is not up: the client cannot log in.");
        }
        if let Some(c) = &s.client {
            let state = c.exit.clone().unwrap_or_else(|| format!("running, pid {}, {}s", c.child.id(), c.started.elapsed().as_secs()));
            ui.small(format!("client: {state}"));
            ui.small(format!("log: {}", c.log.display()));
        }
        ui.small(format!("exe: {}", s.client_exe.display()));
        if !s.message.is_empty() {
            ui.label(&s.message);
        }
    });
    if !open {
        s.open = false;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PlayServer {
        PlayServer {
            open: false,
            client_exe: PathBuf::from("aa-client.exe"),
            lane_dir: PathBuf::from("D:/aaemu-wt/bevy"),
            lane_name: "bevy".into(),
            ports: [27237, 27239, 27250],
            account: "bevylive1".into(),
            character: String::new(),
            password_file: String::new(),
            extra_args: vec!["--exit-after".into(), "5".into()],
            dry_run: true,
            teleport: true,
            log_dir: PathBuf::new(),
            listening: [false; 3],
            last_probe: 0.0,
            armed_until: 0.0,
            restart: None,
            client: None,
            pending_launch: None,
            message: String::new(),
        }
    }

    #[test]
    fn restart_is_the_documented_stop_then_start() {
        let s = restart_script(Path::new("D:\\aaemu-wt\\bevy"));
        assert_eq!(s, "cd \"D:/aaemu-wt/bevy\" && Scripts/live-stack.sh stop && Scripts/live-stack.sh start");
        let line = restart_command_line(Path::new("D:/aaemu-wt/bevy"), "bevy", Path::new("x.log"));
        assert!(line.starts_with("AAEMU_LANE=bevy ") && line.ends_with("> \"x.log\" 2>&1"));
        assert!(!line.contains("| tail") && !line.contains(" sync"));
    }

    #[test]
    fn client_command_line() {
        let a = client_args(&state(), Some(Vec3::new(15578.0, 15382.0, 126.25)));
        let joined = a.join(" ");
        assert!(joined.starts_with("--enter-world --account bevylive1 --login 127.0.0.1:27237"));
        assert!(joined.contains("--hud-type /move 15578.0 15382.0 126.2 6.5"), "{joined}");
        assert!(joined.ends_with("--exit-after 5"));
        assert!(!client_args(&state(), None).join(" ").contains("--hud-type"));
    }

    #[test]
    fn port_check() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(port_open(port));
        drop(l);
        assert!(!port_open(port));
    }

    #[test]
    fn absolute_drops_parent_parts() {
        assert_eq!(absolute(Path::new("C:/a/b/../../c")), PathBuf::from("C:/c"));
    }
}
