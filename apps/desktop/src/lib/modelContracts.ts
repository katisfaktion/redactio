import { z } from "zod";
import { EntityTypeSchema, SafeErrorSchema } from "./contracts";

const RevisionSchema = z.string().regex(/^[0-9a-f]{40}$/);
const Sha256Schema = z.string().regex(/^[0-9a-f]{64}$/);
const BytesSchema = z.int().nonnegative();
const OpaqueIdSchema = z.string().min(1).max(512).refine(value => !/[\u0000-\u001f\u007f-\u009f]/.test(value));
const RepositoryComponentSchema = z.string().min(1).max(96)
  .regex(/^[A-Za-z0-9_][A-Za-z0-9_.-]*$/)
  .refine(value => !/[.-]$/.test(value) && !value.includes("..") && !value.includes("--"));
const RepositorySchema = z.string().refine((value) => {
  const components = value.split("/");
  return components.length === 2 && components.every(component => RepositoryComponentSchema.safeParse(component).success);
});

const windowsReserved = /^(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)/i;
const safeFilename = (value: string): boolean => /^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(value)
  && !value.endsWith(".") && !windowsReserved.test(value);
const GeneratedDirectorySchema = z.string().regex(/^[a-z0-9][a-z0-9-]{0,127}$/)
  .refine(value => !windowsReserved.test(value));
const LegacyPathSchema = z.string().min(1).refine(value => {
  if (value.startsWith("/") || value.includes("\\") || /^[A-Za-z]:/.test(value)) return false;
  return value.split("/").every(component => component !== "" && component !== "." && component !== "..");
});

const UpstreamHashSchema = z.discriminatedUnion("algorithm", [
  z.object({ algorithm: z.literal("git-sha1"), value: RevisionSchema }).strict(),
  z.object({ algorithm: z.literal("sha256"), value: Sha256Schema }).strict(),
]);

export const ArtifactSchema = z.object({
  filename: z.string().refine(safeFilename),
  size: BytesSchema,
  upstream_hash: UpstreamHashSchema,
  sha256: Sha256Schema.nullable(),
}).strict();

const ModelEntityTypesSchema = z.array(EntityTypeSchema)
  .min(1)
  .refine(types => types.every((type, index) => index === 0 || types[index - 1] < type))
  .refine(types => types.every(type => !/^LABEL_[0-9]+$/.test(type)));

const ModelDescriptorObject = z.object({
  name: OpaqueIdSchema,
  version: RevisionSchema,
  repository: RepositorySchema,
  title: z.string(),
  license: z.string().nullable(),
  model_type: z.enum(["bert", "deberta-v2"]),
  architecture: z.enum(["BertForTokenClassification", "DebertaV2ForTokenClassification"]),
  entity_types: ModelEntityTypesSchema,
  window_tokens: z.int().min(2),
  stride_tokens: z.int().positive(),
  special_tokens: z.int().nonnegative().nullable(),
  files: z.array(ArtifactSchema),
}).strict();

export const ModelDescriptorSchema = ModelDescriptorObject.superRefine((descriptor, ctx) => {
  const expectedArchitecture = descriptor.model_type === "bert"
    ? "BertForTokenClassification"
    : "DebertaV2ForTokenClassification";
  if (descriptor.architecture !== expectedArchitecture) {
    ctx.addIssue({ code: "custom", path: ["architecture"], message: "Architecture does not match model type" });
  }
  const contentTokens = descriptor.window_tokens - (descriptor.special_tokens ?? 0);
  const expectedStride = Math.min(Math.max(1, Math.floor(descriptor.window_tokens / 4)), contentTokens - 1);
  if (contentTokens < 2 || descriptor.stride_tokens !== expectedStride) {
    ctx.addIssue({ code: "custom", path: ["stride_tokens"], message: "Invalid tokenizer window or stride" });
  }
  if (descriptor.name.startsWith("hf:")
    && descriptor.name !== `hf:${descriptor.repository}@${descriptor.version}`) {
    ctx.addIssue({ code: "custom", path: ["name"], message: "Imported model ID does not match its source" });
  }
});

export const CatalogEntrySchema = z.object({
  key: z.enum(["biomedbert", "hugginglil"]),
  directory: GeneratedDirectorySchema,
  descriptor: ModelDescriptorSchema,
}).strict();

