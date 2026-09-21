import fixture from "../../../../tests/fixtures/model-management.json";
import {
  CheckedModelSchema,
  ManagedModelSchema,
  ModelJobSchema,
  type CheckedModel,
  type ManagedModel,
  type ModelJob,
} from "../lib/modelContracts";

export function managed(overrides: Partial<ManagedModel> = {}): ManagedModel {
  return ManagedModelSchema.parse({ ...fixture.managed_model, ...overrides });
}

export function checked(overrides: Partial<CheckedModel> = {}): CheckedModel {
  return CheckedModelSchema.parse({ ...fixture.checked_model, ...overrides });
}

export function job(overrides: Partial<ModelJob> = {}): ModelJob {
  return ModelJobSchema.parse({ ...fixture.job, ...overrides });
}
