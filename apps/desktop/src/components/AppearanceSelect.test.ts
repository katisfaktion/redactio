import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, expect, test, vi } from "vitest";
import { OnyxSelect } from "sit-onyx";
import AppearanceSelect from "./AppearanceSelect.vue";

afterEach(() => {
  vi.restoreAllMocks(); vi.unstubAllGlobals();
  localStorage.clear(); document.documentElement.classList.remove("dark");
});

test("dark appearance is available by default and an explicit choice survives reopening", async () => {
  const wrapper = mount(AppearanceSelect);
  expect(document.documentElement.classList.contains("dark")).toBe(true);
  wrapper.getComponent(OnyxSelect).vm.$emit("update:modelValue", "light");
  await flushPromises();
  expect(document.documentElement.classList.contains("dark")).toBe(false);
  wrapper.unmount();
  const reopened = mount(AppearanceSelect);
  expect(reopened.getComponent(OnyxSelect).props("modelValue")).toBe("light");
  expect(document.documentElement.classList.contains("dark")).toBe(false);
  reopened.unmount();
});

test("system appearance follows Windows changes only when selected", async () => {
  const media = Object.assign(new EventTarget(), { matches: false });
  vi.stubGlobal("matchMedia", () => media);
  const wrapper = mount(AppearanceSelect);
  const select = wrapper.getComponent(OnyxSelect);
  select.vm.$emit("update:modelValue", "auto"); await flushPromises();
  expect(document.documentElement.classList.contains("dark")).toBe(false);
  media.matches = true; media.dispatchEvent(new Event("change")); await flushPromises();
  expect(document.documentElement.classList.contains("dark")).toBe(true);
  select.vm.$emit("update:modelValue", "light"); await flushPromises();
  media.dispatchEvent(new Event("change")); await flushPromises();
  expect(document.documentElement.classList.contains("dark")).toBe(false);
  wrapper.unmount();
  document.documentElement.classList.remove("dark");
  media.dispatchEvent(new Event("change")); await flushPromises();
  expect(document.documentElement.classList.contains("dark")).toBe(false);
});

test("unavailable preference storage does not prevent changing appearance", async () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("unavailable"); });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("unavailable"); });
  const wrapper = mount(AppearanceSelect);
  wrapper.getComponent(OnyxSelect).vm.$emit("update:modelValue", "light"); await flushPromises();
  expect(document.documentElement.classList.contains("dark")).toBe(false);
  wrapper.unmount();
});
