<script setup lang="ts">
import { OnyxAppLayout, OnyxButton, OnyxCard, OnyxHeadline, OnyxInfoCard, OnyxModal, OnyxNavBar, OnyxNavItem, OnyxPageLayout, OnyxSelect } from "sit-onyx";
import { computed, onMounted, onUnmounted, ref, shallowRef, useTemplateRef, watch } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm } from "@tauri-apps/plugin-dialog";
import AppearanceSelect from "./components/AppearanceSelect.vue";
import MappingRecovery from "./components/MappingRecovery.vue";
import DocumentList from "./components/DocumentList.vue";
import PairManager from "./components/PairManager.vue";
import RunPanel from "./components/RunPanel.vue";
import DetectionSettings from "./components/DetectionSettings.vue";
import ReviewView from "./components/ReviewView.vue";
import ExportDialog from "./components/ExportDialog.vue";
import { useReview } from "./composables/useReview";
import { useDetection } from "./composables/useDetection";
import { usePairs } from "./composables/usePairs";
import { useRun } from "./composables/useRun";
import { runApi, safeError } from "./lib/ipc";
import type { DocumentKey, SafeError, ScanReport, Settings } from "./lib/contracts";

const props = withDefaults(defineProps<{
  initialSettings: Settings | null;
  initialError?: SafeError | null;
}>(), { initialError: null });

const startupError = ref(props.initialError);
function recovered(settings: Settings) { pairs.settings.value = settings; startupError.value = null; }

