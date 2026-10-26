-- Fake game data for the quest windows (L: questcontext/quest_list, the HUD quest notifier and the
-- quest journal panel). Loaded after fake_ingame.lua, whose X2Quest fakes (no active quest) are replaced.
--
-- The scenario is the new level 1 Nuian of the other fakes, in the Solzreed Peninsula (Desireen), with
-- three quests in progress, all from the game database (D:/aaemu-live/rs-game/Data/compact.sqlite3:
-- quest_contexts, quest_components / quest_component_texts, quest_chat_bubbles, quest_act_* for the
-- rewards; English text from localized_texts, @NPC_NAME / @ITEM_NAME / @PC_NAME macros expanded here):
--   2531 The Prophet Terrien   main quest, chapter 1, level 2: talk to the Prophet Terrien in Desireen
--    330 A Friendly Reminder   Solzreed Peninsula, level 1: meet Woodcutter Gallagher in Desireen
--    250 Fox Infestation       Solzreed Peninsula, level 1: kill 3 Solzreed Foxes (reward 110 exp)
-- The client keeps the list state (open folders, ticked quests, the quest shown on the right) in
-- per-character UI data: the folders start open here, like after the quests were accepted. The HUD
-- tracker starts empty (like the spawn capture); ticking a quest in the log tracks it.

AAUI = AAUI or {}

local PLAYER = "Refone"
local CBK_NORMAL, CBK_SYSTEM = _G.CBK_NORMAL or 1, _G.CBK_SYSTEM or 3
local function icon(file) return "ui/icon/" .. file end

-- Quest categories of the journal (quest_categories).
local CATEGORIES = { [3] = "Nuian Main", [4] = "Solzreed Peninsula", [108] = "Prologue" }

