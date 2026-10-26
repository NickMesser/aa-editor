-- Fake game data for the bag window (see fake_ingame.lua for the pattern).
--
-- A fresh level 1 character's bag as in reference/ingame/bag: the starter supplies of the game database
-- (character_supplies: Teleport Book, 3 Heart's Beat, 3 Mind's Edge, 3 Bound Hereafter Stones), names /
-- icons / prices from the game database (items, icons, localized_texts), in the four default inventory
-- tabs of the database (default_inventory_tabs / default_inventory_tab_groups). The purse is empty.
--
-- The bag scripts (inventory/sort_bag.lua, sort_inventory.lua) do not go through slot:EstablishSlot:
-- every slot is a cooldowninventorybutton the script fills with
--   X2Bag:SlotByIdx(virtualSlot)      -> real slot, nil = empty
--   X2Bag:GetBagItemInfo(1, realSlot) -> { lookType = itemType, itemGrade, name, ... } (tooltip info)
--   X2Item:GetItemIconSet(type, grade) -> { itemIcon, overIcon, frameIcon } (texture paths)
-- so the app serves those, not the AAUI hooks. A right click on an item calls
-- AAUI.UseSlot(ISLOT_BAG, realSlot, "RightButton") (X2Bag.HandleUse below).
--
-- FAKE.items is the item registry the other fakes share (fake_character.lua adds the equipment).

FAKE.items = FAKE.items or {}
local ITEMS = FAKE.items

local function icon(file) return "ui/icon/" .. file end

-- The client's grade frames: ui/icon/item_grade_<n><name>.dds, n = 0 (poor) .. 11 (mythic); 1 is the
-- plain "common" grade everything of a new character has.
local GRADE_NAMES = { [0] = "poor", "common", "uncommon", "rare", "ancient", "heroic", "unique", "artifact",
                      "wonder", "epic", "legendary", "mythic" }
local function grade_frame(grade)
  local name = GRADE_NAMES[grade]
  if name == nil then return nil end
  return string.format("ui/icon/item_grade_%d%s.dds", grade, name)
end