const empty: Settings = { schema_version: 1, sync_pairs: [], selected_sync_pair_id: null };
const pairs = usePairs(props.initialSettings ?? empty);
const detection = useDetection(pairs.selectedPair, (settings) => { pairs.settings.value = settings; });
const view = ref<"documents" | "settings" | "review">("documents");
const navigation = useTemplateRef("navigation");
const review = useReview();
const reviewDocuments = ref<{ value: string; label: string }[]>([]);
const selectedId = computed(() => pairs.selectedPair.value?.id ?? null);
const run = useRun(selectedId);
const documentInvalidation = shallowRef<{ sync_pair_id: string; doc_id?: string } | null>(null);
watch(review.saved, key => { if (key) documentInvalidation.value = { ...key }; });
watch(run.summary, summary => { if (summary) documentInvalidation.value = { sync_pair_id: summary.sync_pair_id }; });
function invalidateDocuments() { scanned.value = false; }
const scanned = ref(false), scanning = ref(false), confirming = ref(false);
function setScanning(value: boolean) { scanning.value = value; }
const exportSelection = shallowRef<{ pairId: string; pairName: string; keys: DocumentKey[] } | null>(null);
const busy = computed(() => scanning.value || !!exportSelection.value || pairs.busy.value || run.busy.value || detection.busy.value || review.busy.value || confirming.value);
const leaveAction = shallowRef<(() => void | Promise<void>) | null>(null);
const closeError = ref("");
function guard(action: () => void | Promise<void>) {
  if (busy.value || leaveAction.value) return;
  if (review.dirty.value) leaveAction.value = action;
  else void action();
}
async function leave(choice: "save" | "discard" | "stay") {
  if (busy.value) return;
  if (choice === "stay") { leaveAction.value = null; return; }
  if (choice === "save" && !await review.save()) return;
  const action = leaveAction.value; leaveAction.value = null;
  review.clear(); await action?.();
}
function changeView(next: "documents" | "settings") {
  navigation.value?.closeMobileMenus();
  if (next === view.value) return;
  guard(() => { review.clear(); view.value = next; if (next === "settings") void loadAuditLocation(); });
}
function changePair(action: () => Promise<void>) {
  guard(async () => { review.clear(); view.value = "documents"; await action(); });
}
function openReview(docId: string) {
  const pairId = selectedId.value;
  if (!pairId || (view.value === "review" && review.key.value?.doc_id === docId)) return;
  guard(async () => { view.value = "review"; await review.open({ sync_pair_id: pairId, doc_id: docId }); });
}
function openExport(keys: DocumentKey[]) {
  const pair = pairs.selectedPair.value;
  if (!pair || !keys.length || keys.some(key => key.sync_pair_id !== pair.id)) return;
  const selection = { pairId: pair.id, pairName: pair.name, keys: keys.map(key => ({ ...key })) };
  guard(() => { review.clear(); view.value = "documents"; exportSelection.value = selection; });
}
function onScanned(report: ScanReport) {
  scanned.value = true;
  reviewDocuments.value = report.files.flatMap(file => file.doc_id && file.state === "current" ? [{ value: file.doc_id, label: `${file.doc_id} · ${file.relative_path}` }] : []);
}
let disposed = false, unlistenClose: (() => void) | undefined;
onMounted(async () => {
  try {
    const unlisten = await getCurrentWindow().onCloseRequested(event => {
      if (busy.value) { event.preventDefault(); closeError.value = "Bitte warten Sie, bis der laufende Vorgang abgeschlossen ist."; return; }
      if (!review.dirty.value && !leaveAction.value) return;
      event.preventDefault();
      guard(async () => {
        try { await getCurrentWindow().destroy(); }
        catch { closeError.value = "Das Fenster konnte nicht geschlossen werden."; }
      });
    });
    if (disposed) unlisten(); else unlistenClose = unlisten;
  } catch { if (!disposed) closeError.value = "Der Schutz beim Schließen ist nicht verfügbar. Speichern Sie Änderungen vor dem Schließen."; }
});
onUnmounted(() => { disposed = true; unlistenClose?.(); leaveAction.value = null; });
const emptyCounts = { discovered: 0, processed: 0, skipped: 0, failed: 0, unprocessed: 0, warned: 0 };
watch(selectedId, () => { scanned.value = false; reviewDocuments.value = []; review.clear(); });
watch(() => pairs.selectedPair.value?.processing_revision, () => { scanned.value = false; reviewDocuments.value = []; });
const auditPath = ref("");
const auditError = ref(false);
async function loadAuditLocation() {
  try { auditPath.value = await runApi.auditLocation(); auditError.value = false; }
  catch { auditError.value = true; }
}
async function openAuditFolder() {
  try { await runApi.openAuditFolder(); auditError.value = false; }
  catch { auditError.value = true; }
}
async function reprocess(files: { relative_path: string; doc_id: string }[]) {
  if (busy.value || !files.length) return;
  const pairId = selectedId.value;
  confirming.value = true;
  try {
    const accepted = await confirm(`Diese Dokumente erneut verarbeiten? Gespeicherte Prüfungen und manuelle Korrekturen werden verworfen:\n\n${files.map((file) => file.relative_path).join("\n")}`, { title: "Erneut verarbeiten", kind: "warning" });
    if (accepted && selectedId.value === pairId) await run.start(files.map((file) => file.relative_path), files.map((file) => file.doc_id));
  } catch (caught) { run.error.value = safeError(caught); }
  finally { confirming.value = false; }
}
const errorText: Record<string, string> = {
  invalid_settings: "Einstellungen konnten nicht geladen werden. Die vorhandene Datei wurde nicht verändert.",
  mapping_recovery_required: "Eine unterbrochene Wiederherstellung muss fortgesetzt werden.",
  invalid_mapping: "Die Zuordnungsdatei ist ungültig und wurde nicht verändert.",
  mapping_missing: "Die Zuordnungsdatei fehlt. Das Ordnerpaar muss repariert werden, bevor es verwendet werden kann.",
  mapping_pair_mismatch: "Die Identität des Ordnerpaars stimmt nicht mit der Zuordnungsdatei überein. Das Ordnerpaar muss repariert werden.",
  mapping_target_mismatch: "Der Zielordner stimmt nicht mit der Zuordnungsdatei überein. Das Ordnerpaar muss repariert werden.",
  invalid_pair_name: "Bitte geben Sie einen Namen ein.",
  duplicate_pair_name: "Dieser Name wird bereits verwendet.",
  target_not_empty: "Der Zielordner ist nicht leer und gehört noch nicht zu dieser Quelle.",
  confirmation_required: "Das Anlegen des Zielordners muss bestätigt werden.",
};
</script>