-- The quests, in the order they were accepted (the client's active quest list).
local QUESTS = {
  {
    type = 2531, name = "The Prophet Terrien", category = 3, level = 2, main = true, chapter = 1,
    zone = "Solzreed Peninsula",
    summary = "Talk to Prophet Terrien in Desireen.",
    objectives = { { text = "Talk to Prophet Terrien in Desireen.", status = "", done = false } },
    bubbles = {
      { who = "Gossiper Parish", kind = CBK_NORMAL,
        text = "/conclude " .. PLAYER .. ", where have you been? The Prophet Terrien is here, performing miracles for all to see! He knew how many chickens were in the mayor's coop, and he told me I'd soon own a dairy cow!" },
      { who = PLAYER, kind = CBK_NORMAL, text = "/question A fortune teller? Did he say anything about me?" },
      { who = "Gossiper Parish", kind = CBK_NORMAL,
        text = "/disagree What are you, daft? You've got to meet him first! Get on into town and see for yourself. It's amazing!" },
    },
    rewardItems = { { id = 23633, name = "Gilda Star", icon = "icon_item_0476.dds", count = 1 } },
  },
  {
    type = 330, name = "A Friendly Reminder", category = 4, level = 1,
    summary = "Meet Woodcutter Gallagher in Desireen.",
    objectives = { { text = "Meet Woodcutter Gallagher in Desireen.", status = "", done = false } },
    bubbles = {
      { who = "", kind = CBK_SYSTEM, text = "Right-click a nearby NPC to talk to them or press F to speak with the nearest NPC in range." },
      { who = "Maude", kind = CBK_NORMAL,
        text = "/haha Ha! You look like a drowned rat. Were you looking for Seafolk scales again? Honestly, you spend more time getting into trouble than anyone else I know." },
      { who = "Maude", kind = CBK_NORMAL,
        text = "/foldedarms Oh, I came to warn you that Woodcutter Gallagher is on his way here looking for you. You'd better dry off and head up the trail to meet him, or you'll miss your big day!" },
    },
    rewardItems = {},
  },
  {
    type = 250, name = "Fox Infestation", category = 4, level = 1,
    summary = "Skills can be dragged from your skill window to the shortcut bar at the bottom of the screen. Select an enemy and click an offensive skill to attack.",
    objectives = { { text = "Kill the Solzreed Foxes on Desireen Peak. (0/3)", status = "0/3", done = false } },
    bubbles = {
      { who = "", kind = CBK_SYSTEM,
        text = "The Solzreed Foxes are plaguing this area and stealing the fish from our nets. I will reward anyone who helps eliminate these pests. - Gott, Mayor of Desireen." },
    },
    exp = "110",
    rewardItems = {},
  },
}

local BY_TYPE = {}
for i, q in ipairs(QUESTS) do q.index = i; BY_TYPE[q.type] = q end

local function byIndex(idx) return QUESTS[idx] end
local function byType(qtype) return BY_TYPE[qtype] end

-- The list of the active quests --------------------------------------------------------------------------
X2Quest.GetActiveQuestListCount = function(self) return #QUESTS end
X2Quest.GetActiveQuestType = function(self, index)
  local q = QUESTS[index]
  return q and q.type or nil
end
X2Quest.GetActiveQuestTitle = function(self, qtype) local q = byType(qtype); return q and q.name or false end
X2Quest.GetActiveQuestLevel = function(self, idx) local q = byIndex(idx); return q and q.level or 1 end
X2Quest.GetActiveQuestComparedLevel = function(self, idx) return 0 end
-- 1 = in progress, 3 = ready to be reported.
X2Quest.GetActiveQuestListStatus = function(self, idx) return 1 end
X2Quest.GetActiveQuestObjectiveText = function(self, qtype, i)
  local q = byType(qtype)
  local o = q and q.objectives[i]
  return o and o.text or false
end
X2Quest.GetActiveQuestContextConditionMessage = function(self) return "" end
X2Quest.IsMainQuest = function(self, qtype) local q = byType(qtype); return q ~= nil and q.main == true end
X2Quest.IsTaskQuest = function(self, qtype) return false end
X2Quest.IsHiddenQuest = function(self, qtype) return false end
X2Quest.IsGroupQuest = function(self, qtype) return false end
X2Quest.IsSelectiveQuest = function(self, qtype) return false end
X2Quest.IsScoreQuest = function(self, qtype) return false end
X2Quest.IsLivelihoodQuest = function(self, qtype) return false end
X2Quest.IsDailyQuest = function(self, qtype) return false, nil end
X2Quest.IsLetItDoneQuest = function(self, idx) return false end
X2Quest.IsOverDoneQuest = function(self, idx) return false end
X2Quest.IsLetItDoneQuestByType = function(self, qtype) return false end
X2Quest.IsOverDoneQuestByType = function(self, qtype) return false end
X2Quest.IsReadyForCompleteQuest = function(self, qtype) return false end
X2Quest.NowIsAggroComponent = function(self, qtype) return false end
X2Quest.GetObjectiveComponentCount = function(self, qtype)
  local q = byType(qtype)
  return q and #q.objectives or 0
end
X2Quest.GetQuestContextGrade = function(self, qtype) return "normal" end
X2Quest.GetQuestContextLeftTime = function(self, qtype) return 0 end
X2Quest.GetUseTypeQuestItems = function(self, qtype, all) return {} end
X2Quest.GetUseTypeQuestItemsByObjIndex = function(self, qtype, index) return {} end
X2Quest.GetQuestReportNpcTypeByQuestType = function(self, qtype) return nil end
X2Quest.GetQuestReportNpcNameByQuestType = function(self, qtype) return nil end

-- Titles, categories ---------------------------------------------------------------------------------------
X2Quest.GetQuestContextMainTitle = function(self, qtype) local q = byType(qtype); return q and q.name or "" end
X2Quest.GetQuestContextSummary = function(self, qtype) local q = byType(qtype); return q and q.summary or "" end
X2Quest.GetQuestContextBody = function(self, qtype) return "" end
-- The folder of a main quest is named after its chapter (the locale's chapter text) and the zone.
local function chapterText(q)
  local get = locale and locale.questContext and locale.questContext.Get_category
  return (get and get(q.chapter)) or ""
end
X2Quest.GetQuestJournalSubTitleByType = function(self, qtype)
  local q = byType(qtype)
  if q == nil or not q.main then return nil end
  local chapter = chapterText(q)
  return (chapter:sub(-1) == " " and "" or " ") .. (q.zone or "")
end
X2Quest.GetQuestCategoryTypeByQuestType = function(self, qtype) local q = byType(qtype); return q and q.category or 0 end
X2Quest.GetQuestCategoryNameByQuestType = function(self, qtype)
  local q = byType(qtype)
  return q and CATEGORIES[q.category] or ""
end
X2Quest.GetQuestCategoryName = function(self, ctype)
  if ctype == 3 then
    local q = QUESTS[1]
    return chapterText(q) .. X2Quest.GetQuestJournalSubTitleByType(self, q.type)
  end
  return CATEGORIES[ctype] or ""
end
X2Quest.GetQuestContextQuestChapterIdxByType = function(self, qtype) local q = byType(qtype); return q and q.chapter or 0 end

-- The journal on the right ------------------------------------------------------------------------------------
X2Quest.GetQuestJournalTitle = function(self, idx) local q = byIndex(idx); return q and q.name or "" end
X2Quest.GetQuestJournalObjectiveCount = function(self, idx) local q = byIndex(idx); return q and #q.objectives or 0 end
-- The summary of the objective is the paragraph under the title, `status` the progress line.
X2Quest.GetQuestJournalObjectiveText = function(self, idx, k)
  local q = byIndex(idx)
  local o = q and q.objectives[k]
  if o == nil then return nil end
  return { summary = q.summary, status = o.status, done = o.done }
end
X2Quest.GetQuestJournalMainBody = function(self, idx) return "" end
-- { who, text, kind }: what the quest giver said when the quest was accepted.
X2Quest.AcceptBubbleText = function(self, qtype)
  local q = byType(qtype)
  return q and q.bubbles or nil
end

-- Rewards -------------------------------------------------------------------------------------------------------
X2Quest.GetQuestContextRewardExp = function(self, qtype) local q = byType(qtype); return q and q.exp or "0" end
X2Quest.GetQuestContextRewardCopper = function(self, qtype) return 0 end
X2Quest.GetQuestContextRewardAAPoint = function(self, qtype) return 0 end
X2Quest.RewardLaborPower = function(self, qtype) return nil end
X2Quest.RewardHonorPoint = function(self, qtype) return nil end
X2Quest.RewardCrimePoint = function(self, qtype) return nil end
X2Quest.RewardLivingPoint = function(self, qtype) return nil end
X2Quest.GetQuestContextRewardAppellation = function(self, qtype) return nil end
X2Quest.GetQuestContextRewardItemAllCount = function(self, qtype)
  local q = byType(qtype)
  return q and #q.rewardItems or 0
end
X2Quest.GetQuestContextRewardSelectiveItemAllCount = function(self, qtype) return 0 end
X2Quest.GetQuestContextItemTooltip = function(self, qtype, i)
  local q = byType(qtype)
  local it = q and q.rewardItems[i]
  if it == nil then return nil end
  return { name = it.name, itemType = it.id, itemGrade = 1 }
end
X2Quest.GetQuestContextRewardItemCount = function(self, qtype, i)
  local q = byType(qtype)
  local it = q and q.rewardItems[i]
  return it and it.count or 1
end

-- Quest guide decals (the path line on the ground to the quest's goal, `Map` button of the journal): every
-- quest here has a goal on the map, so one can be made; the scripts track which slot shows which quest.
X2Decal.CanMakeGuidedDecal = function(self, qtype) return byType(qtype) ~= nil end
X2Decal.SetQuestGuidDecalByIndex = function(self, decalIndex, qtype, show) end

-- Reward items in slots (AAUI.ItemContent(itemType, grade): see fake_ingame.lua) ---------------------------------
local REWARD_ITEMS = {}
for _, q in ipairs(QUESTS) do
  for _, it in ipairs(q.rewardItems) do REWARD_ITEMS[it.id] = it end
end
local prevItemContent = AAUI.ItemContent
AAUI.ItemContent = function(itemType, grade)
  local content = prevItemContent and prevItemContent(itemType, grade)
  if content ~= nil then return content end
  local it = REWARD_ITEMS[itemType]
  if it == nil then return nil end
  return { type = "item", id = itemType, icon = icon(it.icon), grade = grade or 1 }
end

-- Per-character UI data -------------------------------------------------------------------------------------------
-- The list state (folders, ticks, the quest shown on the right): the folders start open. The script
-- changes the table it gets and saves it with SetQuestContextStateValuesUiData.
local listValues = { folderStates = { [3] = "open", [4] = "open" } }
X2.GetQuestContextStateValuesUiData = function(self) return listValues end
X2.SetQuestContextStateValuesUiData = function(self, values) listValues = values end

-- The HUD notifier (the tracker at the right) lists the quests whose box is ticked in the quest log. The
-- capture the HUD is checked against (reference/ingame/spawn) has an empty tracker, so none is tracked at
-- the start: ticking a quest in the log (the box left of its name) adds it to the tracker.
local notifierValues = { listOpen = true, questList = {} }
X2.GetQuestNotifierListUiData = function(self) return notifierValues end
X2.SetQuestNotifierListUiData = function(self, values) notifierValues = values end
