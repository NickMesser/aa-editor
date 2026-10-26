-- Development stand-ins for the client's game API (X2World, X2Unit, ...), so the original
-- login-stage scripts run without a server. The real app registers real objects instead.
-- `FAKE` (set from Rust before this runs) carries the scenario data.

-- RACE_* / GENDER_* are the client's constants (installed by aa-ui).
RACE_WARBORN = RACE_WARBORN or 8

local raceStr = {
  [RACE_NUIAN] = "nuian", [RACE_FAIRY] = "fairy", [RACE_DWARF] = "dwarf", [RACE_ELF] = "elf",
  [RACE_HARIHARAN] = "hariharan", [RACE_FERRE] = "ferre", [RACE_RETURNED] = "returned",
  [RACE_WARBORN] = "warborn",
}
local genderStr = { [GENDER_MALE] = "male", [GENDER_FEMALE] = "female" }

X2Util.GetGameProvider = function(self) return TRION end
X2Locale.GetLocale = function(self) return FAKE.locale end
-- X2Locale:LocalizeUiText comes from aa-ui (UiConfig::ui_texts_db).
X2Unit.GetRaceStr = function(self, race) return raceStr[race] end
X2Unit.GetGenderStr = function(self, gender) return genderStr[gender] end
X2Debug.GetDevMode = function(self) return false end
X2.GetRaceCongestions = function(self) return { 0, 0, 0, 0 } end
X2.GetFeatureset = function(self) return true end
X2.GetAddonInfos = function(self) return {} end

-- World select -----------------------------------------------------------------------------
X2World.GetWorldInfo = function(self)
  local out = {}
  for i, w in ipairs(FAKE.worlds) do
    local rc = {}
    for _, r in pairs(raceStr) do rc[r] = w.raceCongestion or 0 end
    out[i] = { id = w.id, name = w.name, available = w.available, congestion = w.congestion,
               raceCongestions = rc }
  end
  return out
end
X2World.GetWorldCharactersCount = function(self, id)
  for _, w in ipairs(FAKE.worlds) do
    if w.id == id then return w.characters end
  end
  return 0
end
X2World.IsPreSelectCharacterPeriod = function(self) return false end
X2World.GetTotalCharactersCount = function(self) return 1 end
X2World.GetCurrentTotalCharactersCount = function(self) return 1 end
X2World.GetCurrentTotalCharactersLimit = function(self) return 6 end
X2World.GetTotalCharactersDefaultLimit = function(self) return 6 end
X2World.GetTotalCharactersExpandableLimit = function(self) return 0 end
X2World.GetZoneCount = function(self) return 0 end
X2World.EnterWorld = function(self, worldId, zoneId)
  UIParent:LogAlways(string.format("fake: EnterWorld(%s, %s)", tostring(worldId), tostring(zoneId)))
end
X2World.RequestWorldListRefresh = function(self) UIParent:LogAlways("fake: RequestWorldListRefresh") end
-- The last played world: the first one with characters (its name).
X2LoginCharacter.GetLastPlayedWorldInfo = function(self)
  for _, w in ipairs(FAKE.worlds) do
    if w.characters > 0 then return w.name end
  end
  return nil
end
X2LoginCharacter.GetWorldQueueExpectedTime = function(self) return 120 end
X2LoginCharacter.GetWorldQueuePosition = function(self) return 0 end
X2LoginCharacter.UsePremiumEntrance = function(self) return false end

-- Options: the screen resolution is the window's; script options are unset (nil), so the
-- scripts use their defaults like on a fresh install.
X2Option.GetOptionItemValue = function(self, id)
  if id == OPTION_ITEM_SCREEN_RESOLUTION_WIDTH then return UIParent:GetScreenWidth() * UIParent:GetUIScale() end
  if id == OPTION_ITEM_SCREEN_RESOLUTION_HEIGHT then return UIParent:GetScreenHeight() * UIParent:GetUIScale() end
  return nil
end
X2Option.GetOptionItemValueByName = function(self, name) return nil end

Console.GetAttribute = function(self, name) return nil end
Console.ExecuteString = function(self, cmd) UIParent:LogAlways("fake: console " .. tostring(cmd)) end

