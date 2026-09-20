import { z } from "zod";

export const EntityTypeSchema = z.string().regex(/^[A-Z][A-Z0-9_]{0,63}$/);
export const EntityTypesSchema = z.array(EntityTypeSchema).refine(types => new Set(types).size === types.length);

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
  model_entities: EntityTypesSchema.nullable().optional().default(null),
  enabled_entities: EntityTypesSchema,
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

export const ReviewStatusSchema = z.enum(["pending", "approved", "rejected", "needs-rework"]);

export const ScannedFileSchema = z.object({
  relative_path: z.string(),
  doc_id: z.string().regex(/^doc-[0-9]{4,}$/).nullable(),
  size_bytes: z.number().int().nonnegative().nullable(),
  mtime: z.iso.datetime({ offset: true }).nullable(),
  source_hash_sha256: z.string().regex(/^[0-9a-f]{64}$/).nullable(),
  state: ScanStateSchema,
  review_status: ReviewStatusSchema.nullable(),
}).strict().refine(file => file.review_status === null || (file.state === "current" && file.doc_id !== null));

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

export const ModelInfoSchema = z.object({
  name: z.string().min(1), version: z.string().min(1), compatible: z.boolean(),
  entity_types: EntityTypesSchema,
}).strict();
export const DetectionSchema = z.object({
  id: z.string().min(1).max(128), start: z.number().int().nonnegative(), end: z.number().int().positive(),
  entity_type: EntityTypeSchema, confidence: z.number().min(0).max(1).nullable(),
  recognizer: z.string().min(1), origin: z.enum(["automatic", "manual"]),
}).strict().refine(span => span.end > span.start);
export type ModelInfo = z.infer<typeof ModelInfoSchema>;
export type Detection = z.infer<typeof DetectionSchema>;
export type RulePreview = { pairId: string; config: ProcessingConfig; text: string; detections: Detection[] };

export const DocumentKeySchema = z.object({
  sync_pair_id: z.uuid(),
  doc_id: z.string().regex(/^doc-(?!0000$)(?:[0-9]{4}|[1-9][0-9]{4,19})$/),
}).strict();
const HashSchema = z.string().regex(/^[0-9a-f]{64}$/);
const WarningCodesSchema = z.array(z.string().regex(/^[a-z][a-z0-9_]{0,127}$/))
  .refine(codes => new Set(codes).size === codes.length);
export const DecisionsSchema = z.object({
  dismissed_ids: z.array(z.string().min(1).max(128)).refine(ids => new Set(ids).size === ids.length),
  manual: z.array(DetectionSchema.refine(span => span.origin === "manual" && span.confidence === null)),
}).strict().refine(decisions => new Set(decisions.manual.map(span => span.id)).size === decisions.manual.length);
export const OutputEntrySchema = z.object({
  start_offset: z.int().nonnegative(), end_offset: z.int().positive(), entity_type: EntityTypeSchema,
  placeholder: z.string().min(1), confidence: z.number().min(0).max(1).nullable(),
  recognizer: z.string().min(1), origin: z.enum(["automatic", "manual", "merged"]),
}).strict().refine(span => span.end_offset > span.start_offset);
export const SaveReviewSchema = z.object({
  expected_output_hash: HashSchema, expected_review_hash: HashSchema, decisions: DecisionsSchema, status: ReviewStatusSchema,
  notes: z.string(), acknowledged_warnings: WarningCodesSchema,
}).strict();
export const ReviewViewDataSchema = z.object({
  key: DocumentKeySchema, source_hash: HashSchema, revision: z.uuid(), expected_output_hash: HashSchema, expected_review_hash: HashSchema,
  original_text: z.string(), markdown: z.string(), body: z.string(), detections: z.array(DetectionSchema),
  redactions: z.array(OutputEntrySchema), decisions: DecisionsSchema, warnings: WarningCodesSchema,
  acknowledged_warnings: WarningCodesSchema, notes: z.string(), status: ReviewStatusSchema,
}).strict().refine(view => {
  const length = Array.from(view.original_text).length;
  const body = Array.from(view.body);
  const automaticIds = new Set(view.detections.map(span => span.id));
  const allSpans = [...view.detections, ...view.decisions.manual];
  return length <= 1_000_000
    && new Set(allSpans.map(span => span.id)).size === allSpans.length
    && view.detections.every(span => span.origin === "automatic")
    && allSpans.every(span => span.end <= length)
    && view.decisions.dismissed_ids.every(id => automaticIds.has(id))
    && view.redactions.every(span => span.end_offset <= body.length
      && body.slice(span.start_offset, span.end_offset).join("") === span.placeholder)
    && view.acknowledged_warnings.every(code => view.warnings.includes(code))
    && (/[^\p{White_Space}]/u.test(view.original_text) || view.status === "needs-rework")
    && (view.status !== "approved" || (!view.warnings.includes("empty_document")
      && view.warnings.every(code => view.acknowledged_warnings.includes(code))));
});
export type DocumentKey = z.infer<typeof DocumentKeySchema>;

export const ExportSummarySchema = z.object({
  sync_pair_id: DocumentKeySchema.shape.sync_pair_id,
  exported: z.array(DocumentKeySchema.shape.doc_id),
  failed: z.array(z.object({ doc_id: DocumentKeySchema.shape.doc_id, error: SafeErrorSchema }).strict()),
  error: SafeErrorSchema.nullable(),
  cancelled: z.boolean(),
  audit_warning: z.boolean(),
}).strict().refine((summary) => {
  const ids = [...summary.exported, ...summary.failed.map((failure) => failure.doc_id)];
  return new Set(ids).size === ids.length && (!summary.cancelled || ids.length === 0);
}, "Inconsistent export results");
export type ExportSummary = z.infer<typeof ExportSummarySchema>;
export type ReviewStatus = z.infer<typeof ReviewStatusSchema>;
export type Decisions = z.infer<typeof DecisionsSchema>;
export type OutputEntry = z.infer<typeof OutputEntrySchema>;
export type SaveReview = z.infer<typeof SaveReviewSchema>;
export type ReviewViewData = z.infer<typeof ReviewViewDataSchema>;

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