<template>
  <OnyxAppLayout class="onyx-grid-max-lg onyx-grid-center">
    <template #navBar>
      <OnyxNavBar ref="navigation" app-name="Redactio" mobile="xs" aria-label="Ansichten">
        <template #appArea><span class="brand">Redactio</span></template>
        <OnyxNavItem v-if="!startupError" data-testid="documents-nav" label="Dokumente" :active="view !== 'settings'" :aria-current="view !== 'settings' ? 'page' : undefined" :disabled="busy" @click="changeView('documents')" />
        <OnyxNavItem v-if="!startupError" data-testid="settings-nav" label="Einstellungen" :active="view === 'settings'" :aria-current="view === 'settings' ? 'page' : undefined" :disabled="busy" @click="changeView('settings')" />
        <template #mobileActivePage>{{ view === 'settings' ? 'Einstellungen' : view === 'review' ? 'Dokument prüfen' : 'Dokumente' }}</template>
      </OnyxNavBar>
    </template>
    <OnyxPageLayout>
      <div class="shell" :class="{ 'shell--review': view === 'review' }">
        <header class="app-header">
          <div class="app-title">
            <OnyxHeadline is="h1">{{ view === 'settings' ? 'Einstellungen' : view === 'review' ? 'Dokument prüfen' : 'Dokumente' }}</OnyxHeadline>
            <p v-if="view === 'documents'">Lokal verarbeiten, sorgfältig prüfen und freigegeben exportieren.</p>
          </div>
          <AppearanceSelect class="appearance" />
        </header>

        <OnyxInfoCard v-if="startupError" headline="Einstellungen konnten nicht geladen werden" color="danger" role="alert">
          <p>{{ errorText[startupError.code] ?? "Die lokalen Einstellungen sind derzeit nicht verfügbar." }}</p>
          <MappingRecovery v-if="startupError.code !== 'invalid_settings'" @recovered="recovered" />
        </OnyxInfoCard>

        <template v-else>
          <OnyxInfoCard v-if="pairs.error.value" color="danger" role="alert">
            {{ errorText[pairs.error.value.code] ?? "Die Änderung konnte nicht gespeichert werden." }}
          </OnyxInfoCard>
          <PairManager
            :settings="pairs.settings.value"
            :busy="busy"
            @add="(name, source, target, createTarget) => changePair(() => pairs.addPair(name, source, target, createTarget))"
            @rename="pairs.renamePair"
            @select="id => id !== selectedId && changePair(() => pairs.selectPair(id))"
            @remove="id => changePair(() => pairs.removePair(id))"
          >
            <template #context><small v-if="view === 'review'">Prüfung: {{ review.key.value?.doc_id }}</small></template>
          </PairManager>
          <div v-if="view === 'review' && pairs.selectedPair.value" class="view-switcher">
            <OnyxSelect v-if="view === 'review' && pairs.selectedPair.value" class="review-picker" data-testid="review-document-select" label="Dokument prüfen" list-label="Verarbeitete Dokumente" :options="reviewDocuments" :model-value="review.key.value?.doc_id" :hide-clear-icon="true" :disabled="busy" @update:model-value="id => typeof id === 'string' && openReview(id)" />
          </div>
          <p v-if="closeError" role="alert">{{ closeError }}</p>
          <template v-if="view === 'review' && pairs.selectedPair.value">
            <p v-if="scanning" role="status">Dokumentliste wird aktualisiert …</p>
            <ReviewView :key="`${review.key.value?.sync_pair_id}:${review.key.value?.doc_id}`" :review="review" :pair-name="pairs.selectedPair.value.name" :disabled="busy" @back="changeView('documents')" />
          </template>
          <p v-if="view === 'documents' && detection.error.value" role="alert">Die Erkennung ist derzeit nicht verfügbar. Prüfen Sie die Modelle und Regeln unter Einstellungen.</p>
          <OnyxCard v-if="pairs.selectedPair.value" v-show="view === 'documents'" data-testid="document-view" role="region" aria-labelledby="documents-heading">
            <div class="section-heading">
              <OnyxHeadline id="documents-heading" is="h2">Dokumentübersicht</OnyxHeadline>
              <p>Ordner einlesen, Dokumente verarbeiten und anschließend direkt in der Liste prüfen.</p>
            </div>
            <DocumentList :key="`${pairs.selectedPair.value.id}:${pairs.selectedPair.value.processing_revision}`" :pair-id="pairs.selectedPair.value.id" :invalidation="documentInvalidation" :disabled="busy || !!leaveAction" @busy="setScanning" @invalidated="invalidateDocuments" @scanned="onScanned" @reprocess="reprocess" @review="openReview" @export="openExport">
              <RunPanel
                :pair-name="pairs.selectedPair.value.name"
                :counts="run.progress.value ?? emptyCounts"
                :stage="run.progress.value?.stage ?? (run.busy.value ? 'initializing' : null)"
                :outcome="run.summary.value?.outcome"
                :cancelling="run.cancelling.value"
                :disabled="busy || !scanned"
                :error="run.error.value"
                :errors="run.summary.value?.errors"
                :audit-warning="run.summary.value?.audit_warning"
                @start="run.start()" @cancel="run.cancel()"
              />
              <OnyxButton v-if="run.summary.value?.errors.length" label="Fehlgeschlagene Dokumente erneut versuchen" type="button" :disabled="busy" @click="run.start(run.summary.value.errors.map((failure) => failure.relative_path))" />
            </DocumentList>
          </OnyxCard>
          <section v-if="view === 'settings'" class="settings" aria-label="Einstellungen">
            <DetectionSettings v-if="pairs.selectedPair.value" :pair="pairs.selectedPair.value" :models="detection.models.value" :busy="busy"
              :preview="detection.result.value" :error="detection.error.value" :saved="detection.saved.value"
              @save="detection.save" @preview="detection.preview" />
            <OnyxCard class="audit-settings" role="region" aria-labelledby="audit-heading">
              <OnyxHeadline id="audit-heading" is="h2">Protokoll</OnyxHeadline>
              <p>{{ auditPath }}</p>
              <OnyxButton label="Protokollordner öffnen" type="button" @click="openAuditFolder" />
              <p v-if="auditError" role="alert">Der Protokollordner ist derzeit nicht verfügbar.</p>
            </OnyxCard>
          </section>
        </template>
        <ExportDialog v-if="exportSelection" :pair-id="exportSelection.pairId" :pair-name="exportSelection.pairName" :keys="exportSelection.keys" @close="exportSelection = null" />
        <OnyxModal label="Ungespeicherte Prüfung" :open="!!leaveAction" :alert="true" @update:open="open => !open && leave('stay')">
          <div class="leave-dialog">
            <h2>Ungespeicherte Prüfung</h2>
            <p>Änderungen vor dem Verlassen speichern oder verwerfen?</p>
            <p v-if="review.error.value" role="alert">Speichern fehlgeschlagen. Ihre Änderungen bleiben erhalten.</p>
            <div class="view-navigation">
              <OnyxButton data-testid="leave-save" label="Speichern und fortfahren" type="button" :disabled="busy" @click="leave('save')" />
              <OnyxButton data-testid="leave-discard" label="Änderungen verwerfen" type="button" mode="outline" :disabled="busy" @click="leave('discard')" />
              <OnyxButton data-testid="leave-stay" label="Hier bleiben" type="button" mode="outline" :disabled="busy" @click="leave('stay')" />
            </div>
          </div>
        </OnyxModal>
      </div>
    </OnyxPageLayout>
  </OnyxAppLayout>
