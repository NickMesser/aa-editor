# aa-ui-editor: editor for the client's Lua UI

`crates/aa-ui-editor` shows a real screen of the ArcheAge 1.2 client, built from the original UI scripts by `aa-ui`
(the X2 widget toolkit of the remake), and puts editor panels over it. You can pick a widget on screen, see where the
scripts created and laid it out, change it live, and write the change back into the script. You can also edit any
script as Lua and compile it back to the client's bytecode.

```text
cargo run -p aa-ui-editor                                        # the in-game HUD
cargo run -p aa-ui-editor -- --screen world --open toggle_bag --pick
cargo run -p aa-ui-editor -- --screen character_create --race elf --gender female --step 2
cargo test -p aa-ui-editor
```

## Screens

The top bar picks the screen. Each is built the way the client builds it: the init script, then the screen's
`toc.g` contents in the client's order (`aa_lua::Stage`), then the events the client raises when it enters the
screen. The game data comes from the fake game API of the aa-ui examples (`src/fake/`), copied from AABevy1.2.

| Screen | Scenario |
|---|---|
| Login | `ENTERED_LOGIN`, then the login window is shown (the English client hides it: `loginLocale.loginHide`, it logs in through a launcher). Its widgets build with 0 errors and can be inspected, edited and written into the scripts, but `aa-ui` draws nothing for the login stage yet (0 quads, though the windows are visible at full alpha), so it can't be checked on screen. |
| World select | three servers |
| Character select | with or without the reference character |
| Character create | race, gender and step 1-3, picked the way the reference capture did (button clicks) |
| In game (HUD) | the spawn of the reference capture. "Open window" runs the client's key binding actions (bag, character, skills, quests, world map, achievements, options, …) or toggles any `UIC_*` content. I / B / C / K / L / M / Esc work too. |
| Loading screen | `ENTERED_LOADING` |

**Rebuild (F5)** drops the Lua state and builds the screen again, like the client's stage change. Saved scripts are
picked up then.

## Where scripts come from

For every script the loader (`loader.rs`) takes, in order:

1. the edited copy in the workspace (`--workspace`, default `data/local/ui-editor/sources/x2ui/...lua`);
2. with "run decompiled sources" ticked (the default when there are decompiled sources): the decompiled original
   (`--decompiled`, default `data/local/ui-editor/decompiled`, else `D:/AABevy1.2/data/extract/aa-lua/decompiled`);
3. else the `.alb` bytecode from the pak, or from the pak overlay where saved scripts are compiled to.

Running the decompiled sources makes line numbers in errors and call sites match the text in the script editor. All 961
decompiled scripts compile and load (`all_decompiled_scripts_compile`), and every screen builds from them with 0 load
errors. `Script.ReloadScript` / `LoadScript` (the `*_view.lua` files) go through the same loader.

Decompiled sources come from unluac (`crates/aa-lua/scripts/decompile.sh` in AABevy1.2). Without them, the script
editor decompiles a script on demand when `--unluac <unluac.jar>` is given and Java is on the PATH (cached in
`data/local/ui-editor/decompiled-cache`). Without either, it shows a read-only disassembly.

## Widgets panel and picking

* **Widget tree**: the top-level widgets and their children (tick "hidden" and "drawables" to see those too). Find
  searches names, classes and texts.
* **Pick (F2)**: the pointer selects widgets instead of clicking them. Alt+click picks at any time. Hover shows the
  widget under the pointer. The topmost widget that draws something wins, and "Under the pointer" in the inspector
  lists the rest. In pick mode, drag the selection to move it (all its anchor offsets) and drag its bottom-right corner
  to resize it.
* The selection gets a yellow frame with its name and rectangle. Orange lines go from each anchor point to the
  point it is anchored to.
* **F1** hides every panel and shows the screen as the client would.

## Inspector

* Name, class, id, UI layer, the parents (click to go up), and "Find Lua paths": how scripts reach the object
  (`bag.titleBar.closeButton`, `Func^upvalue.x`). It uses the same walk as the reference captures.
* Rectangle and visibility. Live edits:
  * Show and alpha
  * extent
  * anchors (point, target, relative point, x, y; add and remove)
  * text
  * for drawables: texture, colour and coords
* Drawables, children count and the registered handlers.
* **Script**: the lines that created the object (up to four stack frames, e.g.
  `baselib/window_view:220 → baselib/window:90 → inventory/sort_inventory:647 → inventory/sort_bag:472`). It also shows
  the line that last set each property (`anchor:RIGHT AddAnchor inventory/sort_bag:474`). Click one to open it in the
  script editor.
  * **How it is recorded** (`track.rs`): before the scripts run, the native methods of every widget and drawable
    class are wrapped in the class method tables. `Create*` and the layout / look setters record `debug.getinfo` of
    the calling script frames.
