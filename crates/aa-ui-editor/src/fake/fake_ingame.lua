-- Development stand-ins for the in-game game API the HUD reads (reference/ingame/spawn: a
-- level 1 Nuian male "Refone" just spawned in Solzreed Peninsula / Desiree Peak, Nuia Alliance).
-- Loaded after fake_game.lua, before the world-stage scripts. Every X2 method overridden here is a
-- native of the client's in-game state (reference/api/x2_globals_ingame.json). The shapes returned
-- are what the original UI scripts read. Static data (skills, items, buffs, vocations, expert
-- grades, farm groups ...) comes from the game database (D:/aaemu-live/rs-game/Data/compact.sqlite3,
-- English texts from its localized_texts table).

AAUI = AAUI or {}

-- Scenario ---------------------------------------------------------------------------------------
local P = {
  name = "Refone", level = 1, race = RACE_NUIAN, gender = GENDER_MALE, unitId = "262144",
  hp = 540, maxHp = 540, mp = 310, maxMp = 310,
  labor = 105, maxLabor = 5000, bm = "0",
  exp = 0, expStr = "0", levelUpExpStr = "1000",
  money = "0",
  ability = "fight", -- Battlerage: the one ability of a level 1 character
}
FAKE.player = P

local function isPlayer(unit)
  return unit == "player" or unit == P.unitId
end

-- Game time of the capture: 7:05 PM (the sun of the clock widget).
local gameTime = { isAm = false, hour = 7, minute = 5.17 }