</template>

<style scoped>
.shell { display: grid; align-content: start; gap: var(--onyx-spacing-lg); min-height: 100%; min-width: 0; }
.brand { font-size: var(--onyx-font-size-lg); font-weight: var(--onyx-font-weight-semibold); color: var(--onyx-color-text-icons-primary-intense); }
.app-header { display: flex; justify-content: space-between; align-items: flex-end; flex-wrap: wrap; gap: var(--onyx-spacing-lg); }
.app-title, .section-heading { display: grid; gap: var(--onyx-spacing-xs); }
.app-title { flex: 1 1 24rem; }
.app-title p { color: var(--onyx-color-text-icons-neutral-medium); }
.appearance { flex: 0 1 12rem; }
.shell--review { gap: var(--onyx-spacing-lg); }
.shell--review .app-title { flex-basis: 12rem; }
.view-switcher { display: flex; flex-wrap: wrap; align-items: flex-end; gap: var(--onyx-spacing-md) var(--onyx-spacing-lg); }
.review-picker { flex: 1 1 24rem; min-width: 0; }
h1, h2, h3, p { margin: 0; }
[data-testid="document-view"], .audit-settings { display: grid; gap: var(--onyx-spacing-lg); min-width: 0; }
.section-heading p { color: var(--onyx-color-text-icons-neutral-medium); }
.view-navigation { display: flex; flex-wrap: wrap; gap: var(--onyx-spacing-sm); }
.leave-dialog { display: grid; gap: var(--onyx-spacing-md); padding: var(--onyx-spacing-lg); max-width: 36rem; }
.settings { display: grid; gap: var(--onyx-spacing-xl); }
.audit-settings p { overflow-wrap: anywhere; }
.audit-settings > :deep(.onyx-button) { justify-self: start; }
:deep(.pair-manager) { align-self: start; }
</style>