-- Character select ---------------------------------------------------------------------------
local function ch(i) return FAKE.characters[i] end
X2World.GetCurrentWorldName = function(self) return FAKE.worlds[1] and FAKE.worlds[1].name or "" end
X2World.GetExpandedCharacterCount = function(self) return 0 end
X2World.LeaveWorld = function(self, how) UIParent:LogAlways("fake: LeaveWorld") return true end
X2LoginCharacter.GetNumLoginCharacters = function(self) return #FAKE.characters end
X2LoginCharacter.GetLastPlayedCharacterIndex = function(self) return #FAKE.characters > 0 and 1 or nil end
X2LoginCharacter.GetLpManagedCharacterIdx = function(self) return 0 end
X2LoginCharacter.IsUniverseLp = function(self) return true end
X2LoginCharacter.GetLoginCharacterName = function(self, i) return ch(i) and ch(i).name end
X2LoginCharacter.GetLoginCharacterLevel = function(self, i) return ch(i) and ch(i).level end
X2LoginCharacter.GetLoginCharacterAbilities = function(self, i)
  local c = ch(i)
  return c and { c.ability, ABILITY_MAX, ABILITY_MAX } or {}
end
X2LoginCharacter.GetLoginCharacterRace = function(self, i) return ch(i) and ch(i).race end
X2LoginCharacter.GetLoginCharacterGender = function(self, i) return ch(i) and ch(i).gender end
X2LoginCharacter.GetLoginCharacterZone = function(self, i) return ch(i) and ch(i).zone end
X2LoginCharacter.GetLoginCharacterFaction = function(self, i) return 1 end
X2LoginCharacter.GetLoginCharacterFactionName = function(self, i) return ch(i) and ch(i).faction end
X2LoginCharacter.GetLoginCharacterMoney = function(self, i) return "0" end
X2LoginCharacter.GetLoginCharacterLaborPower = function(self, i) return 85 end
X2LoginCharacter.GetLoginCharacterMaxLaborPower = function(self, i) return 2000 end
X2LoginCharacter.GetLoginCharacterBmPoint = function(self, i) return 0 end
X2LoginCharacter.GetLoginCharacterPremiumGrade = function(self, i) return 0 end
X2LoginCharacter.GetLoginCharacterInfoHouse = function(self, i, h) return {} end
X2LoginCharacter.GetLoginCharacterInfo = function(self, i)
  local c = ch(i)
  if c == nil then return nil end
  return { validCharPos = true, inDirtyZone = false, posX = c.x or 15578, posY = c.y or 15382, houseCount = 0 }
end
X2LoginCharacter.IsDeleteRequestedCharacter = function(self, i) return false end
X2LoginCharacter.IsTransferRequestedCharacter = function(self, i) return false end
X2LoginCharacter.IsInEnableStartingLocation = function(self, i) return true end
X2LoginCharacter.CanShowClientDrivenSkipDialog = function(self, i) return false end
X2LoginCharacter.GetCharacterDeleteWaitingTime = function(self, i) return 0 end
X2LoginCharacter.IsInPremiumQueue = function(self) return false end
X2LoginCharacter.GetWorldNormalQueueLength = function(self) return 0 end
X2LoginCharacter.GetWorldPremiumQueueLength = function(self) return 0 end
X2LoginCharacter.IsRecustomizing = function(self) return false end
X2LoginCharacter.RequestCharacterListRefresh = function(self, force)
  FAKE.fire_later("LOGIN_CHARACTER_UPDATED", "refreshed")
end
X2LoginCharacter.SelectCharacter = function(self, i, skip)
  UIParent:LogAlways("fake: SelectCharacter(" .. tostring(i) .. ")")
  return true
end
X2LoginCharacter.CreateCharacter = function(self) UIParent:LogAlways("fake: CreateCharacter") end

-- Character create -------------------------------------------------------------------------
X2Ability.GetRaceBuffCount = function(self, race, gender) return 0 end
-- The reference capture shows "Enter 2-26 characters." (same limits for local and English names).
X2Util.GetNamePolicyInfo = function(self, kind)
  return { local_min = 2, local_max = 26, eng_min = 2, eng_max = 26 }
end
X2LoginCharacter.GetEditUnitRace = function(self) return FAKE.edit_race or "nuian" end
X2LoginCharacter.GetEditUnitGender = function(self) return FAKE.edit_gender or "male" end
X2LoginCharacter.ShowPreviewRaceGender = function(self, race, gender)
  FAKE.edit_race, FAKE.edit_gender = race, gender
