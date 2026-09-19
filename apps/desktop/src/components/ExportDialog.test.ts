import { flushPromises, mount } from "@vue/test-utils";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import { ref } from "vue";
import { beforeEach, expect, test, vi } from "vitest";
import ExportDialog from "./ExportDialog.vue";

const { open, approved } = vi.hoisted(() => ({ open: vi.fn(), approved: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open }));
vi.mock("../lib/ipc", async (original) => ({ ...await original<object>(), exportApi: { approved } }));
const pair = "11111111-1111-4111-8111-111111111111";
const keys = ["doc-0001", "doc-0002"].map((doc_id) => ({ sync_pair_id: pair, doc_id }));
const result = { sync_pair_id: pair, exported: ["doc-0001"], failed: [{ doc_id: "doc-0002", error: { code: "approval_required", retryable: false } }], error: null, cancelled: false, audit_warning: true };
beforeEach(() => {
  open.mockReset(); approved.mockReset();
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute("open", ""); };
  HTMLDialogElement.prototype.close = function () { this.removeAttribute("open"); };
});
function create() {
  return mount(ExportDialog, { props: { pairId: pair, pairName: "Akten <img src=x>", keys },
    global: { plugins: [createOnyx({ i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } } })] } });
}
test("exports the frozen selection through the native picker and shows exact partial IDs", async () => {
  open.mockResolvedValue("/export"); approved.mockResolvedValue(result);
  const wrapper = create();
  await wrapper.get('[data-testid="export-start"]').trigger("click"); await flushPromises();
  expect(approved).toHaveBeenCalledWith(pair, keys, "/export");
  expect(wrapper.get('[data-testid="exported"]').text()).toContain("doc-0001");
  expect(wrapper.get('[data-testid="failed"]').text()).toContain("doc-0002");
  expect(wrapper.text()).toContain("nicht freigegeben");
  expect(wrapper.text()).toContain("Protokoll");
  expect(wrapper.text()).toContain("Bereits exportierte Dateien bleiben erhalten");
  expect(wrapper.find("img").exists()).toBe(false);
  expect(wrapper.emitted("busy")).toEqual([[true], [false]]);
  wrapper.unmount();
});
test("native picker cancellation is audited and the dialog remains dismissible", async () => {
  open.mockResolvedValue(null); approved.mockResolvedValue({ ...result, exported: [], failed: [], cancelled: true, audit_warning: false });
  const wrapper = create();
  await wrapper.get('[data-testid="export-start"]').trigger("click"); await flushPromises();
  expect(approved).toHaveBeenCalledWith(pair, keys, null);
  expect(wrapper.text()).toContain("abgebrochen");
  await wrapper.get('[data-testid="export-close"]').trigger("click");
  expect(wrapper.emitted("close")).toHaveLength(1);
  expect(approved).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});
test("pending export prevents dismissal and indeterminate writes never trigger automatic retry", async () => {
  let resolve!: (value: unknown) => void;
  open.mockResolvedValue("/export"); approved.mockReturnValue(new Promise((done) => { resolve = done; }));
  const wrapper = create();
  await wrapper.get('[data-testid="export-start"]').trigger("click"); await flushPromises();
  expect(wrapper.get('[data-testid="export-close"]').attributes("disabled")).toBeDefined();
  await wrapper.get("dialog").trigger("cancel");
  expect(wrapper.emitted("close")).toBeUndefined();
  resolve({ ...result, failed: [{ doc_id: "doc-0002", error: { code: "storage_durability_uncertain", retryable: false } }] });
  await flushPromises();
  expect(wrapper.text()).toContain("Datei kann bereits vorhanden sein");
  expect(wrapper.text()).toContain("dauerhafte Speicherung ist nicht bestätigt");
  expect(wrapper.find('[data-testid="export-start"]').exists()).toBe(false);
  expect(approved).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});

test("cancelling before selection retains an audit warning until explicitly dismissed", async () => {
  approved.mockResolvedValue({ ...result, exported: [], failed: [], cancelled: true });
  const wrapper = create();
  await wrapper.get('[data-testid="export-close"]').trigger("click"); await flushPromises();
  expect(open).not.toHaveBeenCalled();
  expect(approved).toHaveBeenCalledWith(pair, keys, null);
  expect(wrapper.emitted("close")).toBeUndefined();
  expect(wrapper.text()).toContain("Protokoll");
  await wrapper.get('[data-testid="export-close"]').trigger("click");
  expect(wrapper.emitted("close")).toHaveLength(1);
  wrapper.unmount();
});
