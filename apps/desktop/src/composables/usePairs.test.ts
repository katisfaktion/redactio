import { ref } from "vue";
import { expect, test } from "vitest";
import type { Settings } from "../lib/contracts";
import { isSelectedPair, usePairs, type PairApi } from "./usePairs";

const settings: Settings = {
  schema_version: 1,
  sync_pairs: [
    { id: "11111111-1111-4111-8111-111111111111", name: "A", source_folder: "A source", target_folder: "A target", created_at: "2026-09-19T10:00:00Z", processing_revision: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", config: { model: "de_core_news_lg", enabled_entities: ["PERSON"], custom_rules: [], include_positions: true } },
    { id: "22222222-2222-4222-8222-222222222222", name: "B", source_folder: "B source", target_folder: "B target", created_at: "2026-09-19T10:00:00Z", processing_revision: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", config: { model: "de_core_news_lg", enabled_entities: ["PERSON"], custom_rules: [], include_positions: true } },
  ],
  selected_sync_pair_id: "22222222-2222-4222-8222-222222222222",
};

test("a pair A update cannot replace pair B's document view", () => {
  const documentView = ref("Dokumente von B");
  const pairs = usePairs(settings);

  pairs.applyPairUpdate("11111111-1111-4111-8111-111111111111", () => {
    documentView.value = "Dokumente von A";
  });

  expect(isSelectedPair(pairs.settings.value.selected_sync_pair_id, "11111111-1111-4111-8111-111111111111")).toBe(false);
  expect(documentView.value).toBe("Dokumente von B");
});

test("a late selection response cannot undo the newest selection", async () => {
  let resolveA!: (settings: Settings) => void;
  const api: PairApi = {
    listPairs: async () => settings,
    addPair: async () => settings,
    renamePair: async () => settings,
    removePair: async () => settings,
    selectPair: (id) => id.startsWith("1111")
      ? new Promise((resolve) => { resolveA = resolve; })
      : Promise.resolve({ ...settings, selected_sync_pair_id: id }),
  };
  const pairs = usePairs(settings, api);

  const oldRequest = pairs.selectPair("11111111-1111-4111-8111-111111111111");
  await pairs.selectPair("22222222-2222-4222-8222-222222222222");
  resolveA({ ...settings, selected_sync_pair_id: "11111111-1111-4111-8111-111111111111" });
  await oldRequest;

  expect(pairs.settings.value.selected_sync_pair_id).toBe("22222222-2222-4222-8222-222222222222");
});
