import { mount } from "@vue/test-utils";
import { expect, test } from "vitest";
import App from "./App.vue";

test("an empty registry offers setup without a pretend sync action", () => {
  const wrapper = mount(App, { props: {
    initialSettings: { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null },
  } });
  expect(wrapper.text()).toContain("Ordnerpaar hinzufügen");
  expect(wrapper.find('[data-testid="start-sync"]').exists()).toBe(false);
});
