import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { z } from "zod";
import {
  CheckedModelSchema,
  ManagedModelSchema,
  ModelJobSchema,
  ModelSourceSchema,
  type CheckedModel,
  type ManagedModel,
  type ModelJob,
  type ModelSource,
} from "./modelContracts";

export const modelApi = {
  list: async (): Promise<ManagedModel[]> => z.array(ManagedModelSchema).parse(
    await invoke<unknown>("list_managed_models"),
  ),
  check: async (source: ModelSource): Promise<CheckedModel> => CheckedModelSchema.parse(
    await invoke<unknown>("check_model", { source: ModelSourceSchema.parse(source) }),
  ),
  install: async (planId: string): Promise<string> => z.uuid().parse(
    await invoke<unknown>("start_model_install", { planId }),
  ),
  cancel: async (jobId: string): Promise<void> => { await invoke("cancel_model_job", { jobId }); },
  job: async (jobId: string): Promise<ModelJob | null> => ModelJobSchema.nullable().parse(
    await invoke<unknown>("get_model_job", { jobId }),
  ),
  remove: async (name: string): Promise<string> => z.uuid().parse(
    await invoke<unknown>("remove_model", { name }),
  ),
  listen: (receive: (value: unknown) => void) => listen<unknown>(
    "model-progress", event => receive(event.payload),
  ),
};
export type ModelApi = typeof modelApi;
