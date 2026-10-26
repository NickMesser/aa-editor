//! Fake game API for the examples: permissive `X2*` objects, native constants, UI texts from the
//! game database, and a small scenario (3 servers). The real app registers real objects.

use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// Startup system: make the window size physical pixels (`--size 1920x1080` = the client's
/// 1920x1080 framebuffer even on a scaled desktop), so screenshots compare 1:1 with `reference/`.
pub fn physical_pixels(mut q: Query<&mut Window, With<PrimaryWindow>>) {
    let (pw, ph) = aa_dev::DevArgs::from_env().size;
    for mut w in &mut q {
        w.resolution.set_scale_factor_override(Some(1.0));
        w.resolution.set_physical_resolution(pw, ph);
    }
}

/// Game API objects the UI scripts use (all `X2*` plus a few CryEngine ones).
pub const GAME_OBJECTS: &[&str] = &[
    "X2",
    "X2Ability",
    "X2Achievement",
    "X2Action",
    "X2Auction",
    "X2Bag",
    "X2Bank",
    "X2BattleField",
    "X2Book",
    "X2Bot",
    "X2Camera",
    "X2Chat",
    "X2Coffer",
    "X2Craft",
    "X2Cursor",
    "X2Customizer",
    "X2Debug",
    "X2Decal",
    "X2Demo",
    "X2DialogManager",
    "X2Dominion",
    "X2Equipment",
    "X2Faction",
    "X2Family",
    "X2Friend",
    "X2Gm",
    "X2GoodsMail",
    "X2Helper",
    "X2Hotkey",
    "X2House",
    "X2InGameShop",
    "X2Input",
    "X2Interaction",
    "X2Item",
    "X2ItemEnchant",
    "X2ItemLookConverter",
    "X2Locale",
    "X2LoginCharacter",
    "X2Loot",
    "X2Mail",
    "X2Map",
    "X2Mate",
    "X2MiaMail",
    "X2NameTag",
    "X2Nation",
    "X2Option",
    "X2Player",
    "X2PremiumService",
    "X2Priest",
    "X2Quest",
    "X2Racing",
    "X2Rank",
    "X2Security",
    "X2SiegeWeapon",
    "X2Skill",
    "X2Sound",
    "X2Store",
    "X2Team",
    "X2Time",
    "X2Trade",
    "X2Trial",
    "X2Ucc",
    "X2Unit",
    "X2UserMusic",
    "X2Util",
    "X2Warp",
    "X2World",
    "Console",
    "Movie",
    "DockingManager",
];

pub struct FakeCharacter {
    pub name: &'static str,
    /// `X2Unit:GetRaceStr` names: `nuian`, `elf`, `ferre`, `hariharan`, ...
    pub race: &'static str,
    /// `male` / `female`.
    pub gender: &'static str,
    pub level: i64,
    /// `ABILITY_*` value (1 = fight / Battlerage).
    pub ability: i64,
    pub zone: &'static str,
    pub faction: &'static str,
}

#[allow(dead_code)]
/// `--characters 0|1`: none, or the reference's `Refone` (Nuian male, level 1, Battlerage).
pub fn characters_from_args() -> Vec<FakeCharacter> {
    let args: Vec<String> = std::env::args().collect();
    let v = args
        .iter()
        .position(|a| a == "--characters")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    match v {
        Some("0") => vec![],
        _ => vec![FakeCharacter {
            name: "Refone",
            race: "nuian",
            gender: "male",
            level: 1,
            ability: 1,
            zone: "Solzreed Peninsula",
            faction: "Crescent Throne",
        }],
    }
}

pub struct FakeWorld {
    pub id: i64,
    pub name: &'static str,
    pub available: bool,
    pub congestion: i64,
    pub characters: i64,
}

pub fn default_worlds() -> Vec<FakeWorld> {
    vec![
        FakeWorld {
            id: 1,
            name: "Lane-bevy",
            available: true,
            congestion: 0,
            characters: 2,
        },
        FakeWorld {
            id: 2,
            name: "Kyprosa",
            available: true,
            congestion: 1,
            characters: 0,
        },
        FakeWorld {
            id: 3,
            name: "Melisara",
            available: false,
            congestion: 2,
            characters: 1,
        },
    ]
}

/// `--worlds ref0|ref1` reproduces the reference captures (one server `Lane-bevy` with 0 / 1
/// characters); default: [`default_worlds`].
pub fn worlds_from_args() -> Vec<FakeWorld> {
    let args: Vec<String> = std::env::args().collect();
    let v = args
        .iter()
        .position(|a| a == "--worlds")
        .and_then(|i| args.get(i + 1))
        .map(String::as_str);
    match v {
        Some("ref0") => vec![FakeWorld {
            id: 1,
            name: "Lane-bevy",
            available: true,
            congestion: 0,
            characters: 0,
        }],
        Some("ref1") => vec![FakeWorld {
            id: 1,
            name: "Lane-bevy",
            available: true,
            congestion: 0,
            characters: 1,
        }],
        _ => default_worlds(),
    }
}

