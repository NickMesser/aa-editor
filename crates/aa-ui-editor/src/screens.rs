//! The screens the editor can show, each built like the client builds it, with the fake game API of the aa-ui
//! examples (`fake/`).

use std::cell::RefCell;
use std::rc::Rc;

use aa_ui::{Stage, UiRuntime};
use bevy::prelude::*;

use crate::fake;
use crate::loader::{self, LoadLog, Sources};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Login,
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    World,
    Loading,
}

impl Screen {
    pub const ALL: [Screen; 6] =
        [Screen::Login, Screen::WorldSelect, Screen::CharacterSelect, Screen::CharacterCreate, Screen::World, Screen::Loading];

    pub fn label(self) -> &'static str {
        match self {
            Screen::Login => "Login",
            Screen::WorldSelect => "World select",
            Screen::CharacterSelect => "Character select",
            Screen::CharacterCreate => "Character create",
            Screen::World => "In game (HUD)",
            Screen::Loading => "Loading screen",
        }
    }

    pub fn parse(s: &str) -> Option<Screen> {
        Some(match s {
            "login" => Screen::Login,
            "world_select" => Screen::WorldSelect,
            "character_select" => Screen::CharacterSelect,
            "character_create" => Screen::CharacterCreate,
            "world" | "ingame" | "hud" => Screen::World,
            "loading" => Screen::Loading,
            _ => return None,
        })
    }

    pub fn stage(self) -> Stage {
        match self {
            Screen::Login => Stage::Login,
            Screen::WorldSelect => Stage::WorldSelect,
            Screen::CharacterSelect => Stage::CharacterSelect,
            Screen::CharacterCreate => Stage::CharacterCreate,
            Screen::World => Stage::World,
            Screen::Loading => Stage::Loading,
        }
    }
}

/// What to build: the screen and its scenario.
#[derive(Clone, Debug)]
pub struct ScreenSpec {
    pub screen: Screen,
    /// Character create: race `nuian|elf|ferre|hariharan`, gender `male|female`, step 1..3.
    pub race: String,
    pub gender: String,
    pub step: u32,
    /// Character select / in game: with the reference character (`Refone`) or none.
    pub with_character: bool,
}

impl Default for ScreenSpec {
    fn default() -> Self {
        ScreenSpec { screen: Screen::World, race: "nuian".into(), gender: "male".into(), step: 1, with_character: true }
    }
}

/// The result of a build, read by the editor panels.
#[derive(Default)]
pub struct Built {
    pub log: LoadLog,
    pub ms: f32,
}

thread_local! {
    /// The last build's load log (the setup runs on the main thread, inside the UI plugin).
    pub static LAST: RefCell<Built> = RefCell::default();
}

/// Build `spec` into a fresh runtime.
pub fn build(rt: &mut UiRuntime, spec: &ScreenSpec, sources: &Sources) {
    let t0 = std::time::Instant::now();
    let chars = if spec.with_character { fake::characters_from_args() } else { Vec::new() };
    let worlds = fake::default_worlds();
    fake::install_with(rt, "en_us", &worlds, &chars);
    if matches!(spec.screen, Screen::World | Screen::Login) {
        fake::install_ingame(rt);
    }
    if spec.screen == Screen::Login {
        let _ = rt.exec("@login_fixture_api", "Console.GetAttribute=function() return '' end");
    }
    if let Err(e) = crate::track::install(&rt.lua) {
        warn!("ui-editor: call-site tracking: {e}");
    }
    let log: Rc<RefCell<LoadLog>> = Rc::default();
    if let Err(e) = loader::install_script_table(&rt.lua, sources, &log) {
        warn!("ui-editor: Script table: {e}");
    }
    let mut files = vec![aa_lua::stage::INIT_SCRIPT.to_string()];
    match spec.screen.stage().scripts() {
        Ok(f) => files.extend(f),
        Err(e) => rt.errors.push((spec.screen.stage().name().into(), format!("{e:#}"))),
    }
    loader::run_files(rt, sources, &files, &log);
    let exec = |rt: &UiRuntime, name: &str, code: &str| {
        if let Err(e) = rt.exec(name, code) {
            warn!("ui-editor: {name}: {e}");
        }
    };
    match spec.screen {
        Screen::Login => {
            rt.fire_event("ENTERED_LOGIN", ());
            // what the client does once the login stage's logo / movie state is over
            exec(rt, "@login_ready", "login.backMain:Show(true)\nlogin.stateHandler:Show(false)\nlogin.backMain.mainWindow:Show(true)\nSetReadyToLogin(true)");
        }
        Screen::WorldSelect => {
            rt.fire_event("ENTERED_WORLD_SELECT", ());
            rt.fire_event("SHOW_SERVER_SELECT_WINDOW", true);
        }
        Screen::CharacterSelect => rt.fire_event("SHOW_CHARACTER_SELECT_WINDOW", true),
        Screen::CharacterCreate => {
            rt.fire_event("SHOW_CHARACTER_CREATE_WINDOW", true);
            let ri = match spec.race.as_str() {
                "elf" => 2,
                "ferre" => 3,
                "hariharan" => 4,
                _ => 1,
            };
            let gi = if spec.gender == "female" { 2 } else { 1 };
            exec(rt, "@select race", &format!("step1.content.raceBtn[{ri}]:OnClick(\"LeftButton\")\nstep1.content.rightPanel.genderBtn[{gi}]:OnClick(\"LeftButton\")"));
            for _ in 1..spec.step {
                exec(rt, "@next step", "characterCreate:OnNextStep()");
            }
        }
        Screen::World => fake::enter_world(rt),
        Screen::Loading => {
            rt.fire_event("ENTERED_LOADING", "");
            rt.fire_event("STILL_LOADING", 0.5);
        }
    }
    aa_ui::lua::flush(&rt.lua);
    let log = std::mem::take(&mut *log.borrow_mut());
    let ms = t0.elapsed().as_secs_f32() * 1000.0;
    info!("ui-editor: built {} in {ms:.0} ms, {} scripts, {} errors", spec.screen.label(), log.ran.len(), rt.errors.len());
    LAST.with(|l| *l.borrow_mut() = Built { log, ms });
}

/// The setup for the UI plugin (first build and reloads).
pub fn setup(spec: ScreenSpec, sources: Sources) -> aa_ui::UiSetup {
    aa_ui::UiSetup::new(move |rt| build(rt, &spec, &sources))
}
