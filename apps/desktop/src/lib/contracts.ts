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
  doc_id: z.string().regex(/^doc-[0-9]{4,}$/).nullable(),
  size_bytes: z.number().int().nonnegative().nullable(),
  mtime: z.iso.datetime({ offset: true }).nullable(),
  source_hash_sha256: z.string().regex(/^[0-9a-f]{64}$/).nullable(),
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

export const RecoveryPairSchema = z.object({
  id: z.uuid(), name: z.string(), source_folder: z.string(), target_folder: z.string(),
  pending_target: z.string().nullable(),
}).strict();
export const RecoveryPairsSchema = z.array(RecoveryPairSchema);
export type RecoveryPair = z.infer<typeof RecoveryPairSchema>;

const RunProgressObject = z.object({
  sync_pair_id: z.uuid(), run_id: z.uuid(),
  stage: z.enum(["initializing", "scanning", "processing", "finished"]),
  discovered: z.int().nonnegative(), processed: z.int().nonnegative(),
  skipped: z.int().nonnegative(), failed: z.int().nonnegative(),
  unprocessed: z.int().nonnegative(), warned: z.int().nonnegative(),
}).strict();
function consistentCounts(counts: z.infer<typeof RunProgressObject>): boolean {
  const completed = counts.processed + counts.skipped + counts.failed;
  return Number.isSafeInteger(completed) && Number.isSafeInteger(completed + counts.unprocessed)
    && completed + counts.unprocessed === counts.discovered && counts.warned <= counts.processed;
}
export const RunProgressSchema = RunProgressObject.refine(consistentCounts);
export const RunSummarySchema = RunProgressObject.extend({
  stage: z.literal("finished"),
  outcome: z.enum(["completed", "completed-with-errors", "cancelled", "failed"]),
  errors: z.array(SafeErrorSchema.extend({ relative_path: z.string() }).strict()),
  error: SafeErrorSchema.nullable(), audit_warning: z.boolean(),
}).strict().refine(consistentCounts);
export type RunProgress = z.infer<typeof RunProgressSchema>;
export type RunSummary = z.infer<typeof RunSummarySchema>;