/// `(category, key) -> text` for a locale, from the server's copy of the game database.
/// The game database the fakes read UI texts from (read-only).
pub const GAME_DB: &str = "D:/aaemu-live/rs-game/Data/compact.sqlite3";

/// Toolkit settings of the examples: UI texts from [`GAME_DB`].
pub fn config() -> aa_ui::UiConfig {
    aa_ui::UiConfig {
        ui_texts_db: Some(GAME_DB.into()),
        ..Default::default()
    }
}

/// Install the fake game layer into a fresh runtime (before any script runs).
#[allow(dead_code)]
pub fn install(rt: &mut UiRuntime, locale: &str, worlds: &[FakeWorld]) {
    install_with(rt, locale, worlds, &[]);
}

pub fn install_with(
    rt: &mut UiRuntime,
    locale: &str,
    worlds: &[FakeWorld],
    characters: &[FakeCharacter],
) {
    let lua = &rt.lua;
    for name in GAME_OBJECTS {
        aa_ui::lua::install_game_stub(lua, name).unwrap();
    }
    // `ADDON` is the same native table as `X2`.
    let g = lua.globals();
    let x2: mlua::Table = g.raw_get("X2").unwrap();
    g.raw_set("ADDON", x2).unwrap();
    // aa-ui installs the client's real constants (`SERVER_TEXT`, `TRION`, ...); any other ALL_CAPS
    // global the UI reads but never assigns gets a unique value.
    let mut next = 1000i64;
    for name in include_str!("native_constants.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        if g.raw_get::<mlua::Value>(name).unwrap().is_nil() {
            g.raw_set(name, next).unwrap();
            next += 1;
        }
    }
    // Scenario data.
    let fake = lua.create_table().unwrap();
    fake.raw_set("locale", locale).unwrap();
    let list = lua.create_table().unwrap();
    for (i, w) in worlds.iter().enumerate() {
        let t = lua.create_table().unwrap();
        t.raw_set("id", w.id).unwrap();
        t.raw_set("name", w.name).unwrap();
        t.raw_set("available", w.available).unwrap();
        t.raw_set("congestion", w.congestion).unwrap();
        t.raw_set("characters", w.characters).unwrap();
        list.raw_set(i + 1, t).unwrap();
    }
    fake.raw_set("worlds", list).unwrap();
    let chars = lua.create_table().unwrap();
    for (i, c) in characters.iter().enumerate() {
        let t = lua.create_table().unwrap();
        t.raw_set("name", c.name).unwrap();
        t.raw_set("race", c.race).unwrap();
        t.raw_set("gender", c.gender).unwrap();
        t.raw_set("level", c.level).unwrap();
        t.raw_set("ability", c.ability).unwrap();
        t.raw_set("zone", c.zone).unwrap();
        t.raw_set("faction", c.faction).unwrap();
        chars.raw_set(i + 1, t).unwrap();
    }
    fake.raw_set("characters", chars).unwrap();
    // FAKE.fire_later(name, ...): a UI event at the next frame (a server answer).
    let fire = lua
        .create_function(|lua, (name, rest): (String, mlua::MultiValue)| {
            aa_ui::lua::queue_ui_event(lua, &name, rest.into_iter().collect());
            Ok(())
        })
        .unwrap();
    fake.raw_set("fire_later", fire).unwrap();
    // FAKE.fire_now(name, ...): a UI event right away (native state changes the scripts wait for).
    let fire_now = lua
        .create_function(|lua, (name, rest): (String, mlua::MultiValue)| {
            aa_ui::lua::fire_ui_event(lua, &name, rest.into_iter().collect());
            Ok(())
        })
        .unwrap();
    fake.raw_set("fire_now", fire_now).unwrap();
    g.raw_set("FAKE", fake).unwrap();
    if let Err(e) = rt.exec("@fake_game.lua", include_str!("fake_game.lua")) {
        eprintln!("fake: fake_game.lua: {e}");
    }
}

