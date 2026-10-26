-- Fake game data for the skill window (K: skill/main_window.lua and its tabs combat, general,
-- emotion, vocation = actability, language). Loaded after fake_ingame.lua: the X2Ability fakes
-- there (one active ability, Battlerage at level 1) are completed or replaced here.
--
-- The scenario is the same level 1 Nuian: Battlerage is his only ability, so the combat tab shows
-- its column plus two "can be activated at level 5 / 10" columns. Skill names, icons and levels
-- are the game database's (D:/aaemu-live/rs-game/Data/compact.sqlite3: skills with ability_id 1,
-- passive_buffs, default_skills by skill_book_category_id, icons; English names from
-- localized_texts); no capture of this window exists, so the layout is what the scripts build.
--
-- Slots of the skill window are `ISLOT_ABILITY_VIEW` slots; the toolkit asks
-- `AAUI.SlotContent(ISLOT_ABILITY_VIEW, idx)` for them. The index is the "kind" constant of the
-- scripts plus the position: ACTIVE_SKILL_n (10, 30, 50) + 1..15 and PASSIVE_SKILL_n (70, 80, 90)
-- + 1..8 for the three ability columns of the combat tab, ATTACK_SKILL (100), GENERAL_SKILL (110),
-- JOB_SKILL (130) and EMOTION_SKILL (140) + 1.. for the general and emotion tabs. The content is
-- a skill ({ type = "skill" }) or a passive buff ({ type = "buff" }) with the skill book state
-- fields documented in slot.rs (learned, canLearn, reqPoints, skillLevel).

AAUI = AAUI or {}

local function icon(file) return "ui/icon/" .. file .. ".dds" end