end
-- Step changes: the client hides one step's window and shows the next.
X2LoginCharacter.EndCharacterCreate = function(self, gotoNext)
  if gotoNext then
    FAKE.fire_now("SHOW_CHARACTER_CREATE_WINDOW", false)
    FAKE.fire_now("SHOW_CHARACTER_CUSTOMIZE_WINDOW", true)
  else
    UIParent:LogAlways("fake: back to character select")
  end
end
X2LoginCharacter.EndCharacterCustomize = function(self, gotoNext)
  FAKE.fire_now("SHOW_CHARACTER_CUSTOMIZE_WINDOW", false)
  if gotoNext then
    FAKE.fire_now("SHOW_CHARACTER_ABILITY_WINDOW", true)
  else
    FAKE.fire_now("SHOW_CHARACTER_CREATE_WINDOW", true)
  end
end
X2LoginCharacter.EndCharacterAbility = function(self, gotoNext)
  if not gotoNext then
    FAKE.fire_now("SHOW_CHARACTER_ABILITY_WINDOW", false)
    FAKE.fire_now("SHOW_CHARACTER_CUSTOMIZE_WINDOW", true)
  end
end

-- Character create, step 2 (appearance) -----------------------------------------------------------
-- The client's feature flags the customizing screens read (as in the reference capture):
-- `beautyshopBypass` -> the "Save / Load Appearance" window (custom_saveload.alb) and no old
-- "Temp. Save" window.
X2Player.GetFeatureSet = function(self)
  return { dwarfWarborn = false, customsaveload = true, beautyshopBypass = true }
end

local raceNum, genderNum = {}, {}
for k, v in pairs(raceStr) do raceNum[v] = k end
for k, v in pairs(genderStr) do genderNum[v] = k end
-- The race / gender being edited, as RACE_* / GENDER_* numbers (set by ShowPreviewRaceGender).
local function editRG()
  return raceNum[FAKE.edit_race or "nuian"], genderNum[FAKE.edit_gender or "male"]
end

-- Number of selectable entries per style: the icon sheets the scripts register in
-- cosmetic_list.lua (AddCustomizingStyle) have exactly one cell per entry.
local function styleCount(style)
  local race, gender = editRG()
  return GetCustomizingStyleCount(race, gender, style) or 0
end
X2Customizer.GetNumCustomHair = function(self) return styleCount(HAIR_TYPE) end
X2Customizer.GetNumCustomHairColor = function(self) return styleCount(HAIR_COLOR) end
X2Customizer.GetNumCustomizingEyebrow = function(self) return styleCount(EYEBROW_TYPE) end
X2Customizer.GetNumCustomizingMakeUp = function(self) return styleCount(EYE_MAKEUP) end
-- "Deco" is the cheek (female) or mustache (male) style, whichever the race / gender has.
X2Customizer.GetNumCustomizingDeco = function(self)
  return math.max(styleCount(CHEEK_TYPE), styleCount(MUSTACHE_TYPE))
end
X2Customizer.GetNumCustomizingFaceDiffuse = function(self) return styleCount(SKIN_TYPE) end
X2Customizer.GetNumCustomSkinColor = function(self) return styleCount(SKIN_COLOR) end
X2Customizer.GetNumCustomizingFaceNormal = function(self) return styleCount(WRINKLE_TYPE) end
X2Customizer.GetNumCustomizingTattoo = function(self) return styleCount(TATTOO_TYPE) end
X2Customizer.GetNumCustomizingScar = function(self) return styleCount(SCAR_TYPE) end
-- Premade faces / eyes / nose / mouth / head: 9 per kind and race (custom_face_presets).
X2Customizer.GetCustomPresetCount = function(self, kind) return 9 end
X2Customizer.IsCosmeticFix = function(self) return FAKE.cosmetic_fix or false end
X2Customizer.FixCosmetic = function(self, fixed) FAKE.cosmetic_fix = fixed and true or false end
-- Hair icon of the item list: ui/icon/customizing_<race>_<m|f>/hair_NNN.dds (the client ships
-- one per hairstyle; the folder says "fere" for ferre).
X2Customizer.GetCustomHairIcon = function(self, index)
  local race = FAKE.edit_race or "nuian"
  if race == "ferre" then race = "fere" end
  local g = (FAKE.edit_gender == "female") and "f" or "m"
  return string.format("ui/icon/customizing_%s_%s/hair_%03d.dds", race, g, index or 1)