/// `--dump <file.json>`: write the UI tree (reference `ui.json` format) once, `--dump-after`
/// seconds (default 3) after start.
pub fn dump_tree(rt: Option<NonSend<UiRuntime>>, time: Res<Time>, mut done: Local<bool>) {
    let Some(rt) = rt else { return };
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let Some(path) = value("--dump") else { return };
    let after: f32 = value("--dump-after")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0);
    if *done || time.elapsed_secs() < after {
        return;
    }
    *done = true;
    if let Some(dir) = std::path::Path::new(&path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::write(&path, rt.dump_json()) {
        Ok(()) => info!("fake: UI tree -> {path}"),
        Err(e) => warn!("fake: cannot write {path}: {e}"),
    }
}

/// Scripted input for screenshots: `--hover X Y T` moves the virtual pointer at T seconds,
/// `--click X Y T` clicks there (press at T, release one frame later), `--wheel X Y DELTA T`
/// turns the mouse wheel there, `--lua CODE T` runs Lua. Flags may repeat; coordinates are
/// physical pixels.
pub fn scripted_input(
    rt: Option<NonSend<UiRuntime>>,
    time: Res<Time>,
    mut ptr: ResMut<aa_ui::UiVirtualPointer>,
    mut done: Local<Vec<usize>>,
    mut release_next: Local<bool>,
    mut right_release_next: Local<bool>,
) {
    let Some(rt) = rt else { return };
    if std::mem::take(&mut *release_next) {
        ptr.release = true;
    }
    if std::mem::take(&mut *right_release_next) {
        ptr.right_release = true;
    }
    let args: Vec<String> = std::env::args().collect();
    let t = time.elapsed_secs();
    for (i, a) in args.iter().enumerate() {
        if done.contains(&i) {
            continue;
        }
        let num = |k: usize| args.get(i + k).and_then(|s| s.parse::<f32>().ok());
        match a.as_str() {
            "--hover" | "--click" | "--rclick" => {
                let (Some(x), Some(y), Some(at)) = (num(1), num(2), num(3)) else {
                    continue;
                };
                if t < at {
                    continue;
                }
                done.push(i);
                ptr.pos = Some(Vec2::new(x, y));
                if a == "--click" {
                    ptr.press = true;
                    *release_next = true;
                }
                // `--rclick X Y T`: a right click (OnClick "RightButton" fires on release).
                if a == "--rclick" {
                    ptr.right_press = true;
                    *right_release_next = true;
                }
                info!("fake: {a} {x} {y}");
            }
            // `--wheel X Y DELTA T`: the pointer goes to X, Y and the wheel turns DELTA notches
            // (> 0 up: scrolls chat back, < 0 down).
            "--wheel" => {
                let (Some(x), Some(y), Some(delta), Some(at)) = (num(1), num(2), num(3), num(4))
                else {
                    continue;
                };
                if t < at {
                    continue;
                }
                done.push(i);
                ptr.pos = Some(Vec2::new(x, y));
                ptr.wheel += delta;
                info!("fake: --wheel {x} {y} {delta}");
            }
            "--lua" => {
                let (Some(code), Some(at)) = (args.get(i + 1), num(2)) else {
                    continue;
                };
                if t < at {
                    continue;
                }
                done.push(i);
                if let Err(e) = rt.exec("@--lua", code) {
                    warn!("fake: --lua: {e}");
                }
            }
            // `--type TEXT T`: typed into the focused edit box; `--key NAME T`: enter, escape,
            // backspace, tab, left, right, home, end, delete.
            "--type" | "--key" => {
                let (Some(s), Some(at)) = (args.get(i + 1), num(2)) else {
                    continue;
                };
                if t < at {
                    continue;
                }
                done.push(i);
                let key = match (a.as_str(), s.as_str()) {
                    ("--type", _) => aa_ui::UiKey::Text(s.clone()),
                    (_, "enter") => aa_ui::UiKey::Enter,
                    (_, "escape") => aa_ui::UiKey::Escape,
                    (_, "backspace") => aa_ui::UiKey::Backspace,
                    (_, "delete") => aa_ui::UiKey::Delete,
                    (_, "tab") => aa_ui::UiKey::Tab,
                    (_, "left") => aa_ui::UiKey::Left,
                    (_, "right") => aa_ui::UiKey::Right,
                    (_, "home") => aa_ui::UiKey::Home,
                    (_, "end") => aa_ui::UiKey::End,
                    _ => continue,
                };
                if !rt.key(&key) {
                    if key == aa_ui::UiKey::Escape {
                        match rt.escape() {
                            Ok(e) => info!("fake: escape -> {e:?}"),
                            Err(e) => warn!("fake: escape: {e}"),
                        }
                    } else {
                        warn!("fake: {a} {s}: no widget has the keyboard focus");
                    }
                }
                info!("fake: {a} {s} -> focus {:?}", rt.focus());
            }
            // `--open ACTION T`: a key binding action (`toggle_bag`, `toggle_character`,
            // `toggle_spellbook`, `toggle_quest`, `toggle_worldmap`, `open_config`) or a content
            // constant (`UIC_BAG`).
            "--open" => {
                let (Some(s), Some(at)) = (args.get(i + 1), num(2)) else {
                    continue;
                };
                if t < at {
                    continue;
                }
                done.push(i);
                let r = if s.starts_with("UIC_") {
                    match rt.content_id(s) {
                        Some(c) => rt.toggle_content(c).map(|_| true),
                        None => Ok(false),
                    }
                } else {
                    rt.hotkey(s)
                };
                match r {
                    Ok(true) => info!("fake: --open {s}"),
                    Ok(false) => warn!("fake: --open {s}: not a UI window"),
                    Err(e) => warn!("fake: --open {s}: {e}"),
                }
            }
            _ => {}
        }
    }
}

