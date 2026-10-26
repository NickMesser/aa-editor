-- Fake game data for the target unit frame (unitframe/target.lua): the player targets Maude,
-- the quest giver standing in front of the spawn point in reference/ingame/spawn/real.png
-- (npcs 3597: level 10, Nuia Alliance).
--
--   FAKE.set_target()       -- target Maude: TARGET_CHANGED(unitId, "npc"), the frame shows
--   FAKE.set_target(t)      -- another unit, same fields as FAKE.target_maude
--   FAKE.clear_target()     -- TARGET_CHANGED(nil, nil), the frame hides
--
-- In the example: --lua "FAKE.set_target()" 2. The X2Unit functions below answer for
-- unit == "target" while a target is set and fall back to the HUD fakes (the player) otherwise.

FAKE.target_maude = {
  name = "Maude",
  level = 10,
  hp = 1450, maxHp = 1450,
  mp = 780, maxMp = 780,
  unitId = 30597,
  unitType = "npc",            -- GetUnitTypeString / GetUnitType
  faction = "Nuia Alliance",
  relationship = "friendly",   -- GetCombatRelationshipStr
  distance = 6.4,
  grade = 1,                   -- npc_grade_id: normal
}

FAKE.target = nil

local T = function() return FAKE.target end

local answers = {
  UnitName = function(t) return t.name end,
  UnitLevel = function(t) return t.level end,
  UnitHealth = function(t) return t.hp end,
  UnitMaxHealth = function(t) return t.maxHp end,
  UnitMana = function(t) return t.mp end,
  UnitMaxMana = function(t) return t.maxMp end,
  UnitIsDead = function(t) return t.hp <= 0 end,
  UnitIsOffline = function(t) return false end,
  UnitCombatState = function(t) return false end,
  UnitRace = function(t) return t.race end,
  UnitGender = function(t) return t.gender end,
  GetUnitId = function(t) return t.unitId end,
  GetUnitType = function(t) return t.unitType end,
  GetUnitTypeString = function(t) return t.unitType end,
  GetFactionName = function(t) return t.faction end,
  GetCombatRelationshipStr = function(t) return t.relationship end,
  UnitDistance = function(t) return t.distance end,
  UnitBuffCount = function(t) return 0 end,
  UnitDeBuffCount = function(t) return 0 end,
  UnitBuff = function(t) return nil end,
  UnitDeBuff = function(t) return nil end,
  UnitCastingInfo = function(t) return nil end,
  UnitInGroup = function(t) return false end,
  UnitIsTeamMember = function(t) return false end,
  IsReporter = function(t) return false end,
  IsFirstHitByMeOrMyTeam = function(t) return false end,
  UnitGrade = function(t) return t.grade end,
  UnitNpcGrade = function(t) return t.grade end,
}

for name, answer in pairs(answers) do
  local orig = X2Unit[name]
  X2Unit[name] = function(self, unit, ...)
    if unit == "target" and T() then
      return answer(T(), ...)
    end
    if orig then
      return orig(self, unit, ...)
    end
    return nil
  end
end

-- The player's target, as GetTargetUnitId("player") asks.
do
  local orig = X2Unit.GetTargetUnitId
  X2Unit.GetTargetUnitId = function(self, unit, ...)
    if (unit == nil or unit == "player") and T() then
      return T().unitId
    end
    return orig and orig(self, unit, ...)
  end
end

FAKE.set_target = function(t)
  FAKE.target = t or FAKE.target_maude
  FAKE.fire_now("TARGET_CHANGED", tostring(FAKE.target.unitId), "npc")
end

FAKE.clear_target = function()
  FAKE.target = nil
  FAKE.fire_now("TARGET_CHANGED", nil, nil)
end