end
-- Saved looks (the "Save / Load Appearance" window): none yet.
X2Customizer.GetSavedCustomDataCount = function(self) return 0 end

-- Hair style / hair colour a new character starts with, per race and gender.
local defaultHair = {
  nuian = { male = { 10, 6 }, female = { 1, 33 } },
  elf = { male = { 1, 6 }, female = { 1, 33 } },
  ferre = { male = { 1, 5 }, female = { 6, 16 } },
  hariharan = { male = { 10, 30 }, female = { 1, 30 } },
}

-- The unit being customized (the 3D preview character): holds the current style selection.
-- The scripts take its getters/setters as fields (`unit.GetCustomHair`) and call them with the unit
-- as the first argument, so they are plain functions over `u`. Style numbers (HAIR_TYPE, ...) are
-- script globals, hence the unit is built when the screen asks for it.
local function newCustomUnit()
  local race, gender = editRG()
  local u = { race = race, gender = gender, sel = {}, weight = {}, rgb = {}, face = {}, smile = false,
              scar = { 0, 0, 0, 1 } }
  -- Defaults of a new character: hair style / hair colour as the reference captures show them
  -- highlighted (their page labels are "2 / 3" and "1 / 7" for a Nuian male, ...).
  local look = defaultHair[raceStr[race]][genderStr[gender]]
  u.sel[HAIR_TYPE], u.sel[HAIR_COLOR], u.sel[EYEBROW_TYPE], u.sel[SKIN_COLOR] = look[1], look[2], 1, 1
  u.rgb[EYEBROW_COLOR] = { 0.35, 0.25, 0.15 }
  u.rgb[PUPIL_COLOR] = { 0.30, 0.20, 0.10 }
  u.rgb[LIPS_COLOR] = { 0.75, 0.35, 0.35 }
  u.rgb[MUSTACHE_COLOR] = { 0.35, 0.25, 0.15 }
  -- name -> style the getter/setter works on. "Deco" is the cheek or mustache style.
  local styles = {
    Hair = HAIR_TYPE, HairColor = HAIR_COLOR, Eyebrow = EYEBROW_TYPE, EyebrowColor = EYEBROW_COLOR,
    PupilColor = PUPIL_COLOR, MakeUp = EYE_MAKEUP, LipColor = LIPS_COLOR, Deco = CHEEK_TYPE,
    DecoColor = MUSTACHE_COLOR, FaceDiffuse = SKIN_TYPE, SkinColor = SKIN_COLOR,
    FaceNormal = WRINKLE_TYPE, Tattoo = TATTOO_TYPE, Scar = SCAR_TYPE,
  }
  for name, style in pairs(styles) do
    local colour = u.rgb[style] ~= nil
    -- GetCustomHair(unit) -> index [, weight]; GetCustomEyebrowColor(unit) -> r, g, b
    -- (GetCustomizingDecoColor is the one getter spelled with "Customizing").
    local getter = (name == "DecoColor") and "GetCustomizingDecoColor" or ("GetCustom" .. name)
    u[getter] = function(self)
      if colour then return unpack(u.rgb[style]) end
      return u.sel[style] or 0, u.weight[style]
    end
    -- SetCustomizingHair(unit, index [, ...weight]); SetCustomizingEyebrowColor(unit, r, g, b)
    u["SetCustomizing" .. name] = function(self, a, b, c, d, e)
      if colour then
        u.rgb[style] = { a, b, c }
      else
        u.sel[style] = a or 0
        -- (unit, index, 0, 0, 1, 0) for compound styles, (unit, index, weight) for sliders.
        if name ~= "Scar" then u.weight[style] = b end
      end
    end
  end
  -- Cheek and mustache are two styles of one native "deco" slot.
  u.SetCustomizingDeco = function(self, index, weight)
    u.sel[CHEEK_TYPE], u.weight[CHEEK_TYPE] = index or 0, weight
  end
  u.SetCustomizingScar = function(self, index, x, y, scale, rotate)
    u.sel[SCAR_TYPE] = index or 0
    u.scar = { x or 0, y or 0, rotate or 0, scale or 1 }
  end
  u.GetScarStatus = function(self) return unpack(u.scar) end
  u.ModifyFaceParamValue = function(self, index, value) u.face[index] = value end
  u.GetFaceTargetCurValue = function(self, index) return u.face[index] or 0 end
  u.SetSmile = function(self, smile) u.smile = smile and true or false end
  u.IsSmile = function(self) return u.smile end
  -- Anything else (camera zoom, param slots, ...) is accepted and ignored.
  return setmetatable(u, { __index = function(_, k) return function() end end })
