-- Fake game data for the system menu (Escape: OPEN_CONFIG -> systemConfigFrame) and the option
-- window it opens (option/*.lua), plus the sound calls of the in-game windows.

-- Sounds: PlayUISound returns a sound id the scripts later stop (tutorial OnHide compares it
-- with 0).
-- An app with the AAUI sound hooks (AAUI.PlaySound, AAUI.StopSound) gets the toolkit's natives, which call them.
local soundSeq = 0
local toolkitPlay, toolkitStop = X2Sound.PlayUISound, X2Sound.StopSound
X2Sound.PlayUISound = function(self, name, duplicable)
  if AAUI.PlaySound then return toolkitPlay(self, name, duplicable) end
  soundSeq = soundSeq + 1
  return soundSeq
end
X2Sound.StopSound = function(self, id, mode)
  if AAUI.StopSound then return toolkitStop(self, id, mode) end
end

-- System menu: not in a client-driven instance (the "skip" button stays hidden).
X2.IsInClientDrivenZone = function(self) return false end
X2.IsEnableSkipClientDriven = function(self) return false end

-- Option window (option/screen_option.lua, screen_advanced_option.lua, interface_option.lua).
-- The graphics preset the client detected for this machine (1 low .. 5 very high).
X2Option.GetNextSysSpecFullValue = function(self) return 4 end
local resolutions = { { 1280, 720 }, { 1600, 900 }, { 1920, 1080 }, { 2560, 1440 } }
X2Option.GetResolutionCount = function(self) return #resolutions end
X2Option.GetResolution = function(self, i)
  local r = resolutions[i] or resolutions[3]
  return r[1], r[2], 32
end
-- Multisample formats the device supports: { samples, quality, txaa }.
X2Option.EnumAAFormats = function(self)
  return { { samples = 2, quality = 0, txaa = false }, { samples = 4, quality = 0, txaa = false } }
end
X2Option.GetModifiedRestartOption = function(self) return false end
X2Option.GetMinxMaxOfMouseSensitivity = function(self) return 0.1, 3 end
X2Option.Save = function(self) end
X2Option.Reset = function(self) end
X2Option.RemoveModifiedOption = function(self) end

-- Game info page: what the character carries on the back, in display order.
X2Player.GetBackCarryingOrderKeys = function(self) return { "glider", "weapon", "shield", "cape" } end
X2Player.BackCarryingOrders = function(self, ...) end
X2Player.HasSlaveUnit = function(self) return false end
X2Player.PlayerInCombat = function(self) return false end
X2Player.ShowBackHoldable = function(self) return true end
X2Player.ShowBackPackWithCosplay = function(self) return true end
X2Player.ShowCosplay = function(self) return true end
X2Unit.ShowHelmet = function(self) return true end
X2Nation.IsIndependenceFeature = function(self) return false end
X2Input.IsAltKeyDown = function(self) return false end
X2Input.IsControlKeyDown = function(self) return false end
X2Input.IsShiftKeyDown = function(self) return false end

-- Key binding page: every action can be rebound; only the status-sensitive shortcut slots (mode action bar)
-- take an override binding (their buttons have the blue OPTION_KEY_OVERRIDABLE skin, the others the plain one;
-- reference/ingame/options_keybinds). Buttons show the bound key (X2Hotkey:GetOptionBinding below) or nothing.
X2Hotkey.IsValidActionName = function(self, action) return true end
X2Hotkey.IsOverridableAction = function(self, action) return action == "mode_action_bar_button" end
X2Hotkey.GetOptionBindingButton = function(self, action, index) return nil end
X2Hotkey.InitOptionHotKey = function(self) end
X2Hotkey.EnableHotkey = function(self, on) end
X2Hotkey.SaveHotKey = function(self) end

-- Native option values the option pages read (the client's defaults); others read 0.
do
  local defaults = {
    OPTION_ITEM_GAMMA = 1.0,
  }
  local orig = X2Option.GetOptionItemValue
  X2Option.GetOptionItemValue = function(self, id)
    for name, v in pairs(defaults) do
      if _G[name] ~= nil and _G[name] == id then
        local cur = orig(self, id)
        if cur == nil or cur == 0 then return v end
        return cur
      end
    end
    return orig(self, id)
  end
end

-- The client's default key bindings, as the key binding page shows them
-- (X2Hotkey:GetOptionBinding(action, manager, ...): the primary manager's key, none secondary; actions with a
-- count (mode_action_bar_button 1..10, team_target 1..4, ...) pass the index as the last argument, 0 otherwise).
do
  local primary = {
    moveforward = "W", moveback = "S", moveleft = "A", moveright = "D", jump = "SPACE",
    turnleft = "Q", turnright = "E", autorun = "NUMLOCK", toggle_walk = "NUMPADDIVIDE",
    do_interaction_1 = "F", do_interaction_2 = "G", do_interaction_3 = "H", do_interaction_4 = "J",
    round_target = "F1",
    cycle_hostile_forward = "TAB", cycle_hostile_backward = "SHIFT-TAB",
    cycle_friendly_forward = "CTRL-TAB", cycle_friendly_backward = "CTRL-SHIFT-TAB",
    toggle_force_attack = "CTRL-F",
    screenshotmode = "SHIFT-PRINT", toggle_nametag = "SHIFT-Z",
    zoom_in = "WHEELUP", zoom_out = "WHEELDOWN",
    toggle_character = "C", toggle_bag = "I", toggle_quest = "L", toggle_spellbook = "K",
    toggle_worldmap = "M", toggle_achievement = "Y", open_config = "ESCAPE",
  }
  local indexed = {
    mode_action_bar_button = { "R", "T", "Y", "U" },
    team_target = { "F2", "F3", "F4", "F5" },
  }
  local SECOND_HOTKEY_MANAGER = 2 -- a local of option/key_binding_option.lua
  X2Hotkey.GetOptionBinding = function(self, action, manager, withModifier, index)
    if manager == SECOND_HOTKEY_MANAGER then return nil end
    if indexed[action] then return indexed[action][index or 0] end
    return primary[action]
  end
end