-- Data from the game database ---------------------------------------------------------------------------
-- Battlerage (ability 1): the active skills with their ability level, and the passive buffs
-- ("continuous" effects) with the skill points they cost.
local FIGHT_SKILLS = {
  { id = 18132, name = "Triple Slash", icon = "icon_skill_fight02", level = 1 },
  { id = 11918, name = "Charge", icon = "icon_skill_fight14", level = 3 },
  { id = 13282, name = "Whirlwind Slash", icon = "icon_skill_fight09", level = 10 },
  { id = 10644, name = "Sunder Earth", icon = "icon_skill_fight04", level = 15 },
  { id = 12039, name = "Lasso", icon = "icon_skill_fight19", level = 20 },
  { id = 18308, name = "Terrifying Roar", icon = "icon_skill_fight26", level = 25 },
  { id = 12034, name = "Bondbreaker", icon = "icon_skill_fight16", level = 30 },
  { id = 12026, name = "Precision Strike", icon = "icon_skill_fight15", level = 35 },
  { id = 10455, name = "Frenzy", icon = "icon_skill_fight10", level = 40 },
  { id = 10377, name = "Battle Focus", icon = "icon_skill_fight08", level = 45 },
  { id = 13315, name = "Tiger Strike", icon = "icon_skill_fight18", level = 50 },
}
local FIGHT_BUFFS = {
  { id = 481, name = "Weapon Maneuvers", icon = "icon_skill_fight03", points = 2 },
  { id = 831, name = "Dual-Wield Proficiency", icon = "icon_skill_fight20", points = 4 },
  { id = 811, name = "Attack Speed Training", icon = "icon_skill_fight17", points = 6 },
  { id = 2610, name = "Deflect and Retaliate", icon = "icon_skill_fight23", points = 7 },
  { id = 7542, name = "Reckless Charge", icon = "icon_skill_fight29", points = 8 },
  { id = 2621, name = "Puncture", icon = "icon_skill_fight24", points = 9 },
  { id = 7544, name = "Weapon Mastery", icon = "icon_skill_fight30", points = 10 },
}
-- default_skills by skill_book_category_id: 2 attack, 3 general, 5 emotions.
local ATTACK = {
  { id = 2, name = "Melee Attack", icon = "icon_skill_basic01" },
  { id = 16064, name = "Shoot Arrow", icon = "icon_skill_basic10" },
  { id = 14495, name = "Kick", icon = "icon_skill_fight22" },
  { id = 20297, name = "Escape", icon = "icon_skill_basic11" },
  { id = 22143, name = "Rider's Escape", icon = "icon_skill_cart05" },
}
local GENERAL = {
  { id = 16287, name = "Dash", icon = "icon_skill_basic09" },
  { id = 10594, name = "Recall", icon = "icon_skill_basic04" },
  { id = 14900, name = "Play Instrument", icon = "icon_skill_basic06" },
  { id = 14902, name = "Drop Trade Pack", icon = "icon_skill_basic08" },
  { id = 14901, name = "Use Glider", icon = "icon_skill_basic07" },
  { id = 14152, name = "Dance: Gotta Get Down", icon = "emotion/icon_emotion_039" },
  { id = 18781, name = "Dance: Pumped Up", icon = "emotion/icon_emotion_047" },
  { id = 20786, name = "Dance: Fiery Tango", icon = "emotion/icon_emotion_048" },
  { id = 27088, name = "Countdown", icon = "icon_skill_basic13" },
}
-- Job / race skills: a few the new character could have (skills table).
local JOB = {
  { id = 17657, name = "Fold Glider", icon = "icon_skill_vehicle03" },
  { id = 21571, name = "Fishing", icon = "icon_skill_fishing_06" },
}
local EMOTION = {
  { id = 14451, name = "/waving", icon = "emotion/icon_emotion_027" },
  { id = 14452, name = "/happy", icon = "emotion/icon_emotion_002" },
  { id = 14453, name = "/sad", icon = "emotion/icon_emotion_029" },
  { id = 14454, name = "/surprise", icon = "emotion/icon_emotion_014" },
  { id = 14455, name = "/gloomy", icon = "emotion/icon_emotion_030" },
  { id = 14456, name = "/bored", icon = "emotion/icon_emotion_038" },
  { id = 14457, name = "/confused", icon = "emotion/icon_emotion_032" },
  { id = 14458, name = "/angry", icon = "emotion/icon_emotion_045" },
  { id = 14459, name = "/sorry", icon = "emotion/icon_emotion_025" },
  { id = 14460, name = "/clap", icon = "emotion/icon_emotion_022" },
  { id = 14461, name = "/salute", icon = "emotion/icon_emotion_007" },
  { id = 14462, name = "/thank", icon = "emotion/icon_emotion_035" },
  { id = 14463, name = "/haha", icon = "emotion/icon_emotion_043" },
  { id = 14464, name = "/scared", icon = "emotion/icon_emotion_006" },
  { id = 14465, name = "/decline", icon = "emotion/icon_emotion_005" },
  { id = 14466, name = "/point", icon = "emotion/icon_emotion_036" },
  { id = 14467, name = "/cheer", icon = "emotion/icon_emotion_034" },
  { id = 14469, name = "/taunt", icon = "emotion/icon_emotion_017" },
  { id = 14470, name = "/blush", icon = "emotion/icon_emotion_003" },
  { id = 14471, name = "/bow", icon = "emotion/icon_emotion_037" },
  { id = 14472, name = "/shh", icon = "emotion/icon_emotion_028" },
  { id = 14473, name = "/kneel", icon = "emotion/icon_emotion_019" },
  { id = 14474, name = "/suffer", icon = "emotion/icon_emotion_008" },
  { id = 14475, name = "/up", icon = "emotion/icon_emotion_033" },
  { id = 14476, name = "/sigh", icon = "emotion/icon_emotion_044" },
  { id = 14477, name = "/stretch", icon = "emotion/icon_emotion_010" },
  { id = 14478, name = "/yawn", icon = "emotion/icon_emotion_042" },
  { id = 14479, name = "/frustrated", icon = "emotion/icon_emotion_015" },
  { id = 14480, name = "/prayer", icon = "emotion/icon_emotion_020" },
  { id = 14482, name = "/nope", icon = "emotion/icon_emotion_016" },
  { id = 14483, name = "/disagree", icon = "emotion/icon_emotion_023" },
  { id = 14484, name = "/callout", icon = "emotion/icon_emotion_031" },
  { id = 14485, name = "/nod", icon = "emotion/icon_emotion_013" },
  { id = 14486, name = "/scissors", icon = "emotion/icon_emotion_004" },
  { id = 14487, name = "/rock", icon = "emotion/icon_emotion_021" },
  { id = 14488, name = "/paper", icon = "emotion/icon_emotion_024" },
  { id = 14489, name = "/cough", icon = "emotion/icon_emotion_011" },
  { id = 14490, name = "/stinks", icon = "emotion/icon_emotion_040" },
  { id = 14491, name = "/retch", icon = "emotion/icon_emotion_041" },
  { id = 14492, name = "/flowers", icon = "emotion/icon_emotion_012" },
  { id = 14493, name = "/love", icon = "emotion/icon_emotion_026" },
  { id = 14494, name = "/charge", icon = "emotion/icon_emotion_018" },
  { id = 13734, name = "/sit", icon = "emotion/icon_emotion_046" },
  { id = 15229, name = "/beg", icon = "emotion/icon_emotion_009" },
  { id = 16941, name = "/rumble", icon = "emotion/icon_emotion_001" },
  { id = 26834, name = "/pose", icon = "emotion/icon_emotion_049" },
}

