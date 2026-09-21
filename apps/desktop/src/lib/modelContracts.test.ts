import { expect, test } from "vitest";
import fixture from "../../../../tests/fixtures/model-management.json";
import {
  ArtifactSchema,
  CatalogEntrySchema,
  CheckedModelSchema,
  ManagedModelSchema,
  ModelCatalogSchema,
  ModelDescriptorSchema,
  ModelJobSchema,
  ModelRecordSchema,
  ModelRegistrySchema,
  ModelSourceSchema,
  type Artifact,
  type CheckedModel,
  type ModelDescriptor,
  type ModelJob,
  type ModelRegistry,
} from "./modelContracts";

const revision = "0123456789abcdef0123456789abcdef01234567";
const sha256 = "0123456789abcdef".repeat(4);
const artifact = {
  filename: "model.safetensors",
  size: 1_024,
  upstream_hash: { algorithm: "git-sha1", value: revision },
  sha256,
} satisfies Artifact;
const descriptor = {
  name: "acme-medical-ner",
  version: revision,
  repository: "acme/medical-ner",
  title: "Acme medical NER",
  license: "apache-2.0",
  model_type: "bert",
  architecture: "BertForTokenClassification",
  entity_types: ["DATE", "PERSON"],
  window_tokens: 512,
  stride_tokens: 128,
  special_tokens: 2,
  files: [artifact],
} satisfies ModelDescriptor;
const managed = {
  name: descriptor.name,
  version: descriptor.version,
  repository: descriptor.repository,
  title: descriptor.title,
  license: descriptor.license,
  entity_types: descriptor.entity_types,
  window_tokens: descriptor.window_tokens,
  download_bytes: 1_024,
  installed_bytes: 1_024,
  state: "ready",
  catalog_key: null,
  used_by_pairs: [{ id: "11111111-1111-4111-8111-111111111111", name: "Research" }],
  error: null,
} as const;

test("all model-management variants accept complete payloads and round trip unchanged", () => {
  const registry = {
    schema_version: 2,
    models: [{ descriptor, path: "acme-medical-ner-01234567", state: "ready" }],
    legacy_unavailable: [{ name: "legacy", version: "v1", path: "legacy/models/v1" }],
  } satisfies ModelRegistry;
  const checked = {
    plan_id: "22222222-2222-4222-8222-222222222222",
    model: managed,
  } satisfies CheckedModel;
  const job = {
    job_id: "33333333-3333-4333-8333-333333333333",
    model_name: descriptor.name,
    stage: "downloading",
    downloaded_bytes: 512,
    total_bytes: 1_024,
    error: null,
  } satisfies ModelJob;

  for (const [schema, value] of [
    [ArtifactSchema, artifact],
    [ModelDescriptorSchema, descriptor],
    [CatalogEntrySchema, { key: "biomedbert", directory: "biomedbert", descriptor }],
    [ModelCatalogSchema, { schema_version: 1, models: [{ key: "biomedbert", directory: "biomedbert", descriptor }] }],
    [ModelRecordSchema, registry.models[0]],
    [ModelRegistrySchema, registry],
    [ManagedModelSchema, managed],
    [ModelSourceSchema, { kind: "catalog", key: "hugginglil" }],
    [ModelSourceSchema, { kind: "url", url: "https://huggingface.co/acme/medical-ner" }],
    [ModelSourceSchema, { kind: "receipt", name: descriptor.name }],
    [CheckedModelSchema, checked],
    [ModelJobSchema, job],
  ] as const) expect(schema.parse(value)).toEqual(value);
});

test("the shared cross-language fixture parses without frontend-specific reshaping", () => {
  expect(ArtifactSchema.parse(fixture.artifact)).toEqual(fixture.artifact);
  expect(ModelDescriptorSchema.parse(fixture.descriptor)).toEqual(fixture.descriptor);
  expect(ModelCatalogSchema.parse(fixture.catalog)).toEqual(fixture.catalog);
  expect(CatalogEntrySchema.parse(fixture.catalog_entry)).toEqual(fixture.catalog_entry);
  expect(ModelRegistrySchema.parse(fixture.registry)).toEqual(fixture.registry);
  expect(ManagedModelSchema.parse(fixture.managed_model)).toEqual(fixture.managed_model);
  fixture.sources.forEach(source => expect(ModelSourceSchema.parse(source)).toEqual(source));
  expect(CheckedModelSchema.parse(fixture.checked_model)).toEqual(fixture.checked_model);
  expect(ModelJobSchema.parse(fixture.job)).toEqual(fixture.job);
});

test("strict objects and discriminated unions reject unknown or malformed fields", () => {
  for (const value of [
    { ...artifact, private_path: "/private/model.safetensors" },
    { ...descriptor, files: [{ ...artifact, token: "secret" }] },
    { kind: "catalog", key: "biomedbert", url: "https://example.invalid/model" },
    { kind: "git", url: "https://huggingface.co/acme/medical-ner" },
    { kind: "url", key: "biomedbert" },
  ]) expect(("kind" in value ? ModelSourceSchema : "files" in value ? ModelDescriptorSchema : ArtifactSchema)
    .safeParse(value).success).toBe(false);
});