export const ModelCatalogSchema = z.object({
  schema_version: z.literal(1),
  models: z.array(CatalogEntrySchema),
}).strict();

export const ModelRecordSchema = z.object({
  descriptor: ModelDescriptorSchema,
  path: GeneratedDirectorySchema.nullable(),
  state: z.enum(["ready", "available", "removing"]),
}).strict().superRefine((record, ctx) => {
  if (record.state !== "ready") return;
  if (record.path === null) ctx.addIssue({ code: "custom", path: ["path"], message: "Ready model requires a path" });
  if (record.descriptor.special_tokens === null) {
    ctx.addIssue({ code: "custom", path: ["descriptor", "special_tokens"], message: "Ready model requires tokenizer overhead" });
  }
  record.descriptor.files.forEach((file, index) => {
    if (file.sha256 === null) {
      ctx.addIssue({ code: "custom", path: ["descriptor", "files", index, "sha256"], message: "Ready model requires verified files" });
    }
  });
});

export const LegacyEntrySchema = z.object({
  name: OpaqueIdSchema,
  version: z.string(),
  path: LegacyPathSchema,
}).strict();

export const ModelRegistrySchema = z.object({
  schema_version: z.literal(2),
  models: z.array(ModelRecordSchema),
  legacy_unavailable: z.array(LegacyEntrySchema),
}).strict();

const UsedByPairSchema = z.object({ id: z.uuid(), name: z.string() }).strict();
const ManagedModelObject = z.object({
  name: OpaqueIdSchema,
  version: RevisionSchema,
  repository: RepositorySchema,
  title: z.string(),
  license: z.string().nullable(),
  entity_types: ModelEntityTypesSchema,
  window_tokens: z.int().min(2).nullable(),
  download_bytes: BytesSchema,
  installed_bytes: BytesSchema,
  state: z.enum(["available", "ready", "invalid", "removing"]),
  catalog_key: z.enum(["biomedbert", "hugginglil"]).nullable(),
  used_by_pairs: z.array(UsedByPairSchema),
  error: SafeErrorSchema.nullable(),
}).strict();

export const ManagedModelSchema = ManagedModelObject.superRefine((model, ctx) => {
  if (model.name.startsWith("hf:") && model.name !== `hf:${model.repository}@${model.version}`) {
    ctx.addIssue({ code: "custom", path: ["name"], message: "Imported model ID does not match its source" });
  }
});

const HuggingFaceUrlSchema = z.string().refine((value) => {
  const prefix = "https://huggingface.co/";
  if (!value.startsWith(prefix)) return false;
  const repository = value.slice(prefix.length).replace(/\/$/, "");
  return value === `${prefix}${repository}` || value === `${prefix}${repository}/`
    ? RepositorySchema.safeParse(repository).success
    : false;
});

export const ModelSourceSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("catalog"), key: z.enum(["biomedbert", "hugginglil"]) }).strict(),
  z.object({ kind: z.literal("url"), url: HuggingFaceUrlSchema }).strict(),
  z.object({ kind: z.literal("receipt"), name: OpaqueIdSchema }).strict(),
]);

export const CheckedModelSchema = z.object({
  plan_id: z.uuid(),
  model: ManagedModelSchema,
}).strict();

export const ModelJobSchema = z.object({
  job_id: z.uuid(),
  model_name: OpaqueIdSchema,
  stage: z.enum(["downloading", "validating", "ready", "cancelled", "failed", "removing", "removed"]),
  downloaded_bytes: BytesSchema,
  total_bytes: BytesSchema,
  error: SafeErrorSchema.nullable(),
}).strict();

export type Artifact = z.infer<typeof ArtifactSchema>;
export type ModelDescriptor = z.infer<typeof ModelDescriptorSchema>;
export type CatalogEntry = z.infer<typeof CatalogEntrySchema>;
export type ModelCatalog = z.infer<typeof ModelCatalogSchema>;
export type ModelRecord = z.infer<typeof ModelRecordSchema>;
export type LegacyEntry = z.infer<typeof LegacyEntrySchema>;
export type ModelRegistry = z.infer<typeof ModelRegistrySchema>;
export type ManagedModel = z.infer<typeof ManagedModelSchema>;
export type ModelSource = z.infer<typeof ModelSourceSchema>;
export type CheckedModel = z.infer<typeof CheckedModelSchema>;
export type ModelJob = z.infer<typeof ModelJobSchema>;
