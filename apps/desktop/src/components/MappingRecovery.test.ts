import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, expect, test, vi } from "vitest";
import MappingRecovery from "./MappingRecovery.vue";
import { pairApi } from "../lib/ipc";
import { confirm, open } from "@tauri-apps/plugin-dialog";

vi.mock("../lib/ipc", () => ({ pairApi: { recoveryPairs: vi.fn(), freshStart: vi.fn(), listPairs: vi.fn() }, safeError: () => ({ code: "recovery_failed", retryable: false }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(), open: vi.fn() }));
const id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const candidate = { id, name: "Sammlung", source_folder: "/source", target_folder: "/old", pending_target: null };
const settings = { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null };

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(pairApi.recoveryPairs).mockResolvedValue([candidate]);
  vi.mocked(open).mockResolvedValue("/fresh");
  vi.mocked(pairApi.listPairs).mockResolvedValue(settings as never);
});

test("native selection and explicit confirmation precede fresh recovery", async () => {
  vi.mocked(confirm).mockResolvedValue(true);
  const wrapper = mount(MappingRecovery); await flushPromises();
  await wrapper.get("button").trigger("click"); await flushPromises();
  expect(open).toHaveBeenCalledWith(expect.objectContaining({ directory: true }));
  expect(confirm).toHaveBeenCalled();
  expect(pairApi.freshStart).toHaveBeenCalledWith(id, "/fresh", true);
  expect(wrapper.emitted("recovered")?.[0]).toEqual([settings]);
});

test("declining the native confirmation preserves the current collection", async () => {
  vi.mocked(confirm).mockResolvedValue(false);
  const wrapper = mount(MappingRecovery); await flushPromises();
  await wrapper.get("button").trigger("click"); await flushPromises();
  expect(pairApi.freshStart).not.toHaveBeenCalled();
});

test("an interrupted recovery offers an explicit retry at the saved destination", async () => {
  vi.mocked(pairApi.recoveryPairs).mockResolvedValue([{ ...candidate, pending_target: "/saved-fresh" }]);
  vi.mocked(confirm).mockResolvedValue(true);
  const wrapper = mount(MappingRecovery); await flushPromises();
  expect(wrapper.text()).toContain("Wiederherstellung fortsetzen");
  await wrapper.get("button").trigger("click"); await flushPromises();
  expect(open).not.toHaveBeenCalled();
  expect(pairApi.freshStart).toHaveBeenCalledWith(id, "/saved-fresh", true);
});