test("descriptors enforce revisions, supported runtime pairs, entity order, and window stride", () => {
  for (const invalid of [
    { ...descriptor, version: revision.toUpperCase() },
    { ...descriptor, version: "main" },
    { ...descriptor, architecture: "DebertaV2ForTokenClassification" },
    { ...descriptor, entity_types: [] },
    { ...descriptor, entity_types: ["PERSON", "DATE"] },
    { ...descriptor, entity_types: ["PERSON", "PERSON"] },
    { ...descriptor, entity_types: ["LABEL_0"] },
    { ...descriptor, window_tokens: 1, stride_tokens: 1 },
    { ...descriptor, stride_tokens: 127 },
    { ...descriptor, window_tokens: 5, stride_tokens: 2, special_tokens: 3 },
    { ...descriptor, window_tokens: 5, stride_tokens: 1, special_tokens: 4 },
    { ...descriptor, window_tokens: Number.MAX_SAFE_INTEGER + 1 },
  ]) expect(ModelDescriptorSchema.safeParse(invalid).success).toBe(false);

  expect(ModelDescriptorSchema.safeParse({
    ...descriptor,
    model_type: "deberta-v2",
    architecture: "DebertaV2ForTokenClassification",
    special_tokens: null,
  }).success).toBe(true);
});

test("wire titles and artifact arrays remain unconstrained beyond their declared types", () => {
  for (const title of ["", "x".repeat(513), "line\u0000break"]) {
    expect(ModelDescriptorSchema.safeParse({ ...descriptor, title, files: [] }).success).toBe(true);
    expect(ManagedModelSchema.safeParse({ ...managed, title }).success).toBe(true);
  }
});

test("model IDs stay opaque while imported Hugging Face IDs bind to repository and revision", () => {
  expect(ModelDescriptorSchema.safeParse({ ...descriptor, name: "legacy alias 2024" }).success).toBe(true);
  expect(ModelDescriptorSchema.safeParse({
    ...descriptor,
    name: `hf:${descriptor.repository}@${revision}`,
  }).success).toBe(true);
  for (const invalid of [
    { ...descriptor, name: "" },
    { ...descriptor, name: "bad\u0000name" },
    { ...descriptor, name: `hf:${descriptor.repository}@${"f".repeat(40)}` },
    { ...descriptor, repository: "missing-slash" },
    { ...descriptor, repository: "owner/repo/extra" },
    { ...descriptor, repository: "owner/repo--name" },
    { ...descriptor, repository: "owner/repo." },
  ]) expect(ModelDescriptorSchema.safeParse(invalid).success).toBe(false);
});

test("URL sources accept only canonical public Hugging Face repository roots", () => {
  for (const url of [
    "http://huggingface.co/acme/medical-ner",
    "https://user@huggingface.co/acme/medical-ner",
    "https://huggingface.co:443/acme/medical-ner",
    "https://huggingface.co/acme/medical-ner/tree/main",
    "https://huggingface.co/acme/medical-ner?download=1",
    "https://huggingface.co/acme%2Fmedical-ner",
    "https://example.com/acme/medical-ner",
  ]) expect(ModelSourceSchema.safeParse({ kind: "url", url }).success).toBe(false);
  expect(ModelSourceSchema.parse({
    kind: "url", url: "https://huggingface.co/acme/medical-ner/",
  })).toEqual({ kind: "url", url: "https://huggingface.co/acme/medical-ner/" });
});

test("artifact and registry paths stay within their distinct safe boundaries", () => {
  for (const filename of ["../model.safetensors", "dir/model.safetensors", "dir\\model.safetensors", ".hidden", "model.", "CON", "nul.txt"])
    expect(ArtifactSchema.safeParse({ ...artifact, filename }).success).toBe(false);
  for (const path of ["../model", "publisher/model", "C:/model", "CON", "model_1", "Model"])
    expect(ModelRecordSchema.safeParse({ descriptor, path, state: "available" }).success).toBe(false);
  for (const path of ["/legacy/model", "../legacy", "legacy/../model", "legacy\\model", "C:/legacy"])
    expect(ModelRegistrySchema.safeParse({
      schema_version: 2,
      models: [],
      legacy_unavailable: [{ name: "legacy", version: "v1", path }],
    }).success).toBe(false);
});

test("hash formats, byte counts, UUIDs, and future registry versions fail closed", () => {
  for (const invalid of [
    { ...artifact, size: -1 },
    { ...artifact, size: 1.5 },
    { ...artifact, size: Number.MAX_SAFE_INTEGER + 1 },
    { ...artifact, upstream_hash: { algorithm: "git-sha1", value: sha256 } },
    { ...artifact, upstream_hash: { algorithm: "sha256", value: revision } },
    { ...artifact, sha256: revision },
  ]) expect(ArtifactSchema.safeParse(invalid).success).toBe(false);
  expect(ModelRegistrySchema.safeParse({ schema_version: 3, models: [], legacy_unavailable: [] }).success).toBe(false);
  expect(ModelCatalogSchema.safeParse({ schema_version: 2, models: [] }).success).toBe(false);
  expect(CheckedModelSchema.safeParse({ plan_id: "not-a-uuid", model: managed }).success).toBe(false);
  expect(ModelJobSchema.safeParse({
    job_id: "not-a-uuid", model_name: descriptor.name, stage: "queued",
    downloaded_bytes: -1, total_bytes: 0, error: null,
  }).success).toBe(false);
});

test("ready registry records require a local path, tokenizer overhead, and verified files", () => {
  for (const record of [
    { descriptor, path: null, state: "ready" },
    { descriptor: { ...descriptor, special_tokens: null }, path: "acme-model", state: "ready" },
    { descriptor: { ...descriptor, files: [{ ...artifact, sha256: null }] }, path: "acme-model", state: "ready" },
  ]) expect(ModelRecordSchema.safeParse(record).success).toBe(false);

  expect(ModelRecordSchema.safeParse({
    descriptor: { ...descriptor, special_tokens: null, files: [{ ...artifact, sha256: null }] },
    path: null,
    state: "available",
  }).success).toBe(true);
});
