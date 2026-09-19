import { invoke } from "@tauri-apps/api/core";
import {
  SafeErrorSchema,
  RecoveryPairsSchema,
  ScanReportSchema,
  SettingsSchema,
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

export function safeError(error: unknown): SafeError {
  const parsed = SafeErrorSchema.safeParse(error);
  return parsed.success ? parsed.data : { code: "ipc_error", retryable: false };
}