-- id -> { name, icon (file under ui/icon), grade, maxStack, group (item_groups id), level, price, refund, desc,
-- category (item_categories name), impl (the client's item_impl: "portal", ...), soulBound, sellable,
-- useSkill (items.use_skill_id: the tooltip's "Use" text is its description) }. From the game database.
local function item(id, t)
  t.id = id
  t.grade = t.grade or 1
  t.maxStack = t.maxStack or 1
  t.level = t.level or 0
  ITEMS[id] = t
end
FAKE.add_item = item
item(4045, { name = "Teleport Book", icon = "icon_item_teleport.dds", group = 10, price = 3000, refund = 2700,
             category = "Book", impl = "portal", soulBound = true, sellable = false, useSkill = 11216,
             desc = "Requires 1 or 3 Hereafter Stones.\nA Teleport Scroll utilizes the power stored in a Hereafter Stone "
               .. "to open a Worldgate. Jump through the gate to travel to a recorded location.\n\nOne Hereafter Stone "
               .. "contains enough power to send you anywhere on your current continent. Three stones can send you to a "
               .. "different continent." })
item(18791, { name = "Heart's Beat", icon = "icon_item_0392.dds", group = 5, maxStack = 100, price = 10, refund = 1,
              category = "Potion", soulBound = true, desc = "A healing potion for novice adventurers." })
item(18792, { name = "Mind's Edge", icon = "icon_item_0397.dds", group = 5, maxStack = 100, price = 10, refund = 1,
              category = "Potion", soulBound = true, desc = "A Mana Potion for novice adventurers." })
item(417, { name = "Bound Hereafter Stone", icon = "icon_item_0418.dds", group = 5, maxStack = 100, price = 0,
            category = "Contraption", soulBound = true, sellable = false,
            desc = "Hereafter Stones contain enough power to send you anywhere on your current continent." })
-- Descriptions of the items' use skills (X2Skill:GetSkillTooltip(skill, item, SIK_DESCRIPTION)).
FAKE.skill_descs = FAKE.skill_descs or {}
FAKE.skill_descs[11216] = "Jump into the Worldgate to teleport. Worldgates remain open for 30 seconds (or 60 seconds "
  .. "for characters Lv20+ in Occultism)."


-- Real bag slot -> { id, stack } (slots are 1-based; gaps are empty slots).
local BAG_CAPACITY = 50
local BAG_MAX_CAPACITY = 130 -- the scripts build 13 rows of 10 slots
local bag = {
  [1] = { id = 4045, stack = 1 },
  [2] = { id = 18791, stack = 3 },
  [3] = { id = 18792, stack = 3 },
  [4] = { id = 417, stack = 3 },
}

-- Tabs: [1] is "All" (implicit), the others come from default_inventory_tabs.
-- { name, icon index (BUTTON_CONTENTS.BAG_TAB_ICON), item groups }
local tabs = {
  { name = "Equipment", icon = 1, groups = { 1, 2, 3, 4 } },
  { name = "Consumables", icon = 2, groups = { 5 } },
  { name = "Crafting Materials", icon = 3, groups = { 6, 22, 23, 26 } },
  { name = "Quest Items", icon = 4, groups = { 7 } },
}
local current_tab = 1 -- 1 = All, n + 1 = tabs[n]

local function in_groups(groups, g)
  for _, x in ipairs(groups) do
    if x == g then return true end
  end
  return false
end

-- Real slots shown in a tab, in order (the All tab shows every slot, empty ones too).
local function tab_slots(tab_idx)
  local t = {}
  if tab_idx == 1 then return nil end
  local groups = tabs[tab_idx - 1] and tabs[tab_idx - 1].groups or {}
  for slot = 1, BAG_CAPACITY do
    local b = bag[slot]
    if b and in_groups(groups, ITEMS[b.id].group) then t[#t + 1] = slot end
  end
  return t
end

local function count_items()
  local n = 0
  for _ in pairs(bag) do n = n + 1 end
  return n
end

-- Purse: empty (copper amount string, as fake_ingame.lua's money).
X2Util.GetMyMoneyString = function(self) return "0" end

-- X2Item ----------------------------------------------------------------------------------------------
X2Item.GetItemIconSet = function(self, itemType, grade)
  local it = ITEMS[itemType]
  if it == nil then return nil end
  return { itemIcon = icon(it.icon), frameIcon = grade_frame(grade or it.grade) }
end
X2Item.IsStackable = function(self, itemType)
  local it = ITEMS[itemType]
  return it ~= nil and it.maxStack > 1
end
X2Item.GetLevelRequirement = function(self, itemType)
  local it = ITEMS[itemType]
  return it and it.level or 0
end
X2Item.IsPetArmor = function(self, itemType) return false end
for _, mode in ipairs({ "Convert", "Enchant", "RechargeItemBuff", "Dyeing", "SecurityLock", "SecurityUnlock",
                        "ItemLookConvert", "ItemSkinize" }) do
  X2Item["IsIn" .. mode .. "Mode"] = function(self) return false end
end

-- The tooltip info of an item (what the client's GetBagItemInfo / GetEquippedItemTooltipText return);
-- the fields the bag and equipment scripts read, plus what the game tooltip shows.
function FAKE.item_info(id, stack)
  local it = ITEMS[id]
  if it == nil then return nil end
  return {
    itemType = id,
    lookType = id,
    itemGrade = it.grade,
    -- The grade's name and colour (item_grades: "Basic", FFBA976D for the common grade): the name is drawn in it.
    grade = "Basic",
    gradeColor = "FFBA976D",
    name = it.name,
    category = it.category,
    item_impl = it.impl or "etc",
    soul_bound = it.soulBound and 1 or nil,
    skillType = it.useSkill,
    description = it.desc,
    stack = stack or 1,
    isStackable = it.maxStack > 1,
    level_requirement = it.level,
    level_limit = 0,
    price = it.price,
    refund = it.refund or it.price,
    sellable = it.sellable ~= false and it.price > 0,
  }
end

-- The bag slots of AAUI.SlotContent(ISLOT_BAG, realSlot) (fake_ingame.lua serves the action bars): the same
-- items, in case a `slot` widget is bound to a bag slot. The bag window itself does not use it.
local hud_slot_content = AAUI.SlotContent
AAUI.SlotContent = function(slotType, slotIdx)
  if slotType == ISLOT_BAG then
    local b = bag[slotIdx]
    if b == nil then return nil end
    local it = ITEMS[b.id]
    return { type = "item", id = b.id, icon = icon(it.icon), grade = it.grade,
             stack = (it.maxStack > 1) and b.stack or nil }
  end
  return hud_slot_content(slotType, slotIdx)
end

-- X2Bag -----------------------------------------------------------------------------------------------
-- Virtual (tab) slot -> real slot, nil = empty.
X2Bag.SlotByIdx = function(self, virtualSlotIdx)
  if current_tab == 1 then
    if bag[virtualSlotIdx] ~= nil then return virtualSlotIdx end
    return nil
  end
  return tab_slots(current_tab)[virtualSlotIdx]
end
X2Bag.IsInvalidSlot = function(self, virtualSlotIdx) return virtualSlotIdx < 1 or virtualSlotIdx > BAG_CAPACITY end
X2Bag.GetBagItemInfo = function(self, bagId, realSlot, needInfo)
  local b = bag[realSlot]
  if b == nil then return nil end
  return FAKE.item_info(b.id, b.stack)
end
X2Bag.ItemIdentifier = function(self, realSlot)
  local b = bag[realSlot]
  if b == nil then return 0, 0 end
  return b.id, ITEMS[b.id].grade
end
X2Bag.ItemStack = function(self, realSlot)
  local b = bag[realSlot]
  return b and b.stack or 0
end
X2Bag.ItemDurability = function(self, realSlot, checkRepairable) return nil end -- no durability: nil
X2Bag.ItemRequireLevel = function(self, realSlot) return 0 end
X2Bag.IsLocked = function(self, realSlot) return false end
-- false marks the slot "new" (the corner flag) until the bag is closed (Confirm); the starter supplies
-- are all confirmed. FAKE.bag_new(realSlot) flags one.
local unconfirmed = {}
FAKE.bag_new = function(realSlot) unconfirmed[realSlot] = true end
X2Bag.IsConfirmedSlot = function(self, virtualSlot)
  local real = X2Bag:SlotByIdx(virtualSlot)
  return not (real and unconfirmed[real])
end
X2Bag.GetBagCooldown = function(self, realSlot) return nil end
X2Bag.Capacity = function(self) return BAG_CAPACITY end
X2Bag.ExpansionMaxSlotCount = function(self) return BAG_MAX_CAPACITY end
X2Bag.GetBagNumSlots = function(self, bagId) return BAG_CAPACITY end
X2Bag.CountItems = function(self) return count_items() end
X2Bag.CountEmptyBagSlots = function(self) return BAG_CAPACITY - count_items() end
X2Bag.Slots = function(self) return BAG_CAPACITY end
X2Bag.CurrentTabIdx = function(self) return current_tab end
X2Bag.TabCount = function(self) return #tabs + 1 end
X2Bag.TabInfos = function(self)
  local names, icons = {}, {}
  for i, t in ipairs(tabs) do
    names[i] = t.name
    icons[i] = t.icon
  end
  return names, icons
end
X2Bag.TabInfo = function(self, tabIdx)
  local t = tabs[tabIdx - 1]
  if t == nil then return nil end
  return t.name, t.icon, t.groups
end
X2Bag.SwitchTab = function(self, tabIdx)
  if tabIdx < 1 or tabIdx > #tabs + 1 then return end
  current_tab = tabIdx
  FAKE.fire_later("BAG_TAB_SWITCHED")
end
X2Bag.CreateTab = function(self, iconIdx, text, groups)
  tabs[#tabs + 1] = { name = text, icon = iconIdx, groups = groups or {} }
  current_tab = #tabs + 1
  FAKE.fire_later("BAG_TAB_CREATED")
end
X2Bag.EditTab = function(self, tabIdx, iconIdx, text, groups)
  local t = tabs[tabIdx - 1]
  if t == nil then return end
  t.name, t.icon, t.groups = text, iconIdx, groups or {}
  FAKE.fire_later("BAG_TAB_SWITCHED")
end
X2Bag.RemoveTab = function(self, tabIdx)
  if tabs[tabIdx - 1] == nil then return end
  table.remove(tabs, tabIdx - 1)
  current_tab = 1
  FAKE.fire_later("BAG_TAB_REMOVED")
  FAKE.fire_later("BAG_TAB_SWITCHED")
end
-- Sort: items move to the front of the bag, in item id order.
X2Bag.ManualSort = function(self)
  local list = {}
  for _, b in pairs(bag) do list[#list + 1] = b end
  table.sort(list, function(a, b) return a.id < b.id end)
  bag = {}
  for i, b in ipairs(list) do bag[i] = b end
  FAKE.fire_later("BAG_TAB_SORTED")
end
X2Bag.Sort = X2Bag.ManualSort
-- Clicks on the bag window's buttons (sort_inventory.lua OnClickProc): a right click on an item uses it
-- (X2Bag:HandleUse), which goes to the app like a click on an action bar slot, with the real slot:
--   AAUI.UseSlot(ISLOT_BAG, realSlot, "RightButton")
-- A left click or a drag (HandleClick) picks the item up onto the cursor in the client: not done here.
X2Bag.HandleClick = function(self, virtualSlot) end
X2Bag.HandleUse = function(self, virtualSlot)
  local real = X2Bag:SlotByIdx(virtualSlot)
  if real ~= nil and AAUI.UseSlot ~= nil then AAUI.UseSlot(ISLOT_BAG, real, "RightButton") end
end
X2Bag.Confirm = function(self) unconfirmed = {} end
X2Bag.FindBagItemByEquipSlot = function(self, bagId, equipSlot) return {} end -- fake_character.lua
