import { expect, test } from "vitest";
import { entityLabel, supplementaryEntities } from "./entityLabels";

test("labels known German types and safely exposes future native codes", () => {
  expect(entityLabel("PERSON")).toBe("Person");
  expect(entityLabel("FIRSTNAME")).toBe("Vorname");
  expect(entityLabel("BILLING_ACCOUNT")).toBe("BILLING_ACCOUNT");
  expect(supplementaryEntities).toEqual([
    "EMAIL_ADDRESS", "PHONE_NUMBER", "IBAN_CODE", "IP_ADDRESS", "URL", "DATE_TIME",
  ]);
});