* **Changes**: every live edit is kept per property. Three buttons act on them:
  * **Write into script…** rewrites the script line that last set the property. If its arguments are literals they
    are replaced in place, keeping the call's form (`AddAnchor(point, [target], [relPoint], [x, y])`). If they are
    computed, a new call with the literal values goes right after the statement, on the same receiver. With no
    recorded setter, the call goes after the creation line: the variable it assigns, or `parent.name` for a reflected
    `CreateChildWidget`.
    * A preview lists every change first. A line inside a loop or a shared helper applies to every object that runs
      it.
    * "Apply, save and rebuild" compiles and rebuilds at once (`patch.rs`).
  * **Copy as Lua** copies the changes as calls.
  * **Forget** drops them.

## Script editor

* **Browser**: every `.alb` and `toc.g` under `scriptsbin/`. Filter by name, show only the current screen's scripts
  in load order, or only edited ones (●).
* **Editor**: tabs, Lua syntax colouring (widget API calls and `X2*` globals stand out), line numbers, find / next /
  previous / replace all (Ctrl+F), go to line (Ctrl+G), "Check" (compile only), "Undo changes", "Revert to original".
* **Save (Ctrl+S)** does three things:
  * writes the source to the workspace;
  * compiles it to the client's `.alb` layout (Lua 5.1, 4-byte `size_t`, `float` numbers, as the original `luac`
    wrote them; `loader::compile_alb`) in the pak overlay (`--overlay`, default `data/local/editor/overlay`, shared
    with the world editor);
  * rebuilds the screen.

  A syntax error stops the save and marks the line.
* `toc.g` files are saved as text. **New** creates a script and, if ticked, appends it to its folder's `toc.g`, so
  the client loads it.

## Console

* Load errors and runtime errors of the scripts (click "open" to jump to the line), `print` output, and the count
  of unimplemented natives the screen called.
* **Lua prompt**: runs in the screen's Lua state. An expression shows its value (`=bag:GetWidth()`, or plainly
  `bag:GetWidth()`); a statement just runs (`bag:Show(true)`).

## Using the edits in a client

* **The remade client** (AABevy1.2's `aa-client`) reads the overlay with `AA_PAK_OVERLAY=<overlay>`, the same as
  terrain edits.
* **The original client** reads a `game_pak`. Two ways to write the overlay's UI files into it:
  * the window **Windows → Export to game_pak**: the `scriptsbin/` and `ui/` files, with an undo file next to the
    pak;
  * the world editor's Pak export, which writes the whole overlay.

  Export into a copy of the client: the original client folder is refused. The compiled `.alb` load in the remade
  client's Lua (tested). The original client is expected to load them too, since the chunk layout is identical, but
  that has not been tested in the original client.

## Flags

| Flag | Meaning |
|---|---|
| `--screen login\|world_select\|character_select\|character_create\|world\|loading` | the screen (default `world`) |
| `--race`, `--gender`, `--step` | character create scenario |
| `--characters 0` | character select / in game without the reference character |
| `--overlay <dir>` | the pak overlay (saved `.alb` and `toc.g`) |
| `--workspace <dir>` | edited sources |
| `--decompiled <dir>`, `--unluac <jar>`, `--alb` | where sources come from; `--alb` runs the bytecode |
| `--db <compact.sqlite3>` | UI texts (`LocalizeUiText`) from the game database |
| `--script <pak path> [--line N]` | open a script in the editor |
| `--pick` | start in pick mode |
| `--open <action>`, `--lua <code>`, `--select <Lua expression or widget name>` | after the screen is built |
| `--set-extent w,h`, `--move dx,dy`, `--set-text <text>`, `--write-into-script` | checks: edit the selection, write it into its script, save and rebuild |
| `--shot`, `--shot-after`, `--exit-after`, `--size` | aa-dev |

Round-trip check (writes into the given folders only):

```text
aa-ui-editor --overlay <tmp>/overlay --workspace <tmp>/ws --screen world --open toggle_bag --select bag \
  --set-extent 600,460 --move -300,0 --write-into-script --exit-after 14
# sort_bag.lua:474   newbag:AddAnchor("RIGHT", -30, 0)  ->  newbag:AddAnchor("RIGHT", -330, 0)
# sort_inventory.lua  self:SetExtent(510, height)  is computed  ->  a new line  self:SetExtent(600, 460)  after it
aa-ui-editor --overlay <tmp>/overlay --workspace <tmp>/ws --screen world --open toggle_bag --select bag
# the bag opens at 600 x 460, 300 px further left
```

## Known limits

* The game data is fake (the examples' scenario), so windows show what the fakes provide. Screens and windows the
  fakes don't drive (auction, mail, housing …) build but may be empty.
* Translucent panels blend slightly lighter than in the client: egui needs the linear main texture, so the UI blends
  linearly (as in aa-ui's split mode).
* Writing into a script needs the script to have run from its source, which is the default. In `--alb` mode, line
  numbers refer to the original source, which does not exist.
* Edits to widgets that a script creates per row or slot (lists, grids) rewrite the shared line: the preview says
  so.
* Texture files (`ui/*.dds`) are not edited here. Point a drawable at another texture with the inspector or in the
  script.