-- The skills and passive buffs of every ability the fake knows (only Battlerage's lists).
local ABILITY_SKILLS = {
  [ABILITY_FIGHT] = { active = FIGHT_SKILLS, passive = FIGHT_BUFFS },
}
local MAX_ACTIVE, MAX_PASSIVE = 15, 8 -- slots per ability column (tab_combat.lua)

-- Layout constants of the scripts (the engine's ACTIVE_SKILL_n ... EMOTION_SKILL).
local ACTIVE_BASE = { ACTIVE_SKILL_1, ACTIVE_SKILL_2, ACTIVE_SKILL_3 }
local PASSIVE_BASE = { PASSIVE_SKILL_1, PASSIVE_SKILL_2, PASSIVE_SKILL_3 }
local FLAT_GROUPS = {
  { base = ATTACK_SKILL, list = ATTACK },
  { base = GENERAL_SKILL, list = GENERAL },
  { base = JOB_SKILL, list = JOB },
  { base = EMOTION_SKILL, list = EMOTION },
}

-- Demo mode: the skill window only opens when the client is not a demo.
X2Demo.GetDemoOptionCount = function(self) return 0 end

-- The ability view -----------------------------------------------------------------------------------------
-- The three columns of the combat tab show the active abilities in order; a column can also
-- preview an inactive ability (SetAbilityToView). nil = nothing in that column.
local view = { ABILITY_FIGHT }
X2Ability.GetAbilityFromView = function(self, index) return view[index] end
X2Ability.SetAbilityToView = function(self, index, ability)
  if ability == nil or ability >= ABILITY_MAX then
    view[index] = nil
  else
    view[index] = ability
  end
end
X2Ability.ResetAbilityView = function(self)
  view = {}
  for i, a in ipairs(X2Ability:GetActiveAbility()) do view[i] = a.type end
end
X2Ability.IsActiveAbility = function(self, ability)
  for _, a in ipairs(X2Ability:GetActiveAbility()) do
    if a.type == ability then return true end
  end
  return false
end
-- Skill points: the level 1 character has spent the one point (Triple Slash). (total, used)
X2Ability.GetSkillPoint = function(self, ability) return 1, 1 end

-- What the slot index of a view slot refers to: "active" / "passive" (+ the ability and the
-- entry of its list) or "skill" (a flat list).
local function slotEntry(idx)
  for col = 1, 3 do
    local ability = view[col]
    local lists = ability and ABILITY_SKILLS[ability]
    local base = ACTIVE_BASE[col]
    if idx > base and idx <= base + MAX_ACTIVE then
      return "active", lists and lists.active[idx - base], ability
    end
    base = PASSIVE_BASE[col]
    if idx > base and idx <= base + MAX_PASSIVE then
      return "passive", lists and lists.passive[idx - base], ability
    end
  end
  for _, g in ipairs(FLAT_GROUPS) do
    if idx > g.base and idx <= g.base + #g.list then return "skill", g.list[idx - g.base] end
  end
  return nil
end

-- Number of slots of a kind (the scripts show that many, and hide the rest).
X2Ability.GetAbilitySlotCount = function(self, kind)
  for col = 1, 3 do
    local ability = view[col]
    local lists = ability and ABILITY_SKILLS[ability]
    if kind == ACTIVE_BASE[col] then return lists and #lists.active or 0 end
    if kind == PASSIVE_BASE[col] then return lists and #lists.passive or 0 end
  end
  for _, g in ipairs(FLAT_GROUPS) do
    if kind == g.base then return #g.list end
  end
  return 0
end
-- Name of the skill in a view slot (the label next to the icon in the general and emotion tabs).
X2Ability.GetAbilitySlotName = function(self, idx)
  local _, e = slotEntry(idx)
  if e == nil then return nil end
  return e.name
end

-- Slot contents ---------------------------------------------------------------------------------------------
local function viewContent(idx)
  local kind, e, ability = slotEntry(idx)
  if e == nil then return nil end
  if kind == "active" then
    local learned = e.level <= X2Ability:GetAbilityLevel(ability)
    return { type = "skill", id = e.id, icon = icon(e.icon), canUse = learned, learned = learned,
             canLearn = false, skillLevel = 1 }
  elseif kind == "passive" then
    -- A passive buff is learned with skill points (none to spare): dimmed with its cost over it.
    return { type = "buff", id = e.id, icon = icon(e.icon), canUse = false, learned = false,
             canLearn = false, reqPoints = e.points }
  end
  return { type = "skill", id = e.id, icon = icon(e.icon), canUse = true }
end

local prevSlotContent = AAUI.SlotContent
AAUI.SlotContent = function(slotType, slotIdx)
  if slotType == ISLOT_ABILITY_VIEW then return viewContent(slotIdx) end
  if prevSlotContent then return prevSlotContent(slotType, slotIdx) end
  return nil
end

-- AAUI.SkillContent(skillType): slot:EstablishSkill (the skill notifier's icon, link tooltips).
-- Every skill listed above; the action bar skills of fake_ingame.lua keep their own content.
local BY_ID = {}
for _, list in ipairs({ FIGHT_SKILLS, ATTACK, GENERAL, JOB, EMOTION }) do
  for _, s in ipairs(list) do BY_ID[s.id] = s end
end
local prevSkillContent = AAUI.SkillContent
AAUI.SkillContent = function(skillType)
  local prev = prevSkillContent and prevSkillContent(skillType)
  if prev ~= nil then return prev end
  local s = BY_ID[skillType]
  if s == nil then return nil end
  return { type = "skill", id = s.id, icon = icon(s.icon), canUse = true }
end

-- Vocations (actabilities) and languages ------------------------------------------------------------------------
-- Proficiency points of a new character: a few vocations he has already touched (the rest 0).
-- Languages: the Nuian he speaks since birth.
local POINTS = { [3] = 340, [9] = 125, [6] = 60, [21] = 1500 }
local function actabilityInfo(group)
  return { type = group.type, name = group.name, grade = 0, point = POINTS[group.type] or 0, modifyPoint = 0 }
end
-- Everything the skill window lists: the visible vocations plus the languages with a skill page
-- (tab_actability skips languages, tab_language keeps only them).
local function listedActabilities()
  local all = {}
  for type = 1, 34 do
    local g = X2Craft:GetActabilityGroupInfoByGroupType(type)
    if g ~= nil and (g.visible or g.skill_page_visible) then all[#all + 1] = actabilityInfo(g) end
  end
  return all
end
X2Ability.GetAllMyActabilityInfos = function(self) return listedActabilities() end
X2Ability.GetMyActabilityInfo = function(self, type)
  local g = X2Craft:GetActabilityGroupInfoByGroupType(type)
  if g == nil then return nil end
  return actabilityInfo(g)
end
-- The gauge of a proficiency runs to the point limit of its grade.
X2Ability.GetMaxActabilityPoint = function(self, type)
  local info = X2Ability:GetMyActabilityInfo(type)
  local grade = X2Ability:GetGradeInfo(info and info.grade or 0)
  return grade and grade.upLimit or 10000
end