end

X2LoginCharacter.GetCustomizingUnit = function(self)
  local race, gender = editRG()
  if FAKE.unit == nil or FAKE.unit.race ~= race or FAKE.unit.gender ~= gender then
    FAKE.unit = newCustomUnit()
  end
  return FAKE.unit
end
X2LoginCharacter.RemoveUnit = function(self, unit) FAKE.unit = nil end

-- Face morph targets ("Details" window): the morph names of the client's detail sliders, in
-- modifier_list.lua's order (a script local, so repeated here) per part: eyes, nose, mouth, head
-- shape (at most 25 per part: the detail window has that many sliders). Native part numbers are the scripts' EYES..SHAPE minus one; target indices are arbitrary
-- but unique. Value range -100..100.
local faceMorphs = {
  [1] = { "eye_Size", "eye_Length", "eye_Rotate", "eye_Close", "eye_Zposition", "eye_Xposition",
          "eye_Yposition", "eye_SloeDown", "eye_LacrimalRotate", "eye_LacrimalCaruncle",
          "eye_EyelineZcenter", "eye_EyelineZinner", "eye_LowShapeShallow", "eye_DoubleEyelidAngle",
          "eye_DoubleEyelidThickness", "eye_EyeCoverVolume",
          "eye_PupilSize", "eye_PupilRotate", "eye_RotateUp", "eye_BrowridgeDown",
          "eye_SupraorbitalRidge", "eye_BrowRotate", "eye_BrowThickness", "eye_Innerbrow",
          "eye_BridgeLength" },
  [2] = { "nose_Size", "nose_Width", "nose_Height", "nose_Length", "nose_SeptonasalTractus",
          "nose_SeptumZposition", "nose_SeptonasalAxialPosition", "nose_AlarNasi", "nose_AlarVolume",
          "nose_AlarReduction", "nose_AlarHeight", "nose_AlarDepth", "nose_AlarZposition",
          "nose_NasalBoneElevation", "nose_NasalBoneVolume", "nose_NasalBoneWidth",
          "nose_NasalUpperSideY", "nose_NasalUpperZ", "nose_YShape" },
  [3] = { "mouth_Size", "mouth_Length", "mouth_Zposition", "mouth_Yposition", "mouth_LipFoward",
          "mouth_UnderbiteOverbite", "mouth_LipClosedShape", "mouth_LipSideY", "mouth_Deform",
          "mouth_LipDeform", "mouth_LipUpperShapeFlow", "mouth_UpperLipThickness",
          "mouth_LipUpperVol", "mouth_LipLowerShapeFlow", "mouth_LowerLipThickness",
          "mouth_LipLowerVol", "mouth_LipLowerWidth", "mouth_LipShapeLowY", "mouth_LipOpen",
          "mouth_LipShapeUpperWidth", "mouth_ToothWidth", "mouth_ToothZposition", "mouth_ToothSize" },
  [4] = { "FS_Width", "FS_ForeheadWidth", "FS_JawSize", "FS_JawShape", "FS_JawShapeZ",
          "FS_JawZposition", "FS_JawShapeFlow", "FS_JawWidth", "FS_ChinVolume", "FS_ChinYposition",
          "FS_SubmaxillaYposition", "FS_ChinWidth", "FS_JawNeckVol", "FS_CheekboneVolume",
          "FS_CheekboneZ", "FS_CheekMeat", "FS_Front", "FS_EarSize", "elf_ElfEar", "FS_EarRotateX",
          "elf_EarRotate", "FS_NewPreset" },
}
local faceTargetName = {}
local faceTargetIndex = {}
do
  local index = 0
  for part, names in ipairs(faceMorphs) do
    faceTargetIndex[part] = {}
    for _, name in ipairs(names) do
      index = index + 1
      faceTargetName[index] = name
      faceTargetIndex[part][#faceTargetIndex[part] + 1] = index
    end
  end
end
X2Customizer.GetNumFaceTargets = function(self, part) return #(faceTargetIndex[part] or {}) end
X2Customizer.GetFaceTargetIndex = function(self, part, order)
  return (faceTargetIndex[part] or {})[order + 1]
end
X2Customizer.GetFaceTargetName = function(self, index) return faceTargetName[index] end
X2Customizer.GetFaceTargetMinValue = function(self, index) return -100 end
X2Customizer.GetFaceTargetMaxValue = function(self, index) return 100 end

-- Character create, step 3 (skillset + name) -------------------------------------------------------
-- The three starter skills per skillset (ABILITY_SKILL in loginstage/common.lua) as the client's
-- skill tooltip tables: { name, icon file in ui/icon, mana, cooldown ms, casting ms, range m },
-- from the game database.
local skillInfo = {
  [18132] = { "Triple Slash", "icon_skill_fight02", 30, 0, 0, 4 },
  [10455] = { "Frenzy", "icon_skill_fight10", 0, 90000, 0, 0 },
  [13282] = { "Whirlwind Slash", "icon_skill_fight09", 0, 12000, 0, 4 },
  [10752] = { "Flamebolt", "icon_skill_magic01", 16, 0, 1000, 20 },
  [11967] = { "Chain Lightning", "icon_skill_magic19", 0, 36000, 0, 20 },
  [10664] = { "Meteor Strike", "icon_skill_magic13", 0, 45000, 5000, 25 },
  [13564] = { "Piercing Shot", "icon_skill_vocation01", 0, 12000, 0, 20 },
  [13281] = { "Missile Rain", "icon_skill_wild10", 0, 36000, 2000, 50 },
  [10708] = { "Intensity", "icon_skill_wild09", 0, 60000, 0, 0 },
  [10534] = { "Antithesis", "icon_skill_love01", 0, 5000, 2000, 25 },
  [10546] = { "Revive", "icon_skill_love06", 0, 0, 7000, 25 },
  [10721] = { "Twilight", "icon_skill_love13", 0, 120000, 0, 3 },
  [10201] = { "Crippling Mire", "icon_skill_death11", 0, 18000, 0, 20 },
  [11441] = { "Absorb Lifeforce", "icon_skill_death05", 0, 36000, 0, 20 },
  [10434] = { "Summon Wraith", "icon_skill_death13", 0, 60000, 0, 3 },
  [10481] = { "Toxic Shot", "icon_skill_wild01", 0, 9000, 0, 20 },
  [12029] = { "Wallop", "icon_skill_vocation27", 0, 12000, 0, 4 },
  [12075] = { "Shadow Step", "icon_skill_vocation16", 0, 30000, 0, 20 },
}
X2LoginCharacter.GetSkillTooltip = function(self, skillId, level)
  local s = skillInfo[skillId] or { "Skill " .. tostring(skillId), "icon_skill_fight01", 0, 0, 0, 0 }
  -- Tooltip `ability` is the skillset's key in tooltip.abilityColor / locale.common.abilityCategory;
  -- the numbers FIGHT=1, DEATH=5, WILD=6, MAGIC=7, VOCATION=8, LOVE=10 index this list.
  local abilityKeys = { "fight", "illusion", "adamant", "will", "death", "wild", "magic", "vocation",
                        "romance", "love" }
  local ability = "fight"
  for _, a in ipairs(ABILITY_TYPE) do
    for _, id in ipairs(ABILITY_SKILL[a]) do if id == skillId then ability = abilityKeys[a] end end
  end
  return { tipType = "skill", name = s[1], path = "ui/icon/" .. s[2] .. ".dds", ability = ability,
           skillLevel = 0, description = s[1], mana = s[3], cooldown = s[4], casting = s[5],
           channeling = 0, minRange = 0, maxRange = s[6], targetAreaRadius = 0 }
end

-- Name field: letters only, 2..26 of them (the policy of GetNamePolicyInfo).
X2Util.CheckAvailableName = function(self, text, composing)
  text = text or ""
  if composing then return text end
  return (string.gsub(text, "[^%a]", ""))
end
X2Util.IsValidName = function(self, text, kind)
  text = text or ""
  return #text >= 2 and #text <= 26 and string.find(text, "^%a+$") ~= nil
end
