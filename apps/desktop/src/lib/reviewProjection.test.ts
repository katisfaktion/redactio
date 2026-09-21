import { expect, test } from "vitest";
import type { Detection } from "./contracts";
import { previewSelection, projectReview } from "./reviewProjection";

function detection(id: string, start: number, end: number, entity_type: Detection["entity_type"] = "PERSON",
  confidence: number | null = .9, origin: Detection["origin"] = "automatic"): Detection {
  return { id, start, end, entity_type, confidence, origin, recognizer: origin === "manual" ? "manual" : "test" };
}

test("projects emoji offsets and the Python overlap winner into linked segments", () => {
  const projected = projectReview("🙂 abcdef Z", [
    detection("automatic", 2, 5, "LOCATION", .99),
    detection("bridge", 4, 7, "LOCATION", .5),
    detection("manual", 6, 8, "PERSON", null, "manual"),
  ]);

  expect(projected.text).toBe("🙂 <PERSON_1> Z\n");
  expect(projected.original).toEqual([
    { text: "🙂 ", sourceStart: 0, sourceEnd: 2, start: 0, end: 2, ids: [] },
    { text: "abcdef", sourceStart: 2, sourceEnd: 8, start: 2, end: 8, ids: ["automatic", "bridge", "manual"] },
    { text: " Z", sourceStart: 8, sourceEnd: 10, start: 8, end: 10, ids: [] },
  ]);
  expect(projected.preview).toEqual([
    { text: "🙂 ", sourceStart: 0, sourceEnd: 2, start: 0, end: 2, ids: [] },
    { text: "<PERSON_1>", sourceStart: 2, sourceEnd: 8, start: 2, end: 12, ids: ["automatic", "bridge", "manual"] },
    { text: " Z", sourceStart: 8, sourceEnd: 10, start: 12, end: 14, ids: [] },
    { text: "\n", sourceStart: 10, sourceEnd: 10, start: 14, end: 15, ids: [] },
  ]);
});

test("automatic overlap uses confidence then type name and keeps every contributor ID", () => {
  const high = projectReview("abcdef", [
    detection("low", 0, 4, "PERSON", .2),
    detection("high", 2, 6, "LOCATION", .8),
  ]);
  expect(high.text).toBe("<LOCATION_1>\n");
  expect(high.preview[0]?.ids).toEqual(["low", "high"]);

  const typeTie = projectReview("abcdef", [
    detection("person", 0, 4, "PERSON", .8),
    detection("location", 2, 6, "LOCATION", .8),
  ]);
  expect(typeTie.text).toBe("<LOCATION_1>\n");
});

test("reuses typed counters for NFC-equivalent values with Python whitespace trimming", () => {
  const projected = projectReview("\u0085Café\u0085 Cafe\u0301 \uFEFFCafé\uFEFF", [
    detection("composed", 0, 6),
    detection("decomposed", 7, 12),
    detection("bom", 13, 19),
  ]);

  expect(projected.text).toBe("<PERSON_1> <PERSON_1> <PERSON_2>\n");
});

test("keeps adjacent spans separate with counters scoped by entity type", () => {
  const projected = projectReview("AnnaBerlin", [
    detection("person", 0, 4),
    detection("location", 4, 10, "LOCATION"),
  ]);

  expect(projected.text).toBe("<PERSON_1><LOCATION_1>\n");
  expect(projected.original.map(segment => segment.ids)).toEqual([["person"], ["location"]]);
});

test("maps placeholders atomically while literal placeholder text stays ordinary", () => {
  const projected = projectReview("<PERSON_1> Anna", [detection("anna", 11, 15)]);
  expect(projected.text).toBe("<PERSON_1> <PERSON_1>\n");
  expect(projected.preview[0]?.ids).toEqual([]);
  expect(projected.preview[1]?.ids).toEqual(["anna"]);

  expect(previewSelection(projected.preview, 1, 9)).toEqual({ start: 1, end: 9 });
  expect(previewSelection(projected.preview, 13, 14)).toEqual({ start: 11, end: 15 });
  expect(previewSelection(projected.preview, 9, 13)).toEqual({ start: 9, end: 15 });
  expect(previewSelection(projected.preview, 21, 22)).toBeNull();
  expect(previewSelection(projected.preview, 0, 0)).toBeNull();
  expect(previewSelection(projected.preview, -1, 1)).toBeNull();
  expect(previewSelection(projected.preview, 0, 23)).toBeNull();
});

test("preserves an existing final newline without adding a zero-source segment", () => {
  const projected = projectReview("Anna\n", [detection("anna", 0, 4)]);
  expect(projected.text).toBe("<PERSON_1>\n");
  expect(projected.preview.at(-1)).toEqual({
    text: "\n", sourceStart: 4, sourceEnd: 5, start: 10, end: 11, ids: [],
  });
});
