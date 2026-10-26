-- Fake game data for the worldmap window (see fake_ingame.lua for the pattern).

-- X2Map:GetMapIconText(iconType): the ui_texts key (category MAP_TEXT) naming a map icon type, which the
-- filter panel looks up with locale.map.getSymbolText. The icon types are the M_* numbers of map/common.lua
-- (IncreaseSymbolIndex order); each one's text is keyed by the constant's own name ("M_NPC_STATION").
local MAP_ICON_NAMES = {
  "M_QUEST_TALK_OR_EMPLOYMENT", "M_QUEST_MAIN_GIVES", "M_QUEST_ZONE_GIVES", "M_QUEST_REPEAT",
  "M_QUEST_MAIN_COMPLETES", "M_QUEST_MAIN_PROGRESS", "M_QUEST_ZONE_COMPLETES",
  "M_QUEST_ZONE_PROGRESS", "M_QUEST_LIVELIHOOD_GIVES", "M_QUEST_LIVELIHOOD_COMPLETE",
  "M_QUEST_HUNT_GIVES", "M_QUEST_NOTIFIER", "M_PARTY", "M_RAIDTEAM", "M_OFFLINE_PARTY",
  "M_RAIDTEAM_OWNER", "M_OVER_HEAD_MARK1", "M_OVER_HEAD_MARK2", "M_OVER_HEAD_MARK3",
  "M_OVER_HEAD_MARK4", "M_OVER_HEAD_MARK5", "M_OVER_HEAD_MARK6", "M_OVER_HEAD_MARK7",
  "M_OVER_HEAD_MARK8", "M_OVER_HEAD_MARK9", "M_OVER_HEAD_MARK10", "M_OVER_HEAD_MARK11",
  "M_OVER_HEAD_MARK12", "M_QUEST_LETIT", "M_QUEST_OVER", "M_NPC_STATION", "M_NPC_GRAVEYARD",
  "M_NPC_ABILITY_CHANGER", "M_NPC_STABLER", "M_FRIENDLY", "M_HOSTILE", "M_NPC_STORE_POTION",
  "M_NPC_STORE_CLOTHES", "M_NPC_STORE_FOOD", "M_NPC_STORE_WEAPON", "M_NPC_STORE_DEFENSE",
  "M_NPC_STORE_TREE", "M_NPC_STORE_PLANTS", "M_NPC_STORE_MATERIAL", "M_NPC_STORE_GOODS",
  "M_NPC_STORE_ENLISTING", "M_NPC_STORE_SHIP", "M_NPC_STORE_SIEGE_WEAPON", "M_NPC_STORE_BUILDING",
  "M_NPC_STORE_FURNITURE", "M_NPC_STORE_LIVESTOCK", "M_NPC_STORE_BANK",
  "M_NPC_STORE_AUCTION_HOUSE", "M_NPC_STORE_EXPEDITION", "M_NPC_STORE_PIRATE_EXPEDITION",
  "M_NPC_STORE_TERRITORY", "M_NPC_STORE_TRADE", "M_NPC_STORE_DELPHINAD", "M_NPC_STORE_ROAMER",
  "M_NPC_STORE_BLACKSMITH", "M_NPC_STORE_GLADIATOR", "M_NPC_INSTANCE_TARGET_CORPS1",
  "M_NPC_INSTANCE_TARGET_CORPS2", "M_HOUSING_HOUSE", "M_HOUSING_FARM", "M_HOUSING_FISHFARM",
  "M_MY_CROPS", "M_CORPSE_POS", "M_MY_PING", "M_PORTAL", "M_SHIP", "M_SHIPYARD", "M_SHIP_ENEMY",
  "M_SHIPYARD_ENEMY", "M_COMMON_FARM", "M_TRANSFER_AIRSHIP", "M_TRANSFER_CARRIAGE",
  "M_TRANSFER_LANDSHIP", "M_FORCE_RELEASED_SHIP", "M_FORCE_RELEASED_VEHICLE", "M_FISH_SCHOOL",
  "M_DOODAD_MAIL", "M_DOODAD_INN", "M_DOODAD_LEATHER", "M_DOODAD_CRAFT", "M_DOODAD_METAL",
  "M_DOODAD_MACHINERY", "M_DOODAD_TIMBER", "M_DOODAD_WOODWORK", "M_DOODAD_WEAPON",
  "M_DOODAD_STONE", "M_DOODAD_ARCHIUM", "M_DOODAD_ALCHEMY", "M_DOODAD_COOK", "M_DOODAD_COATOFARMS",
  "M_DOODAD_FABRIC", "M_DOODAD_SPECIAL_PRODUCT", "M_DOODAD_LIGHT_ARMOR", "M_DOODAD_LEATHER_ARMOR",
  "M_DOODAD_HEAVY_ARMOR", "M_DOODAD_PAPER", "M_DOODAD_PRINT", "M_DOODAD_FISH_STAND",
  "M_DOODAD_ART_WORK", "M_DOODAD_PORTAL", "M_DOODAD_PORTAL_ARCHEMALL", "M_DOODAD_PORTAL_DUNGEON",
  "M_HOUSE_NORMAL_HOUSE", "M_HOUSE_SEA_FARM_HOUSE", "M_HOUSE_HIGH_HOUSE", "M_HOUSE_PUMPKIN_HOUSE",
  "M_HOUSE_FARM_HOUSE", "M_HOUSE_BENNY_HOUSE", "M_HOUSE_BUNGALOW_HOUSE", "M_MY_SLAVE", "M_PLAYER",
}
X2Map.GetMapIconText = function(self, iconType)
  return MAP_ICON_NAMES[iconType]