/// The in-game fakes (`fake_ingame.lua`): the player, abilities, action bars, chat, zone ...
/// of `reference/ingame/spawn`. Run after [`install_with`], before the world stage scripts.
#[allow(dead_code)]
pub fn install_ingame(rt: &mut UiRuntime) {
    // `--cooldown`: an action bar skill cooling down (the spawn capture has none).
    if std::env::args().any(|a| a == "--cooldown") {
        let _ = rt.exec("@--cooldown", "FAKE_COOLDOWN_DEMO = true");
    }
    // The HUD first, then the game data of the windows players open (one file per window).
    for (name, code) in [
        ("fake_ingame.lua", include_str!("fake_ingame.lua")),
        ("fake_bag.lua", include_str!("fake_bag.lua")),
        ("fake_character.lua", include_str!("fake_character.lua")),
        ("fake_skill.lua", include_str!("fake_skill.lua")),
        ("fake_quest.lua", include_str!("fake_quest.lua")),
        ("fake_worldmap.lua", include_str!("fake_worldmap.lua")),
        ("fake_options.lua", include_str!("fake_options.lua")),
        ("fake_target.lua", include_str!("fake_target.lua")),
    ] {
        if let Err(e) = rt.exec(&format!("@{name}"), code) {
            eprintln!("fake: {name}: {e}");
        }
    }
}

/// Key bindings of the examples, the client's defaults: I / B bag, C character, K skills,
/// L quests, M world map, Escape closes the topmost window or opens the system menu.
#[allow(dead_code)]
pub fn hotkeys(
    rt: Option<NonSend<UiRuntime>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut had_focus: Local<bool>,
) {
    let Some(rt) = rt else { return };
    let focused = std::mem::replace(&mut *had_focus, rt.focus().is_some());
    if focused || rt.focus().is_some() {
        return;
    }
    for (key, action) in [
        (KeyCode::KeyI, "toggle_bag"),
        (KeyCode::KeyB, "toggle_bag"),
        (KeyCode::KeyC, "toggle_character"),
        (KeyCode::KeyK, "toggle_spellbook"),
        (KeyCode::KeyL, "toggle_quest"),
        (KeyCode::KeyM, "toggle_worldmap"),
    ] {
        if keys.just_pressed(key)
            && let Err(e) = rt.hotkey(action)
        {
            warn!("fake: {action}: {e}");
        }
    }
    if keys.just_pressed(KeyCode::Escape)
        && let Err(e) = rt.escape()
    {
        warn!("fake: escape: {e}");
    }
}

/// What the client raises when the player has entered the world and the loading screen
/// closed (`FAKE.enter_world`, defined by `fake_ingame.lua`).
#[allow(dead_code)]
pub fn enter_world(rt: &mut UiRuntime) {
    if let Err(e) = rt.exec(
        "@enter_world",
        "if FAKE.enter_world then FAKE.enter_world() end",
    ) {
        eprintln!("fake: enter_world: {e}");
    }
}

/// Log script errors and the stubbed natives the screen called; `--coverage` also prints the
/// per-class native coverage as a markdown table.
pub fn report(ui: &UiRuntime, screen: &str) {
    if std::env::args().any(|a| a == "--coverage")
        && let Ok(cov) = aa_ui::lua::coverage(&ui.lua)
    {
        println!("| class | native | stub | listed |");
        println!("|---|---|---|---|");
        for (class, n, s) in cov {
            println!("| `{class}` | {n} | {s} | {} |", n + s);
        }
    }
    let stubs = ui.stub_calls();
    info!(
        "{screen}: {} script errors, {} stubbed natives called",
        ui.errors.len(),
        stubs.len()
    );
    for (name, n) in stubs.iter().take(80) {
        info!("  stub {name} x{n}");
    }
}
