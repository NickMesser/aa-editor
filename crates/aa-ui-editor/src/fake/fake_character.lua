-- Fake game data for the character window (see fake_ingame.lua for the pattern; the item registry
-- FAKE.items and FAKE.item_info come from fake_bag.lua, which loads first).
--
-- The level 1 Nuian Battlerage character wears the starter equipment of the database
-- (character_equip_packs: ability 1 -> equip_pack_cloths 11 "first steps armor" and
-- equip_pack_weapons 242 "beginner of fighting"):
--   shirt 23387, pants 23388, shoes 23390 (Civilian cloth; icons costume_*/nu_m_*_cloth510)
--   main hand 5569 longsword, off hand 6152 shield, ranged 6127 bow, musical 6177 lute
-- The stats panel reads X2Unit:UnitInfo("player") (fake_ingame.lua STATS).

local add_item = FAKE.add_item
-- id, name, icon file under ui/icon, price (items / icons / localized_texts of the game database).
add_item(23387, { name = "Civilian Shirt", icon = "costume_ar/nu_m_ar_cloth510.dds", group = 2, price = 660 })
add_item(23388, { name = "Civilian Pants", icon = "costume_pt/nu_m_pt_cloth510.dds", group = 2, price = 520 })
add_item(23390, { name = "Civilian Shoes", icon = "costume_bo/nu_m_bo_cloth510.dds", group = 2, price = 320 })
add_item(5569, { name = "Civilian Longsword", icon = "icon_item_sword_1h_0001.dds", group = 1, price = 660 })
add_item(6152, { name = "Civilian Shield", icon = "icon_item_shield_0001.dds", group = 1, price = 660 })
add_item(6127, { name = "Civilian Bow", icon = "icon_item_bow_0001.dds", group = 1, price = 660 })
add_item(6177, { name = "Civilian Lute", icon = "icon_item_ins_s_0001.dds", group = 1, price = 660,
                 desc = "Sounds like a guitar when playing sheet music." })

-- Equipment slot (ES_*) -> item type. Slots not listed are empty.
local equipped = {
  [ES_CHEST] = 23387,
  [ES_LEGS] = 23388,
  [ES_FEET] = 23390,
  [ES_MAINHAND] = 5569,
  [ES_OFFHAND] = 6152,
  [ES_RANGED] = 6127,
  [ES_MUSICAL] = 6177,
}

-- Tooltip info of the item in an equipment slot; an empty slot has itemType 0 (the scripts test it).
local function equipped_info(slot)
  local id = equipped[slot]
  if id == nil then return { itemType = 0, lookType = 0, itemGrade = 0 } end
  local info = FAKE.item_info(id, 1)
  info.durability = 100
  info.maxDurability = 100
  info.equip = true
  return info
end
X2Equipment.GetEquippedItemTooltipText = function(self, unit, slot) return equipped_info(slot) end
X2Equipment.GetEquippedItemTooltipInfo = function(self, slot, flags) return equipped_info(slot) end
X2Equipment.ItemIdentifier = function(self, slot)
  local id = equipped[slot]
  if id == nil then return 0, 0 end
  return id, FAKE.items[id].grade
end
X2Equipment.IsInventoriable = function(self, slot) return true end
X2Equipment.DameagedItems = function(self, includeBag) return {} end
X2Equipment.DameagedItemCount = function(self, includeBag) return 0 end
for _, name in ipairs({ "PickupEquippedItem", "UnequipItem", "UseEquippedItem", "ItemRepair" }) do
  X2Equipment[name] = function(self, slot) end
end

-- Stats the detail popup reads besides the ones the HUD fake already has (incoming damage per tribe).
local stats = X2Unit:UnitInfo("player")
for _, k in ipairs({ "incoming_melee_damage_val", "incoming_ranged_damage_val", "incoming_spell_damage_val",
                     "incoming_siege_damage_val", "incoming_melee_damage_mul", "incoming_ranged_damage_mul",
                     "incoming_spell_damage_mul", "incoming_siege_damage_mul" }) do
  if stats[k] == nil then stats[k] = 0 end
end
