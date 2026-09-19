import { z } from "zod";

export const EntityTypeSchema = z.enum([
  "PERSON",
  "LOCATION",
  "EMAIL_ADDRESS",
  "PHONE_NUMBER",
  "IBAN_CODE",
  "IP_ADDRESS",
  "URL",
  "DATE_TIME",
  "CUSTOM",
]);

const CustomRuleBaseSchema = z.object({
  id: z.uuid(),
  entity_type: EntityTypeSchema,
  enabled: z.boolean(),
}).strict();

export const CustomRuleSchema = z.discriminatedUnion("kind", [
  CustomRuleBaseSchema.extend({ kind: z.literal("regex"), pattern: z.string() }).strict(),
  CustomRuleBaseSchema.extend({ kind: z.literal("words"), words: z.array(z.string()) }).strict(),
]);

export const ProcessingConfigSchema = z.object({
  model: z.string(),
  enabled_entities: z.array(EntityTypeSchema),
  custom_rules: z.array(CustomRuleSchema),
  include_positions: z.boolean(),
}).strict();

export const SyncPairSchema = z.object({
  id: z.uuid(),
  name: z.string(),
  source_folder: z.string(),
  target_folder: z.string(),
  created_at: z.iso.datetime({ offset: true }),
  processing_revision: z.uuid(),
  config: ProcessingConfigSchema,
}).strict();

export const SettingsSchema = z.object({
  schema_version: z.literal(1),
  sync_pairs: z.array(SyncPairSchema),
  selected_sync_pair_id: z.uuid().nullable(),
}).strict();

export const SafeErrorSchema = z.object({
  code: z.string(),
  retryable: z.boolean(),
}).strict();

export const ScanStateSchema = z.enum([
  "new",
  "current",
  "stale",
  "missing-output",
  "conflict",
  "missing-source",
  "recovery-pending",
]);

export const ScannedFileSchema = z.object({
  relative_path: z.string(),
  size_bytes: z.number().int().nonnegative(),
  mtime: z.iso.datetime({ offset: true }),
  source_hash_sha256: z.string().regex(/^[0-9a-f]{64}$/),
  state: ScanStateSchema,
}).strict();

export const ScanFailureSchema = z.object({
  relative_path: z.string(),
  code: z.string(),
}).strict();

export const ScanReportSchema = z.object({
  files: z.array(ScannedFileSchema),
  errors: z.array(ScanFailureSchema),
}).strict();

export type EntityType = z.infer<typeof EntityTypeSchema>;
export type CustomRule = z.infer<typeof CustomRuleSchema>;
export type ProcessingConfig = z.infer<typeof ProcessingConfigSchema>;
export type SyncPair = z.infer<typeof SyncPairSchema>;
export type Settings = z.infer<typeof SettingsSchema>;
export type SafeError = z.infer<typeof SafeErrorSchema>;
export type ScanState = z.infer<typeof ScanStateSchema>;
export type ScannedFile = z.infer<typeof ScannedFileSchema>;
export type ScanFailure = z.infer<typeof ScanFailureSchema>;
export type ScanReport = z.infer<typeof ScanReportSchema>;