-- Static data from the game database --------------------------------------------------------------
local EXPERT_GRADES = {
  [0] = { name = "Amateur", upLimit = 10000, expertLimit = 0, advantage = 0, castAdvantage = 0, color = "FFBA976D", gaugeColor = "FFBA976D" },
  [1] = { name = "Novice", upLimit = 20000, expertLimit = 7, advantage = 5, castAdvantage = 5, color = "FF77B064", gaugeColor = "FF77B064" },
  [2] = { name = "Veteran", upLimit = 30000, expertLimit = 6, advantage = 10, castAdvantage = 10, color = "FF558FD7", gaugeColor = "FF558FD7" },
  [3] = { name = "Expert", upLimit = 40000, expertLimit = 5, advantage = 15, castAdvantage = 15, color = "FFCB72D8", gaugeColor = "FFCB72D8" },
  [4] = { name = "Master", upLimit = 50000, expertLimit = 4, advantage = 20, castAdvantage = 20, color = "FFD78B06", gaugeColor = "FFD78B06" },
  [5] = { name = "Authority", upLimit = 70000, expertLimit = 3, advantage = 23, castAdvantage = 23, color = "FFE17853", gaugeColor = "FFE17853" },
  [6] = { name = "Champion", upLimit = 90000, expertLimit = 2, advantage = 23, castAdvantage = 23, color = "FFF95252", gaugeColor = "FFF95252" },
  [7] = { name = "Artisan Lv3", upLimit = 110000, expertLimit = 0, advantage = 23, castAdvantage = 23, color = "FFCF7D5D", gaugeColor = "FFCF7D5D" },
  [8] = { name = "Artisan Lv4", upLimit = 130000, expertLimit = 0, advantage = 23, castAdvantage = 23, color = "FF8FA5CA", gaugeColor = "FF8FA5CA" },
  [9] = { name = "Artisan Lv5", upLimit = 150000, expertLimit = 0, advantage = 23, castAdvantage = 23, color = "FFBF7900", gaugeColor = "FFBF7900" },
  [10] = { name = "Maestro Lv1", upLimit = 180000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFC90B0B", gaugeColor = "FFC90B0B" },
  [11] = { name = "Maestro Lv2", upLimit = 210000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [12] = { name = "Maestro Lv3", upLimit = 240000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [13] = { name = "Maestro Lv4", upLimit = 270000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [14] = { name = "Maestro Lv5", upLimit = 300000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [15] = { name = "Maestro Lv6", upLimit = 330000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [16] = { name = "Maestro Lv7", upLimit = 360000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [17] = { name = "Maestro Lv8", upLimit = 390000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [18] = { name = "Maestro Lv9", upLimit = 420000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [19] = { name = "Maestro Lv10", upLimit = 450000, expertLimit = 0, advantage = 26, castAdvantage = 26, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [20] = { name = "Sage Lv1", upLimit = 500000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [21] = { name = "Sage Lv2", upLimit = 550000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [22] = { name = "Sage Lv3", upLimit = 600000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [23] = { name = "Sage Lv4", upLimit = 650000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [24] = { name = "Sage Lv5", upLimit = 700000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [25] = { name = "Sage Lv6", upLimit = 750000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [26] = { name = "Sage Lv7", upLimit = 800000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [27] = { name = "Sage Lv8", upLimit = 850000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [28] = { name = "Sage Lv9", upLimit = 900000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [29] = { name = "Sage Lv10", upLimit = 950000, expertLimit = 0, advantage = 29, castAdvantage = 29, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
  [30] = { name = "Legend", upLimit = 1000000, expertLimit = 0, advantage = 30, castAdvantage = 30, color = "FFFFFFFF", gaugeColor = "FFFFFFFF" },
}
local ACTABILITIES = {
  { type = 1, name = "Alchemy", icon = "ui/icon/craft/alchemy.dds", visible = true, skillPageVisible = true },
  { type = 2, name = "Construction", icon = "ui/icon/craft/build.dds", visible = true, skillPageVisible = true },
  { type = 3, name = "Cooking", icon = "ui/icon/craft/cook.dds", visible = true, skillPageVisible = true },
  { type = 4, name = "Handicrafts", icon = "ui/icon/craft/handicraft.dds", visible = true, skillPageVisible = true },
  { type = 5, name = "Husbandry", icon = "ui/icon/craft/animal.dds", visible = true, skillPageVisible = true },
  { type = 6, name = "Farming", icon = "ui/icon/craft/farm.dds", visible = true, skillPageVisible = true },
  { type = 7, name = "Fishing", icon = "ui/icon/craft/fish.dds", visible = false, skillPageVisible = true },
  { type = 8, name = "Logging", icon = "ui/icon/craft/felling.dds", visible = true, skillPageVisible = true },
  { type = 9, name = "Gathering", icon = "ui/icon/craft/collect.dds", visible = true, skillPageVisible = true },
  { type = 10, name = "Machining", icon = "ui/icon/craft/machine.dds", visible = true, skillPageVisible = true },
  { type = 11, name = "Metalwork", icon = "ui/icon/craft/metal.dds", visible = true, skillPageVisible = true },
  { type = 12, name = "Printing", icon = "ui/icon/craft/print.dds", visible = true, skillPageVisible = true },
  { type = 13, name = "Mining", icon = "ui/icon/craft/ore.dds", visible = false, skillPageVisible = true },
  { type = 14, name = "Masonry", icon = "ui/icon/craft/stone.dds", visible = true, skillPageVisible = true },
  { type = 15, name = "Tailoring", icon = "ui/icon/craft/cloth.dds", visible = true, skillPageVisible = true },
  { type = 16, name = "Leatherwork", icon = "ui/icon/craft/tanning.dds", visible = true, skillPageVisible = true },
  { type = 17, name = "Weaponry", icon = "ui/icon/craft/blacksmith.dds", visible = true, skillPageVisible = true },
  { type = 18, name = "Carpentry", icon = "ui/icon/craft/woodwork.dds", visible = true, skillPageVisible = true },
  { type = 19, name = "Quest", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = false },
  { type = 20, name = "Larceny", icon = "ui/icon/craft/crime.dds", visible = false, skillPageVisible = true },
  { type = 21, name = "Nuian Language", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = true },
  { type = 22, name = "Elven Language", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = true },
  { type = 23, name = "Dwarven Language", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = false },
  { type = 24, name = "Faerie Language", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = false },
  { type = 25, name = "Harani Language", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = true },
  { type = 26, name = "Firran Language", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = true },
  { type = 27, name = "Warborn Language", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = false },
  { type = 28, name = "Returned Language", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = false },
  { type = 29, name = "Nuia Continent Dialect", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = true },
  { type = 30, name = "Haranya Continent Dialect", icon = "ui/icon/craft/language.dds", visible = false, skillPageVisible = true },
  { type = 31, name = "Commerce", icon = "ui/icon/craft/trade.dds", visible = true, skillPageVisible = true },
  { type = 32, name = "Commemorative Event", icon = "ui/icon/craft/event.dds", visible = false, skillPageVisible = false },
  { type = 33, name = "Artistry", icon = "ui/icon/craft/art.dds", visible = true, skillPageVisible = true },
  { type = 34, name = "Exploration", icon = "ui/icon/craft/exploration.dds", visible = false, skillPageVisible = false },
}
local FARM_GROUPS = {
  { type = 1, name = "Public Farm", count = 10, description = "Land meant for growing crops." },
  { type = 2, name = "Pub. Nursery", count = 5, description = "Land meant for growing trees." },
  { type = 3, name = "Public Ranch", count = 5, description = "Land meant for raising livestock." },
  { type = 4, name = "Public Stable", count = 5, description = "Land meant for raising mounts." },
}
local CRAFT_CATEGORIES = {
  { type = 13, name = "Tax Certificate", group = 19 },
  { type = 26, name = "Ships", group = 10 },
  { type = 35, name = "Paper", group = 12 },
  { type = 42, name = "Scrolls", group = 12 },
  { type = 43, name = "Books", group = 12 },
  { type = 44, name = "Husbandry Gear", group = 5 },
  { type = 76, name = "Farming Seed Bundles", group = 6 },
  { type = 77, name = "Gathering Seed Bundles", group = 9 },
  { type = 78, name = "Farm Tools", group = 6 },
  { type = 80, name = "Carpentry Supplies", group = 18 },
  { type = 88, name = "Prized Possessions", group = 19 },
  { type = 95, name = "Fishing Rods", group = 4 },
  { type = 96, name = "Tailoring Gear", group = 15 },
  { type = 97, name = "Leather Goods", group = 16 },
  { type = 103, name = "Special Alchemy Goods", group = 1 },
  { type = 104, name = "Special Construction Goods", group = 19 },
  { type = 106, name = "Special Husbandry Goods", group = 19 },
  { type = 107, name = "Special Handicraft Goods", group = 4 },
  { type = 108, name = "Special Farming Goods", group = 19 },
  { type = 109, name = "Special Fishing Goods", group = 19 },
  { type = 110, name = "Special Logging Goods", group = 19 },
  { type = 111, name = "Special Gathering Goods", group = 19 },
  { type = 112, name = "Special Machining Goods", group = 19 },
  { type = 114, name = "Special Printing Goods", group = 19 },
  { type = 115, name = "Special Mining Goods", group = 19 },
  { type = 116, name = "Special Masonry Goods", group = 19 },
  { type = 117, name = "Special Tailoring Goods", group = 15 },
  { type = 118, name = "Special Leatherwork Goods", group = 16 },
  { type = 119, name = "Special Weaponry Goods", group = 17 },
  { type = 120, name = "Special Carpentry Goods", group = 18 },
  { type = 122, name = "Fishing Gear", group = 4 },
  { type = 124, name = "Commemorative Coin Rewards", group = 32 },
  { type = 129, name = "Bright Future's Promise", group = 32 },
  { type = 130, name = "Solid Future's Promise", group = 32 },
  { type = 131, name = "Hopeful Future's Promise", group = 32 },
  { type = 132, name = "Opulent Future's Promise", group = 32 },
  { type = 136, name = "Abyssal Weapon", group = 19 },
  { type = 137, name = "Abyssal Legacy", group = 19 },
  { type = 139, name = "Decor: Stone Handicraft", group = 14 },
  { type = 140, name = "Ancient Dragon Necklace", group = 19 },
  { type = 141, name = "Ancient Dragon Ring", group = 19 },
  { type = 142, name = "Ancient Dragon Earring", group = 19 },
  { type = 153, name = "Dragon Slayer's Artifact", group = 19 },
  { type = 154, name = "Proven Warrior Necklace", group = 19 },
  { type = 156, name = "Red Dragon's Legacy", group = 32 },
  { type = 157, name = "Enraged Red Dragon's Legacy", group = 32 },
  { type = 160, name = "Fusion Alembic", group = 32 },
  { type = 161, name = "Customized Gear", group = 32 },
  { type = 162, name = "Premium Racewear", group = 32 },
  { type = 163, name = "Handicraft Supplies", group = 4 },
  { type = 164, name = "Premium Champion", group = 32 },
  { type = 166, name = "Fishing Champ Supplies", group = 19 },
  { type = 168, name = "Premium Sportswear", group = 32 },
  { type = 169, name = "Premium Jeans", group = 32 },
  { type = 170, name = "Premium Sneakers", group = 32 },
  { type = 173, name = "Premium Consumable", group = 32 },
  { type = 174, name = "Premium Pack", group = 32 },
  { type = 175, name = "Premium Weapon", group = 32 },
  { type = 177, name = "Mirage Gold Voucher", group = 19 },
  { type = 179, name = "Sunpoint Regrade Surprise", group = 19 },
  { type = 181, name = "Lunagem", group = 4 },
  { type = 249, name = "Spellbook", group = 12 },
  { type = 250, name = "Runes and Dice", group = 14 },
  { type = 251, name = "Index", group = 12 },
  { type = 252, name = "New Year's Feast", group = 32 },
  { type = 253, name = "Woodlot", group = 8 },
  { type = 254, name = "Animal Pen", group = 5 },
  { type = 255, name = "ArcheAge Birthday Raffle Ticket", group = 19 },
  { type = 263, name = "Prime Vocation Boost Codex", group = 19 },
  { type = 265, name = "Divine Key", group = 19 },
  { type = 266, name = "Artistry Goods", group = 33 },
  { type = 268, name = "Construction Materials", group = 2 },
  { type = 269, name = "Designs", group = 2 },
  { type = 270, name = "Experia Plants", group = 9 },
  { type = 271, name = "Experia Seeds", group = 6 },
  { type = 272, name = "Experia Livestock", group = 5 },
  { type = 273, name = "Experia Saplings", group = 8 },
  { type = 25, name = "Siege Gear", group = 10 },
  { type = 267, name = "Small Instruments", group = 33 },
  { type = 100, name = "Instruments", group = 33 },
  { type = 152, name = "Specialty Dye", group = 32 },
  { type = 150, name = "Premium Fusion Item", group = 32 },
  { type = 1, name = "Daggers", group = 17 },
  { type = 2, name = "Swords", group = 17 },
  { type = 4, name = "Katanas", group = 17 },
  { type = 5, name = "Clubs", group = 17 },
  { type = 6, name = "Scepters", group = 18 },
  { type = 7, name = "Shortspears", group = 17 },
  { type = 17, name = "Cloth Armor", group = 15 },
  { type = 18, name = "Leather Armor", group = 16 },
  { type = 23, name = "Metal", group = 11 },
  { type = 41, name = "Potions", group = 1 },
  { type = 47, name = "Axes", group = 17 },
  { type = 56, name = "Greatswords", group = 17 },
  { type = 57, name = "Nodachis", group = 17 },
  { type = 58, name = "Greataxes", group = 17 },
  { type = 59, name = "Greatclubs", group = 17 },
  { type = 60, name = "Longspears", group = 17 },
  { type = 61, name = "Staves", group = 18 },
  { type = 247, name = "Roasted Meat", group = 3 },
  { type = 8, name = "Bows", group = 18 },
  { type = 9, name = "Explosives", group = 17 },
  { type = 10, name = "Shields", group = 17 },
  { type = 29, name = "Bread and Soup", group = 3 },
  { type = 50, name = "Honor: Metal Weapons", group = 17 },
  { type = 54, name = "Honor: Wood Weapons", group = 18 },
  { type = 83, name = "Hoods", group = 15 },
  { type = 84, name = "Caps", group = 16 },
  { type = 85, name = "Helms", group = 11 },
  { type = 243, name = "Minor Healing Potion", group = 1 },
  { type = 30, name = "Drinks", group = 3 },
  { type = 184, name = "Shirts", group = 15 },
  { type = 190, name = "Jerkins", group = 16 },
  { type = 196, name = "Cuirasses", group = 11 },
  { type = 245, name = "Minor Mana Potion", group = 1 },
  { type = 11, name = "Lutes", group = 4 },
  { type = 12, name = "Flutes", group = 4 },
  { type = 185, name = "Pants", group = 15 },
  { type = 191, name = "Breeches", group = 16 },
  { type = 197, name = "Greaves", group = 11 },
  { type = 244, name = "Healing Potion", group = 1 },
  { type = 248, name = "Feast Table", group = 3 },
  { type = 14, name = "Necklaces", group = 4 },
  { type = 15, name = "Rings", group = 4 },
  { type = 16, name = "Earrings", group = 4 },
  { type = 105, name = "Special Cooking Goods", group = 3 },
  { type = 186, name = "Gloves", group = 15 },
  { type = 192, name = "Fists", group = 16 },
  { type = 198, name = "Gauntlets", group = 11 },
  { type = 246, name = "Mana Potion", group = 1 },
  { type = 71, name = "Lunadrops", group = 1 },
  { type = 72, name = "Lunafrosts", group = 1 },
  { type = 187, name = "Shoes", group = 15 },
  { type = 193, name = "Boots", group = 16 },
  { type = 199, name = "Sabatons", group = 11 },
  { type = 53, name = "Talismans", group = 1 },
  { type = 188, name = "Sleeves", group = 15 },
  { type = 194, name = "Guards", group = 16 },
  { type = 200, name = "Vambraces", group = 11 },
  { type = 24, name = "Vehicles", group = 10 },
  { type = 45, name = "Contraptions", group = 4 },
  { type = 52, name = "Gliders", group = 10 },
  { type = 189, name = "Sashes", group = 15 },
  { type = 195, name = "Belts", group = 16 },
  { type = 201, name = "Tassets", group = 11 },
  { type = 31, name = "Lumber", group = 18 },
  { type = 32, name = "Hide", group = 16 },
  { type = 33, name = "Stone Brick", group = 14 },
  { type = 36, name = "Fabric", group = 15 },
  { type = 39, name = "Archeum", group = 1 },
  { type = 27, name = "Components", group = 10 },
  { type = 46, name = "Pigment/Oil/Polish", group = 1 },
  { type = 48, name = "Alchemy", group = 1 },
  { type = 81, name = "Specialty Pigments", group = 1 },
  { type = 66, name = "Storage Chests", group = 4 },
  { type = 82, name = "Cloth Costume", group = 15 },
  { type = 135, name = "Leather Costume", group = 16 },
  { type = 138, name = "Ship Supplies", group = 10 },
  { type = 68, name = "Beds", group = 18 },
  { type = 69, name = "Chairs", group = 18 },
  { type = 62, name = "Cabinets", group = 18 },
  { type = 70, name = "Tables", group = 18 },
  { type = 64, name = "Lighting", group = 4 },
  { type = 65, name = "Decor", group = 4 },
  { type = 63, name = "Structures", group = 18 },
  { type = 20, name = "Haranyan Specialties", group = 31 },
  { type = 21, name = "Nuian Specialties", group = 31 },
  { type = 180, name = "Fellowship Specialities: Haranya", group = 31 },
  { type = 182, name = "Fellowship Specialities: Nuia", group = 31 },
  { type = 183, name = "Fellowship Specialities: Auroria", group = 19 },
  { type = 22, name = "Commerce", group = 31 },
  { type = 94, name = "Plate Costume", group = 11 },
  { type = 113, name = "Special Metalcraft Goods", group = 11 },
  { type = 121, name = "Metal Fishing Pole Frames", group = 11 },
  { type = 202, name = "Dagger Reforging", group = 10 },
  { type = 203, name = "Sword Reforging", group = 10 },
  { type = 204, name = "Greatsword Reforging", group = 10 },
  { type = 205, name = "Katana Reforging", group = 10 },
  { type = 206, name = "Nodachi Reforging", group = 10 },
  { type = 207, name = "Axe Reforging", group = 10 },
  { type = 208, name = "Greataxe Reforging", group = 10 },
  { type = 209, name = "Club Reforging", group = 10 },
  { type = 210, name = "Greatclub Reforging", group = 10 },
  { type = 211, name = "Shortspear Reforging", group = 10 },
  { type = 212, name = "Longspear Reforging", group = 10 },
  { type = 213, name = "Scepter Reforging", group = 10 },
  { type = 214, name = "Staff Reforging", group = 10 },
  { type = 215, name = "Bow Reforging", group = 10 },
  { type = 216, name = "Shield Reforging", group = 10 },
  { type = 217, name = "Lute Reforging", group = 10 },
  { type = 218, name = "Flute Reforging", group = 10 },
  { type = 219, name = "Hood Reforging", group = 10 },
  { type = 220, name = "Shirt Reforging", group = 10 },
  { type = 221, name = "Pants Reforging", group = 10 },
  { type = 222, name = "Gloves Reforging", group = 10 },
  { type = 223, name = "Shoes Reforging", group = 10 },
  { type = 224, name = "Sleeves Reforging", group = 10 },
  { type = 225, name = "Sash Reforging", group = 10 },
  { type = 226, name = "Cap Reforging", group = 10 },
  { type = 227, name = "Jerkin Reforging", group = 10 },
  { type = 228, name = "Breeches Reforging", group = 10 },
  { type = 229, name = "Fists Reforging", group = 10 },
  { type = 230, name = "Boots Reforging", group = 10 },
  { type = 231, name = "Guards Reforging", group = 10 },
  { type = 232, name = "Belt Reforging", group = 10 },
  { type = 233, name = "Helm Reforging", group = 10 },
  { type = 234, name = "Cuirass Reforging", group = 10 },
  { type = 235, name = "Greaves Reforging", group = 10 },
  { type = 236, name = "Gauntlets Reforging", group = 10 },
  { type = 237, name = "Sabatons Reforging", group = 10 },
  { type = 238, name = "Vambraces Reforging", group = 10 },
  { type = 239, name = "Tassets Reforging", group = 10 },
  { type = 240, name = "Necklace Reforging", group = 10 },
  { type = 241, name = "Earring Reforging", group = 10 },
  { type = 242, name = "Ring Reforging", group = 10 },
  { type = 40, name = "Workbenches/stations", group = 18 },
}

-- Achievements per (main category, sub category): how many exist (none completed yet).
local ACHIEVEMENT_TOTALS = {
  [1] = { [1] = 46, [2] = 102, [3] = 131, [4] = 61 },
  [2] = { [1] = 126, [2] = 27, [4] = 13 },
  [3] = { [1] = 80, [2] = 113, [3] = 59, [4] = 106, [5] = 210 },
  [4] = { [1] = 424, [2] = 112, [3] = 36 },
  [5] = { [1] = 29, [2] = 5, [3] = 63 },
}
local ACHIEVEMENT_SUBCATEGORIES = { 4, 4, 5, 3, 3 }
-- Zone groups that produce trade goods (items.specialty_zone_id), for the specialty info window.
local PRODUCTION_ZONES = {
  { id = 1, name = "Gweonid Forest" },
  { id = 2, name = "Marianople" },
  { id = 3, name = "Dewstone Plains" },
  { id = 4, name = "Solis Headlands" },
  { id = 5, name = "Solzreed Peninsula" },
  { id = 6, name = "Lilyut Hills" },
  { id = 7, name = "Arcum Iris" },
  { id = 8, name = "Two Crowns" },
  { id = 9, name = "Mahadevi" },
  { id = 11, name = "Falcorth Plains" },
  { id = 12, name = "Villanelle" },
  { id = 14, name = "Windscour Savannah" },
  { id = 15, name = "Perinoor Ruins" },
  { id = 16, name = "Rookborne Basin" },
  { id = 17, name = "Ynystere" },
  { id = 18, name = "White Arden" },
  { id = 19, name = "Karkasse Ridgelands" },
  { id = 20, name = "Cinderstone Moor" },
  { id = 22, name = "Halcyona" },
  { id = 23, name = "Hasla" },
  { id = 24, name = "Tigerspine Mountains" },
  { id = 25, name = "Silent Forest" },
  { id = 26, name = "Hellswamp" },
  { id = 27, name = "Sanddeep" },
  { id = 33, name = "Heedmar" },
  { id = 34, name = "Nuimari" },
  { id = 43, name = "Marcala" },
  { id = 44, name = "Calmlands" },
  { id = 54, name = "Exeloch" },
  { id = 56, name = "Sungold Fields" },
}

-- Icons -------------------------------------------------------------------------------------------
local function icon(file) return "ui/icon/" .. file .. ".dds" end

-- Skills the new character has on its action bars (default_skills + the Battlerage starter skill).
local SKILLS = {
  [18132] = { name = "Triple Slash", icon = "icon_skill_fight02", usable = false },
  -- The tooltip of Shoot Arrow as the client builds it (reference/ingame/tooltip_skill: skills, skill texts;
  -- #{damage_multiplier} is 120 at rank 1).
  [16064] = { name = "Shoot Arrow", icon = "icon_skill_basic10", usable = false,
              tip = { ability = "general", skillLevel = 1, mana = 0, casting = 0, channeling = 0, minRange = 0,
                      maxRange = 20, cooldown = 0,
                      description = "Shoots an arrow at a target, dealing damage equal to |cFFFF9C27120%|r of Ranged "
                        .. "Attack.\nDamage is reduced |cFFFF9C27-20%|r if target is within |cFFFF9C278m|r. |cff00ff45\n"
                        .. "Fast skill; no cooldown. Hold down for continuous use." } },
  [2] = { name = "Melee Attack", icon = "icon_skill_basic01", usable = false },
  [16287] = { name = "Dash", icon = "icon_skill_basic09", usable = true },
  [14495] = { name = "Kick", icon = "icon_skill_fight22", usable = false },
  [14152] = { name = "Dance: Gotta Get Down", icon = "emotion/icon_emotion_039", usable = true },
  [10594] = { name = "Recall", icon = "icon_skill_basic04", usable = true },
  [14900] = { name = "Play Instrument", icon = "icon_skill_basic06", usable = true },
  [14902] = { name = "Drop Trade Pack", icon = "icon_skill_basic08", usable = false },
  [14901] = { name = "Use Glider", icon = "icon_skill_basic07", usable = false },
}

-- Items in the bag (also what the action bar's item slots show). stack counts as in the capture.
local ITEMS = {
  [4045] = { name = "Teleport Book", icon = "icon_item_teleport", grade = 1, stack = 1, maxStack = 1 },
  [18791] = { name = "Heart's Beat", icon = "icon_item_0392", grade = 1, stack = 3, maxStack = 100 },
  [18792] = { name = "Mind's Edge", icon = "icon_item_0397", grade = 1, stack = 3, maxStack = 100 },
}
local BAG_ORDER = { 4045, 18791, 18792 }
local BAG_CAPACITY = 50

-- Action bars: slotIdx (1..12 bar 1, 13..18 bar 2 left, 19..24 bar 2 right) -> skill / item.
local ACTION = {
  [1] = { "skill", 18132 }, [2] = { "skill", 16064 }, [3] = { "skill", 2 },
  [10] = { "item", 4045 }, [11] = { "item", 18791 }, [12] = { "item", 18792 },
  [13] = { "skill", 16287 }, [14] = { "skill", 14495 },
  [20] = { "skill", 14152 }, [21] = { "skill", 10594 }, [22] = { "skill", 14900 },
  [23] = { "skill", 14902 }, [24] = { "skill", 14901 },
}

-- Buffs on the player frame: path / buff id / timeLeft (ms, nil = permanent).
local BUFFS = {
  { buff_id = 4831, name = "Adapting to Society", path = icon("icon_skill_romance23"), timeLeft = 0, stack = 1,
    description = "Your time in prison has left you temporarily out-of-touch with society. Unable to attack or be attacked." },
  -- Patron (the premium buff; reference/ingame/tooltip_buff: its tooltip).
  { buff_id = 8000011, name = "Patron", path = icon("icon_skill_buff220"), stack = 1, tipType = "buff", category = "Buff",
    description = "Regenerates |cFFFF9C2710 Labor|r every |cFFFF9C275 minutes|r while online and |cFFFF9C275 Labor|r every "
      .. "|cFFFF9C275 minutes|r while offline.\nMax Labor: |cFFFF9C275000|r\nBestows |cFFFF9C275|r Loyalty Tokens daily "
      .. "for logging in.\nIncreases XP gain |cFFFF9C27+10%|r.\nAllows items to be listed on the Auction House." },
  { buff_id = 30810, name = "Tutorial Blessing", path = icon("icon_item_1657"), timeLeft = 6 * 86400000 + 3600000, stack = 1,
    description = "A gift for new adventurers." },
  { buff_id = 8226, name = "Equip Shield", path = icon("icon_skill_buff260"), stack = 1,
    description = "Wielding a shield increases Physical Defense and Magic Defense." },
}

-- Hooks of the toolkit's native widgets ------------------------------------------------------------
-- Skills cooling down (ms), for `cooldown = { remain, total }` of the slot hooks. The spawn capture has
-- none; with the example's --cooldown flag Dash was just used (12 s cooldown, 8 s left when the slot is
-- bound; the native counts it down from there).
local COOLDOWNS = FAKE_COOLDOWN_DEMO and { [16287] = { remain = 8000, total = 12000 } } or {}

-- The tooltip info of a skill (slot:GetTooltip): tipType "skill" and the fields the tooltip sections read.
function FAKE.skill_tooltip(id)
  local s = SKILLS[id]
  if s == nil or s.tip == nil then return nil end
  local t = { tipType = "skill", type = id, name = s.name, path = icon(s.icon) }
  for k, v in pairs(s.tip) do t[k] = v end
  return t
end
-- X2Skill:GetSkillTooltip(skillType, itemType, flags): the description of an item's use skill (the item
-- tooltip's "Use" text, FAKE.skill_descs of fake_bag.lua) or of an action bar skill.
X2Skill.GetSkillTooltip = function(self, skillType, itemType, flags)
  local t = FAKE.skill_tooltip(skillType)
  if t ~= nil then return t end
  return { description = (FAKE.skill_descs or {})[skillType] }
end

-- AAUI.SlotContent(slotType, slotIdx) -> nil | { type, id, icon, stack, grade [, canUse, cooldown, tooltip] }.
AAUI.SlotContent = function(slotType, slotIdx)
  if slotType == ISLOT_ACTION then
    local a = ACTION[slotIdx]
    if a == nil then return nil end
    if a[1] == "skill" then
      local s = SKILLS[a[2]]
      return { type = "skill", id = a[2], icon = icon(s.icon), canUse = s.usable, cooldown = COOLDOWNS[a[2]],
               tooltip = FAKE.skill_tooltip(a[2]) }
    end
    local it = ITEMS[a[2]]
    return { type = "item", id = a[2], icon = icon(it.icon), stack = (it.maxStack > 1) and it.stack or nil,
             grade = it.grade }
  elseif slotType == ISLOT_BAG then
    local id = BAG_ORDER[slotIdx]
    if id == nil then return nil end
    local it = ITEMS[id]
    return { type = "item", id = id, icon = icon(it.icon), stack = (it.maxStack > 1) and it.stack or nil,
             grade = it.grade }
  end
  return nil
end

-- AAUI.SlotHotKey(slotType, slotIdx): the key bound to an action slot, as the client names it (the scripts
-- abbreviate it for the label): bar 1 (1..12) is 1..9, 0, MINUS, EQUALS and bar 2 (13..24) the same with SHIFT-.
local BAR_KEYS = { "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "MINUS", "EQUALS" }
AAUI.SlotHotKey = function(slotType, slotIdx)
  if slotType ~= ISLOT_ACTION or type(slotIdx) ~= "number" then return nil end
  if slotIdx >= 1 and slotIdx <= 12 then return BAR_KEYS[slotIdx] end
  if slotIdx >= 13 and slotIdx <= 24 then return "SHIFT-" .. BAR_KEYS[slotIdx - 12] end
  return nil
end

-- AAUI.SkillContent(skillType) / AAUI.ItemContent(itemType, grade): the content of a slot filled by
-- slot:EstablishSkill / slot:EstablishItem (same shape as SlotContent; nil = unknown id).
AAUI.SkillContent = function(skillType)
  local s = SKILLS[skillType]
  if s == nil then return nil end
  return { type = "skill", id = skillType, icon = icon(s.icon), canUse = s.usable, cooldown = COOLDOWNS[skillType] }
end
AAUI.ItemContent = function(itemType, grade)
  local it = ITEMS[itemType]
  if it == nil then return nil end
  return { type = "item", id = itemType, icon = icon(it.icon), grade = grade or it.grade }
end

-- AAUI.ChatTabs(windowId): the tabs of the main chat window: All (every line), Party and Trade.
-- Filters are the CMF_* message filters a tab shows; a tab without filters shows every line.
local function filters(...)
  local t = {}
  for _, name in ipairs({ ... }) do
    local f = _G[name]
    if f ~= nil then t[#t + 1] = f end
  end
  return t
end
AAUI.ChatTabs = function(windowId)
  return {
    { id = 1, name = "All" },
    { id = 2, name = "Party", filters = filters("CMF_PARTY", "CMF_RAID", "CMF_RAID_COMMAND", "CMF_PARTY_AND_RAID_INFO") },
    { id = 3, name = "Trade", filters = filters("CMF_TRADE", "CMF_TRADE_STORE_MSG", "CMF_FIND_PARTY") },
  }
end
-- AAUI.ChatSend(channel, text, target): a line typed into the chat input (the slash command already
-- taken off; channel CHAT_SAY 0, CHAT_ZONE 1, ..., whisper -3 with the target's name). The server would
-- echo it as CHAT_MESSAGE; the fake does at once.
AAUI.ChatSend = function(channel, text, target)
  FAKE.fire_now("CHAT_MESSAGE", channel, "player", false, P.name, text, false, target ~= "" and target or nil)
end
-- AAUI.UseSlot(slotType, slotIdx, button), UseSkill(skillType, button), UseItem(itemType, grade, button):
-- a click on a slot. The game would cast or use; the fake only logs.
AAUI.UseSlot = function(slotType, slotIdx, button) UIParent:Log(string.format("use slot %d:%d %s", slotType, slotIdx, button)) end
AAUI.UseSkill = function(skillType, button) UIParent:Log(string.format("use skill %d %s", skillType, button)) end
AAUI.UseItem = function(itemType, grade, button) UIParent:Log(string.format("use item %d %s", itemType, button)) end

-- AAUI.RoadMap(): the zone map overlay (top right). level is the one the scripts register their maps
-- with (`roadmap:SaveMapData(RMS_ZONE + 1, ...)`: 2 = zone maps, 3 = city maps; the anchor handler
-- subtracts 1 again), zone = the id of the "<zone>_<size>" keys of map/road_map_list.lua (5 =
-- Solzreed Peninsula), player = position on the map image (0..1) and heading (radians clockwise from north:
-- the capture's entity yaw is 2.7705 rad counter-clockwise, so -2.7705; camera.json), markers = quest / NPC
-- icons on the map image (kind: a symbol of map/common.lua, see aa_ui::lua::roadmap).
local ROADMAP_W, ROADMAP_H = 780, 516   -- the "5_100" map image (map/form.lua W_SOLZREED_100)
AAUI.RoadMap = function()
  local pu, pv = 0.846, 0.265
  return {
    level = 2, zone = 5,
    player = { u = pu, v = pv, angle = -2.77053833 },
    markers = {
      -- Maude, the quest giver below the player in the capture (a yellow "!" 20 px south, 10 px west of the
      -- cursor; pixel-exact with real.png).
      { u = pu - 10 / ROADMAP_W, v = pv + 20 / ROADMAP_H, kind = "quest_zone_gives", tooltip = "Maude" },
    },
  }
end

-- X2 (global functions of the client) ----------------------------------------------------------------
-- Persisted per-character / per-account UI data (action bar layout, quest list state ...): a fresh
-- character has none, the scripts fill in their defaults. The one saved entry is the quest notifier's
-- window bound (ui_management: `ui_bound_<key>` = { screenResolution, bound }), which the reference
-- character had (x 1480 instead of the script default).
X2.GetCharacterUiData = function(self, key)
  if key == "ui_bound_questNotifier" then
    return {
      screenResolution = { x = UIParent:GetScreenWidth(), y = UIParent:GetScreenHeight() },
      bound = { x = 1480, y = 440, width = 440, height = 300 },
    }
  end
  return nil
end
X2.SetCharacterUiData = function(self, key, value) end
X2.GetAccountUiData = function(self, key) return nil end
X2.SetAccountUiData = function(self, key, value) end
X2.GetInstanceIndex = function(self) return nil end
X2.GetFarmGroups = function(self) return FARM_GROUPS end
X2.GetFarmGorupInfo = function(self, farmType)
  for _, g in ipairs(FARM_GROUPS) do
    if g.type == farmType then return g.name, g.count, g.description end
  end
  return "", 0, ""
end
X2.GetFarmGroupDoodadList = function(self, farmType) return {} end
X2.GetRuntimeCommonFarmDoodadsInfo = function(self, farmType) return {} end
X2.GetRuntimeCommonFarmDoodadCount = function(self, farmType) return 0 end
X2.GetQuestNotifierListUiData = function(self) return nil end
X2.GetQuestContextStateValuesUiData = function(self) return nil end
X2.IsEnteredWorld = function(self) return true end

-- Options ------------------------------------------------------------------------------------------------
-- A store like the client's option table: scripts create their options with a default
-- (CreateOptionItemFloat / String), then read and write them by name. Native (numeric) options
-- not set by the scripts read 0.
local optByName, optById = {}, {}
local screenOption = X2Option.GetOptionItemValue
X2Option.CreateOptionItemFloat = function(self, id, default, saveLevel)
  if optByName[id] == nil then optByName[id] = default end
end
X2Option.CreateOptionItemString = X2Option.CreateOptionItemFloat
X2Option.GetOptionItemValueByName = function(self, name) return optByName[name] end
X2Option.GetOptionItemValue = function(self, id)
  local v = screenOption(self, id)
  if v ~= nil then return v end
  v = optById[id]
  if v ~= nil then return v end
  -- the app's own value (the sound sliders: AAUI.OptionValue)
  if AAUI.OptionValue then
    v = AAUI.OptionValue(id)
    if v ~= nil then return v end
  end
  return 0
end
X2Option.SetItemFloatValueByName = function(self, name, v) optByName[name] = v end
X2Option.SetItemStringValueByName = X2Option.SetItemFloatValueByName
X2Option.SetItemFloatValue = function(self, id, v)
  optById[id] = v
  if AAUI.OptionChanged then AAUI.OptionChanged(id, v) end
end
X2Option.SetItemStringValue = X2Option.SetItemFloatValue
X2Option.SetItemFloatValueWithoutModify = X2Option.SetItemFloatValue
X2Option.SetItemDefaultFloatValueByName = function(self, name, v) if optByName[name] == nil then optByName[name] = v end end
X2Option.SetItemDefaultStringValueByName = X2Option.SetItemDefaultFloatValueByName
X2Option.SetItemDefaultFloatValue = function(self, id, v) if optById[id] == nil then optById[id] = v end end
X2Option.SetItemDefaultStringValue = X2Option.SetItemDefaultFloatValue
X2Option.IsRestartOption = function(self) return false end

-- Hotkeys ---------------------------------------------------------------------------------------------------
-- The first key of each action (the bag's is I: reference/ingame/tooltip_hud shows "Bag (I)").
local defaultKeys = {
  toggle_character = "C", toggle_bag = "I", toggle_quest_context_list = "L", toggle_achievement = "Y",
  toggle_skill = "K", toggle_worldmap = "M", toggle_craft = "N", toggle_community = "O",
  toggle_ranking = "R", toggle_option = "ESC", toggle_commercial_mail = "", toggle_ingameshop = "",
  toggle_friend = "F", toggle_expedition = "X", toggle_raid_team_manager = "",
}
X2Hotkey.GetBinding = function(self, action, index)
  if index ~= 1 then return nil end
  return defaultKeys[action]
end
X2Hotkey.GetOptionBinding = function(self, action, manager, ...)
  return nil
end

-- Chat ----------------------------------------------------------------------------------------------------------
-- Main chat window id; the tabs come from AAUI.ChatTabs.
X2Chat.AllChatWindowIds = function(self) return { 0 } end
X2Chat.GetChatChannelName = function(self, channel) return nil end

-- X2Unit ----------------------------------------------------------------------------------------------------------
X2Unit.UnitName = function(self, unit) if isPlayer(unit) then return P.name end return nil end
X2Unit.UnitLevel = function(self, unit) if isPlayer(unit) then return P.level end return nil end
X2Unit.UnitHealth = function(self, unit) if isPlayer(unit) then return P.hp end return nil end
X2Unit.UnitMaxHealth = function(self, unit) if isPlayer(unit) then return P.maxHp end return nil end
X2Unit.UnitMana = function(self, unit) if isPlayer(unit) then return P.mp end return nil end
X2Unit.UnitMaxMana = function(self, unit) if isPlayer(unit) then return P.maxMp end return nil end
X2Unit.UnitIsDead = function(self, unit) return false end
X2Unit.UnitIsOffline = function(self, unit) return false end
X2Unit.UnitCombatState = function(self, unit) return false end
X2Unit.UnitIsForceAttack = function(self, unit) return false end
X2Unit.UnitRace = function(self, unit) if isPlayer(unit) then return P.race end return nil end
X2Unit.UnitGender = function(self, unit) if isPlayer(unit) then return P.gender end return nil end
X2Unit.GetUnitId = function(self, unit) if isPlayer(unit) then return P.unitId end return nil end
X2Unit.GetTargetUnitId = function(self, unit) return nil end
X2Unit.GetUnitType = function(self, unit) if isPlayer(unit) then return "character" end return nil end
X2Unit.GetUnitTypeString = function(self, unit) if isPlayer(unit) then return "character" end return nil end
-- The player's faction as the character window shows it (reference/ingame/character_info).
X2Unit.GetFactionName = function(self, unit) if isPlayer(unit) then return "Crescent Throne" end return nil end
X2Unit.IsReporter = function(self, unit) return false end
X2Unit.UnitTeamAuthority = function(self, unit) return nil end
X2Unit.UnitInGroup = function(self, unit) return false end
X2Unit.UnitIsTeamMember = function(self, unit) return false end
X2Unit.GetModeActionsCount = function(self) return 0 end
X2Unit.GetOverHeadMarkerUnitId = function(self, index) return nil end
X2Unit.IsFirstHitByMeOrMyTeam = function(self, unit) return false end
X2Unit.GetCombatRelationshipStr = function(self, unit) return "friendly" end
X2Unit.UnitDistance = function(self, unit) return nil end
X2Unit.UnitCastingInfo = function(self, unit) return nil end
-- The abilities (skillsets) of a unit: a level 1 character only has Battlerage.
X2Unit.GetTargetAbilityTemplates = function(self, unit)
  if isPlayer(unit) then return { { name = P.ability } } end
  return nil
end
X2Unit.UnitBuffCount = function(self, unit) if isPlayer(unit) then return #BUFFS end return 0 end
X2Unit.UnitDeBuffCount = function(self, unit) return 0 end
X2Unit.UnitBuff = function(self, unit, index, flags)
  if not isPlayer(unit) then return nil end
  return BUFFS[index]
end
X2Unit.UnitDeBuff = function(self, unit, index, flags) return nil end

-- Abilities and skills ------------------------------------------------------------------------------------------------
local ABILITY_KEYS = { "fight", "illusion", "adamant", "will", "death", "wild", "magic", "vocation",
                       "romance", "love" }
X2Ability.FindAbilityIndexForStr = function(self, name)
  for i, key in ipairs(ABILITY_KEYS) do
    if key == name then return i end
  end
  return ABILITY_MAX
end
X2Ability.GetAbilityStr = function(self, index) return ABILITY_KEYS[index] end
-- The abilities the character has learned: Battlerage at level 1.
local function myAbility()
  return { type = ABILITY_FIGHT, level = 1, levelPercent = 0, exp = 0, prevLevelTotalExp = 0,
           nextLevelTotalExp = 1000 }
end
X2Ability.GetActiveAbility = function(self) return { myAbility() } end
X2Ability.NumActiveAbility = function(self) return 1 end
X2Ability.IndexFromAbility = function(self, ability)
  if ability == ABILITY_FIGHT then return 1 end
  return nil
end
-- One entry per ability type 1..10.
X2Ability.GetAllAbility = function(self)
  local all = {}
  for i = 1, 10 do all[i] = { type = i, level = (i == ABILITY_FIGHT) and 1 or 0 } end
  return all
end
X2Ability.GetAbilityLevel = function(self, ability) return (ability == ABILITY_FIGHT) and 1 or 0 end
X2Ability.GetSkillPoint = function(self) return 0, 0 end
X2Ability.CanBuyAbilityChange = function(self) return false end
X2Ability.GetAbilityChangeCost = function(self) return "0" end
X2Ability.GetCurrencyForAbilityChange = function(self) return CURRENCY_GOLD end
X2Ability.GetCurrencyForSkillsReset = function(self) return CURRENCY_GOLD end
X2Ability.GetResetSkillsCost = function(self) return "0" end
X2Ability.GetAbilitySlotCount = function(self, kind) return 3 end
X2Ability.GetAbilityFromView = function(self, index) return ABILITY_MAX end
X2Ability.GetNumSkillsByAbility = function(self, ability) return 0 end
X2Ability.GetBuffCount = function(self) return 0 end

-- Vocations (actabilities). A new character has no proficiency in any of them.
X2Ability.GetMaxGrade = function(self) return 6 end
X2Ability.GetGradeInfo = function(self, grade)
  local g = EXPERT_GRADES[grade]
  if g == nil then return nil end
  return { grade = grade, name = g.name, colorString = g.color, gaugeColorString = g.gaugeColor,
           expertLimit = g.expertLimit, advantage = g.advantage, castAdvantage = g.castAdvantage,
           upLimit = g.upLimit, style = grade }
end
X2Ability.GetActabilityCountByGrade = function(self, grade) return 0 end
X2Ability.GetAllMyActabilityInfos = function(self)
  local all = {}
  for _, a in ipairs(ACTABILITIES) do
    if a.visible then
      all[#all + 1] = { type = a.type, name = a.name, grade = 0, point = 0, modifyPoint = 0 }
    end
  end
  return all
end
X2Ability.GetMyActabilityInfo = function(self, type)
  for _, a in ipairs(ACTABILITIES) do
    if a.type == type then return { type = a.type, name = a.name, grade = 0, point = 0, modifyPoint = 0 } end
  end
  return nil
end
X2Ability.GetMinActabilityPoint = function(self, type) return 0 end
X2Ability.GetMaxActabilityPoint = function(self, type) return EXPERT_GRADES[6].upLimit end
X2Ability.CanUpgradeExpert = function(self, type) return false end
X2Ability.CanDowngradeExpert = function(self, type) return false end
X2Ability.CanExpandExpert = function(self) return false end
X2Ability.GetExpandExpertInfo = function(self) return { requireLP = 0, remainExpandCnt = 0 } end
X2Ability.IsLanguageActability = function(self, type)
  for _, a in ipairs(ACTABILITIES) do
    if a.type == type then return a.icon == "ui/icon/craft/language.dds" end
  end
  return false
end
X2Craft.GetActabilityGroupInfoByGroupType = function(self, type)
  for _, a in ipairs(ACTABILITIES) do
    if a.type == type then
      return { type = a.type, name = a.name, icon_path = a.icon, visible = a.visible,
               skill_page_visible = a.skillPageVisible }
    end
  end
  return nil
end
X2Craft.GetActabilityGroupNameByGroupType = function(self, type)
  for _, a in ipairs(ACTABILITIES) do
    if a.type == type then return a.name end
  end
  return nil
end
local craftCategoryIds = {}
for i, c in ipairs(CRAFT_CATEGORIES) do craftCategoryIds[i] = c.type end
X2Craft.GetCategoryType = function(self) return craftCategoryIds end
X2Craft.GetCategoryName = function(self, type)
  for _, c in ipairs(CRAFT_CATEGORIES) do
    if c.type == type then return c.name end
  end
  return ""
end
X2Craft.IsWorkingCraft = function(self) return false end

X2Ability.GetRaceBuffCount = function(self, race, gender) return 0 end

-- X2Player ----------------------------------------------------------------------------------------------------------------
-- Features of this server: no second password, achievements and ranking on, no item lock (the bag has no
-- lock button in reference/ingame/bag).
X2Player.GetFeatureSet = function(self)
  return { dwarfWarborn = false, customsaveload = true, beautyshopBypass = true, achievement = true,
           ranking = true, premium = false, secondpass = false, ingameshopSecondpass = false,
           ingamecashshop = true, hudAuctionButton = true, siege = true, itemSecure = false,
           taxItem = false, backpackProfitShare = false, sensitiveOpeartion = false,
           levelLimit = false, mateLevelLimit = false }
end
X2Player.GetLaborPower = function(self) return P.labor end
X2Player.GetMaxLaborPower = function(self) return P.maxLabor end
X2Player.GetBmPoint = function(self) return P.bm end
X2Player.GetPayMethod = function(self) return "person" end
X2Player.GetPayLocation = function(self) return "person" end
X2Player.GetPremiumServiceOnlineLaborPower = function(self) return 0 end
X2Player.GetPremiumServiceOfflineLaborPower = function(self) return 0 end
-- Experience: percent of the level, exp of the level, exp for the next level.
X2Player.GetExpInfo = function(self) return P.exp, P.expStr, P.levelUpExpStr end
X2Player.GetRecoverableExp = function(self) return "0", 0 end
X2Player.GetScheduleItemList = function(self, onlyActive) return {} end
X2Player.CheckEventScheduleState = function(self) return false end
X2Player.GetAppellationCount = function(self) return 0 end
X2Player.GetShowingAppellation = function(self) return nil end
X2Player.GetBreathTime = function(self) return 0, 0 end
X2Player.IsEnableChat = function(self) return true end
X2Player.GetGamePoints = function(self)
  return { honorPointStr = "0", livingPointStr = "0", crimePoint = 0, crimeRecord = 0 }
end
X2Player.GetMyCash = function(self) return "0" end
X2Player.GetCrimeInfo = function(self) return { crimePoint = 0, crimeRecord = 0 } end
-- Trial (jury) points: none, so the character window has no "Trials Served" row.
X2Player.GetJuryPoint = function(self) return 0 end

X2Time.GetGameTime = function(self) return gameTime.isAm, gameTime.hour, gameTime.minute end

-- World / zone ------------------------------------------------------------------------------------------------------------------
X2World.GetZoneText = function(self) return "Solzreed Peninsula" end
X2World.GetSubZoneText = function(self) return "Desiree Peak" end
X2World.GetZoneFaction = function(self) return 148 end
X2Faction.GetFactionInfo = function(self, id)
  if id == 148 then return { name = "Nuia Alliance" } end
  return nil
end

-- Bag and equipment ---------------------------------------------------------------------------------------------------------------------
X2Bag.CountEmptyBagSlots = function(self) return BAG_CAPACITY - #BAG_ORDER end
X2Bag.GetCurrency = function(self) return CURRENCY_GOLD end
X2Bag.TabInfos = function(self) return {}, {} end
X2Bag.TabCount = function(self) return 1 end -- the "All" tab
X2Bag.FindBagItemByEquipSlot = function(self, a, slot) return {} end
X2Bag.CountItems = function(self) return #BAG_ORDER end
X2Bag.Capacity = function(self) return BAG_CAPACITY end
-- Virtual (tab) slot index -> real bag slot: no tabs, so the same.
X2Bag.IsInvalidSlot = function(self, virtualSlotIdx) return virtualSlotIdx < 1 or virtualSlotIdx > BAG_CAPACITY end
X2Bag.SlotByIdx = function(self, virtualSlotIdx) return virtualSlotIdx end
X2Action.GetActionBarPage = function(self) return 1 end
X2Equipment.DameagedItemCount = function(self, includeBag) return 0 end
-- Tooltip data of an equipped item: nothing is equipped in the fake (empty slot -> bare table).
X2Equipment.GetEquippedItemTooltipText = function(self, unit, slot) return {} end

-- Item groups the bag sorter offers (item_groups with visible_ui = 't', English names from localized_texts).
local ITEM_GROUPS = {
  { 1, "Weapons", "hand-held items (weapons, shields, instruments)" },
  { 2, "Armor", "armor (cloth, leather, plate)" },
  { 3, "Accessories", "accessories (earrings, necklaces, rings)" },
  { 4, "Costume", "specialty clothing" },
  { 5, "Consumables", "potions, food, lunastones, designs" },
  { 6, "Materials", "processed crafting materials" },
  { 7, "Quest", "quest items" },
  { 8, "Transportation", "mounts, vessels, vehicles" },
  { 9, "Pet Gear", "pet equipment" },
  { 12, "Buildings", "building designs" },
  { 13, "Plants", "seeds and seedlings" },
  { 14, "Animals", "farm animals" },
  { 15, "Indoor Decor", "furniture" },
  { 17, "Sundries", "toys and relics" },
  { 18, "Other", "specialty items" },
  { 22, "Gems", "gems and jewels" },
  { 23, "Raw Goods", "raw wood, ore, and rock" },
  { 24, "Machining", "vehicle and glider materials" },
  { 26, "Archeum", "Archeum dust, shards, crystals" },
  { 27, "Currencies", "specialty coins (Gilda Stars, Nui's Tears)" },
  { 28, "Battle Pets", "pets that engage in battle" },
  { 29, "Fishing Supplies", "fishing gear" },
}
local ITEM_GROUP_BY_ID = {}
for _, g in ipairs(ITEM_GROUPS) do ITEM_GROUP_BY_ID[g[1]] = g end
X2Item.GroupTypes = function(self)
  local t = {}
  for i, g in ipairs(ITEM_GROUPS) do t[i] = g[1] end
  return t
end
X2Item.GroupName = function(self, t) local g = ITEM_GROUP_BY_ID[t]; return g and g[2] or "" end
X2Item.GroupDescription = function(self, t) local g = ITEM_GROUP_BY_ID[t]; return g and g[3] or "" end

-- Mail ----------------------------------------------------------------------------------------------------------------------------------------
X2GoodsMail.GetCountUnreadMail = function(self) return 0 end
X2MiaMail.GetCountUnreadMail = function(self) return 0 end
X2Mail.GetCountUnreadMail = function(self) return 0 end

-- Teams, quests, store, ranking, premium service ---------------------------------------------------------------------------------------------
X2Team.GetMaxMembers = function(self) return 50 end
X2Team.GetMaxParties = function(self) return 10 end
X2Team.GetMaxPartyMembers = function(self) return 5 end
X2Team.IsPartyTeam = function(self) return false end
X2Team.IsRaidTeam = function(self) return false end
X2Team.GetTeamDistributorName = function(self) return nil end
X2Team.GetTeamPlayerIndex = function(self) return nil end

X2Quest.GetActiveQuestListCount = function(self) return 0 end
X2Quest.GetActiveQuestType = function(self, index) return nil end
X2Quest.IsTaskQuest = function(self, qtype) return false end
X2Decal.GetMaxDecalCount = function(self) return 24 end

X2Store.GetSellItemsMaxCount = function(self) return 12 end

X2Rank.GetRankCount = function(self) return 4 end
X2Rank.GetRankInfo = function(self, i)
  local names = { "Battlefield", "Fish Length", "Fish Weight" }
  return { name = names[i], v1 = "0", v2 = "0" }
end

X2PremiumService.IsPremiumServiceEnable = function(self) return false end
X2PremiumService.GetPremiumGradePoint = function(self) return { gradePoint = { 0, 1 } } end
X2PremiumService.GetPremiumMaxGrade = function(self) return 2 end

-- Achievements ------------------------------------------------------------------------------------------------------------------------------------
X2Achievement.GetMainCategoryCount = function(self) return #ACHIEVEMENT_SUBCATEGORIES end
X2Achievement.GetSubCategoryCount = function(self, main) return ACHIEVEMENT_SUBCATEGORIES[main] or 0 end
X2Achievement.GetSubcategoryInfo = function(self, main, sub)
  local total = (ACHIEVEMENT_TOTALS[main] or {})[sub] or 0
  return { complete = 0, total = total }
end
X2Achievement.GetAchievementList = function(self, main, sub, filter) return {} end
X2Achievement.GetTracingAchievementCount = function(self) return 0, 5 end
X2Achievement.IsTracingAchievement = function(self, achievementType) return false end
X2Achievement.GetNewCompleteAchievementCount = function(self) return 0 end

-- Specialty (trade goods) info ---------------------------------------------------------------------------------------------------------------------
X2Store.GetProductionZoneGroups = function(self)
  local zones = {}
  for i, z in ipairs(PRODUCTION_ZONES) do zones[i] = { id = z.id, name = z.name } end
  return zones
end
X2Store.GetSellableZoneGroups = function(self, zoneGroup)
  local zones = {}
  for _, z in ipairs(PRODUCTION_ZONES) do
    if z.id ~= zoneGroup then zones[#zones + 1] = { id = z.id, name = z.name } end
  end
  return zones
end

-- Money strings (copper amounts as decimal strings: 1 gold = 100 silver = 10000 copper) --------------------------------------------------------------
local function num(s) return tonumber(s) or 0 end
local function str(n) return string.format("%.0f", n) end
X2Util.GetMyMoneyString = function(self) return P.money end
X2Util.GetMyBankMoneyString = function(self) return "0" end
X2Util.GetMyAAPointString = function(self) return "0" end
X2Util.GetMyBankAAPointString = function(self) return "0" end
local function compare(self, a, b)
  local x, y = num(a), num(b)
  if x < y then return -1 end
  if x > y then return 1 end
  return 0
end
X2Util.CompareMoneyString = compare
X2Util.CompareAAPointString = compare
X2Util.MakeMoneyString = function(self, gold, silver, copper)
  return str(num(gold) * 10000 + num(silver) * 100 + num(copper))
end
X2Util.MakeAAPointString = function(self, points) return str(num(points)) end
X2Util.StrNumericAdd = function(self, a, b) return str(num(a) + num(b)) end
X2Util.StrNumericSub = function(self, a, b) return str(num(a) - num(b)) end
X2Util.MultiplyNumberString = function(self, a, b) return str(num(a) * num(b)) end
X2Util.DivideNumberString = function(self, a, b)
  if num(b) == 0 then return "0" end
  return str(math.floor(num(a) / num(b)))
end
X2Util.NumberToString = function(self, n) return str(n) end
X2Util.InsertComma = function(self, s)
  local sign, digits, rest = string.match(tostring(s), "^(-?)(%d+)(.*)$")
  if digits == nil then return tostring(s) end
  digits = digits:reverse():gsub("(%d%d%d)", "%1,"):reverse()
  if digits:sub(1, 1) == "," then digits = digits:sub(2) end
  return sign .. digits .. rest
end
X2Util.ApplyUIMacroString = function(self, text) return text end
X2Util.UTF8StringLength = function(self, s)
  s = s or ""
  local n = 0
  for i = 1, #s do
    local c = s:byte(i)
    if c < 128 or c >= 192 then n = n + 1 end -- count the lead bytes of UTF-8 characters
  end
  return n
end
X2Util.UTF8StringLimit = function(self, s, limit)
  s = s or ""
  local count, i = 0, 1
  while i <= #s do
    local c = s:byte(i)
    local len = (c >= 240 and 4) or (c >= 224 and 3) or (c >= 192 and 2) or 1
    if count == limit then return s:sub(1, i - 1) end
    count = count + 1
    i = i + len
  end
  return s
end

-- Character stats (the character window): a level 1 Nuian Battlerage character in its starter clothes, the
-- values of reference/ingame/character_info ---------------------------------------------------------------------
local STATS = {
  str = 10, dex = 10, sta = 10, int = 10, spi = 10,
  melee_min_dps = 17, melee_max_dps = 22, melee_dps = 19.42, mainhand_melee_speed = 100, offhand_melee_speed = 100,
  melee_success_rate = 100, melee_critical_rate = 5, melee_critical_bonus = 150, melee_damage_mul = 100,
  melee_parry_rate = 0, ranged_min_dps = 15, ranged_max_dps = 19, ranged_dps = 16.81, ranged_speed = 100,
  ranged_success_rate = 100, ranged_critical_rate = 5, ranged_critical_bonus = 150, ranged_damage_mul = 100,
  spell_dps = 2, spell_success_rate = 100, spell_critical_rate = 5, spell_critical_bonus = 150, spell_damage_mul = 100,
  heal_dps = 2, heal_critical_rate = 5, heal_critical_bonus = 150,
  armor = 269, armor_percentage = 4.83, magic_resist = 377, magic_resist_percentage = 6.64,
  dodge_rate = 3, block_rate = 0, battle_resist = 0, bulls_eye = 0, flexibility = 0,
  casting_time = 100, global_cooldown_mul = 100, move_speed = 5.4,
  health_regen = 5, persistent_health_regen = 5, mana_regen = 4, persistent_mana_regen = 4,
}
X2Unit.UnitInfo = function(self, unit)
  if isPlayer(unit) then return STATS end
  return nil
end
-- Modifiers (buffs, gear): the clothes' armor and magic resistance (the window shows those two green), every
-- other attribute reads 0.
local NO_MODIFIERS = setmetatable({ armor = 245, magic_resist = 357 }, { __index = function() return 0 end })
X2Unit.UnitModifierInfo = function(self, unit)
  if isPlayer(unit) then return NO_MODIFIERS end
  return nil
end
X2Unit.UnitAttr = function(self, unit, attribute) return 0 end

-- Events of entering the world ---------------------------------------------------------------------------------------------------------------------
-- What the client raises when the character has spawned and the loading screen closed, in the
-- order of the capture: the world is entered, the zone chat channels are joined (the "Entering
-- Chat" lines), the server's welcome notice and the labor power gain arrive, and the "Game
-- Controls" tutorial (id 2584, two pages) opens.
FAKE.enter_world = function()
  FAKE.fire_now("ENTERED_WORLD", true)
  FAKE.fire_now("LEFT_LOADING")
  -- CHAT_JOINED_CHANNEL(channel, name)
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_ZONE, "Solzreed Peninsula")
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_TRADE, "Solzreed Peninsula")
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_FIND_PARTY, "Solzreed Peninsula")
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_FACTION, "Nuia Alliance")
  FAKE.fire_now("CHAT_JOINED_CHANNEL", CHAT_RACE, "Nuia Alliance")
  -- CHAT_MESSAGE(channel, unit, isHostile, name, message, speakerInChatBound, specifyName)
  FAKE.fire_now("CHAT_MESSAGE", CHAT_NOTICE, nil, false, "", "Welcome to AAEmu!", false, nil)
  -- LABORPOWER_CHANGED(diff, laborPower): the state already holds the new value.
  FAKE.fire_now("LABORPOWER_CHANGED", 10, P.labor)
  -- TUTORIAL_EVENT(id, pages): page = { { title = "" }, { "<TUTORIAL_TEXT key of the text>" } }
  FAKE.fire_now("TUTORIAL_EVENT", 2584, {
    { { title = "" }, { "tutorial_contents_2584_2_1" } },
    { { title = "" }, { "tutorial_contents_2584_2_2" } },
  })
end
