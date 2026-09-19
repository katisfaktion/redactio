import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { z } from "zod";
import {
  SafeErrorSchema,
  RecoveryPairsSchema,
  ScanReportSchema,
  SettingsSchema,
  RunSummarySchema,
  ModelInfoSchema,
  DetectionSchema,
  DocumentKeySchema,
  ReviewViewDataSchema,
  SaveReviewSchema,
  ExportSummarySchema,
  type DocumentKey,
  type SaveReview,
  type ProcessingConfig,
  type SafeError,
  type Settings,
} from "./contracts";

async function settingsCommand(command: string, args?: Record<string, unknown>): Promise<Settings> {
  return SettingsSchema.parse(await invoke<unknown>(command, args));
}

export const pairApi = {
  listPairs: () => settingsCommand("list_pairs"),
  recoveryPairs: async () => RecoveryPairsSchema.parse(await invoke<unknown>("list_recovery_pairs")),
  freshStart: async (pairId: string, targetFolder: string, confirmed: boolean): Promise<void> => {
    await invoke("fresh_start_pair", { pairId, targetFolder, confirmed });
  },
  addPair: (name: string, sourceFolder: string, targetFolder: string, createTarget: boolean) =>
    settingsCommand("add_pair", { name, sourceFolder, targetFolder, createTarget }),
  renamePair: (pairId: string, name: string) => settingsCommand("rename_pair", { pairId, name }),
  selectPair: (pairId: string) => settingsCommand("select_pair", { pairId }),
  removePair: (pairId: string) => settingsCommand("remove_pair", { pairId }),
  scanPair: async (pairId: string) => ScanReportSchema.parse(
    await invoke<unknown>("scan_pair", { pairId }),
  ),
};

export const runApi = {
  listen: (receive: (payload: unknown) => void) => listen<unknown>("run-progress", (event) => receive(event.payload)),
  start: async (pairId: string, relativePaths: string[] | null, forceDocIds: string[]) => z.uuid().parse(
    await invoke<unknown>("start_sync", { pairId, relativePaths, forceDocIds })),
  cancel: async (pairId: string, runId: string): Promise<void> => { await invoke("cancel_sync", { pairId, runId }); },
  summary: async (pairId: string, runId: string) => RunSummarySchema.nullable().parse(
    await invoke<unknown>("get_run_summary", { pairId, runId })),
  auditLocation: async () => z.string().parse(await invoke<unknown>("audit_location")),
  openAuditFolder: async (): Promise<void> => { await invoke("open_audit_folder"); },
};

export const detectionApi = {
  listModels: async () => z.array(ModelInfoSchema).parse(await invoke<unknown>("list_models")),
  refresh: (pairId: string) => settingsCommand("refresh_processing_config", { pairId }),
  save: (pairId: string, config: ProcessingConfig) => settingsCommand("save_processing_config", { pairId, config }),
  preview: async (pairId: string, config: ProcessingConfig, text: string) => z.array(DetectionSchema).parse(
    await invoke<unknown>("preview_rules", { pairId, config, text })),
};
export type DetectionApi = typeof detectionApi;

function reviewResult(value: unknown, key: DocumentKey) {
  const result = ReviewViewDataSchema.parse(value);
  if (result.key.sync_pair_id !== key.sync_pair_id || result.key.doc_id !== key.doc_id) {
    throw { code: "ipc_error", retryable: false };
  }
  return result;
}
export const reviewApi = {
  open: async (key: DocumentKey) => {
    const requested = DocumentKeySchema.parse(key);
    return reviewResult(await invoke<unknown>("open_review", { key: requested }), requested);
  },
  save: async (key: DocumentKey, input: SaveReview) => {
    const requested = DocumentKeySchema.parse(key);
    const saved = SaveReviewSchema.parse(input);
    return reviewResult(await invoke<unknown>("save_review", {
      key: requested, expectedOutputHash: saved.expected_output_hash, decisions: saved.decisions,
      status: saved.status, notes: saved.notes, acknowledgedWarnings: saved.acknowledged_warnings,
    }), requested);
  },
};
export type ReviewApi = typeof reviewApi;

export const exportApi = {
  approved: async (pairId: string, keys: DocumentKey[], destination: string | null) => {
    const pair = DocumentKeySchema.shape.sync_pair_id.parse(pairId);
    const requested = z.array(DocumentKeySchema).min(1).parse(keys);
    const ids = requested.map((key) => key.doc_id);
    if (requested.some((key) => key.sync_pair_id !== pair) || new Set(ids).size !== ids.length) {
      throw { code: "invalid_export_selection", retryable: false };
    }
    const result = ExportSummarySchema.parse(await invoke<unknown>("export_approved", {
      pairId: pair, docIds: ids, destination: z.string().min(1).nullable().parse(destination),
    }));
    const returned = [...result.exported, ...result.failed.map((failure) => failure.doc_id)];
    if (result.sync_pair_id !== pair || returned.some((id) => !ids.includes(id))
      || (!result.error && !result.cancelled && returned.length !== ids.length)
      || result.cancelled !== (destination === null)) {
      throw { code: "ipc_error", retryable: false };
    }
    return result;
  },
};
export type ExportApi = typeof exportApi;

export function safeError(error: unknown): SafeError {
  const parsed = SafeErrorSchema.safeParse(error);
  return parsed.success ? parsed.data : { code: "ipc_error", retryable: false };
}