end

-- AAUI.WorldMap(): where the player is for the world map window (M). level / zone: the map the player
-- is on as the scripts register maps (`worldmap:SaveMapData(WMS_* - 1, id, ...)`: 2 zone, 3 city; id = zone
-- group id, 5 = Solzreed Peninsula), player.u / v = place on that map's image (0..1) and angle = heading
-- (radians clockwise from north; same heading as AAUI.RoadMap), maps = the place on the continent (3, west)
-- and world (2) maps. The client computes these from the player's global position (the Nuian spawn of the
-- server's CharTemplates.json, 15578.042 / 15382.122) and each map's rectangle in the game database
-- (zone_groups / world_groups x, y, w, h): u = (x - X) / W, v = 1 - (y - Y) / H; the results put the cursor
-- where reference/ingame/worldmap_zone, _continent and _world have it.
-- Without this hook the world map reads AAUI.RoadMap.
local function map_uv(x, y, X, Y, W, H) return (x - X) / W, 1 - (y - Y) / H end
local PX, PY = 15578.042, 15382.122
local zu, zv = map_uv(PX, PY, 11925, 13266, 4810.244629, 2882)  -- zone_groups 5 w_solzreed
local cu, cv = map_uv(PX, PY, -6758, 2673, 32130, 19308)        -- world_groups 3 west
local wu, wv = map_uv(PX, PY, -14731, 46, 64651, 38865)         -- world_groups 2 main_world
AAUI.WorldMap = function()
  return {
    level = 2, zone = 5,
    player = { u = zu, v = zv, angle = -2.77053833 },
    maps = { { level = 1, id = 3, u = cu, v = cv }, { level = 0, id = 2, u = wu, v = wv } },
  }
end
-- AAUI.MapClimate(zoneGroupId): climates.id of the zones of a map (zones.zone_climate_id ->
-- zone_climate_elems.climate_id); Solzreed Peninsula is temperate (2).
AAUI.MapClimate = function(zoneId)
  if zoneId == 5 then return { 2 } end
  return nil
end
-- Conflict zones of the western continent (world_groups 3), as in reference/ingame/worldmap_continent:
-- Karkasse Ridgelands (19), Cinderstone Moor (20), Hellswamp (26) and Sanddeep (27) in tension (the
-- orange circles), Halcyona (22) at peace. Solzreed (5) is no conflict zone.
-- States: HPWS_TROUBLE_0 .. 4 = 0 .. 4, HPWS_BATTLE 5, HPWS_WAR 6, HPWS_PEACE 7.
local CONFLICT = { [19] = 1, [20] = 1, [22] = 7, [26] = 1, [27] = 1 }
AAUI.ConflictZones = function(worldId)
  if worldId == 3 then return { 19, 20, 22, 26, 27 } end
  return {}
end
AAUI.ConflictState = function(zoneId)
  local state = CONFLICT[zoneId]
  if state == nil then return -1 end
  return state, 3600000
end

-- map/ping.lua reads M_EXPAND_CLICKED (a flag of a removed expand button that no script ever sets) and calls
-- SetExpansion, which does not exist, when it is truthy; the client has it undefined. The fake constants
-- (native_constants.txt: every ALL_CAPS global the scripts read but never assign) gave it a number.
M_EXPAND_CLICKED = nil

-- Peace tax and conflict state of a zone group (the info frame at the bottom left of the map): none.
X2Dominion.GetPeaceTaxMoney = function(self, zoneId) return "" end
X2Dominion.GetPeaceTaxAAPoint = function(self, zoneId) return "" end
