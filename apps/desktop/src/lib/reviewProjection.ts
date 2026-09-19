import type { Detection } from "./contracts";

export type ReviewSegment = {
  text: string;
  sourceStart: number;
  sourceEnd: number;
  start: number;
  end: number;
  ids: string[];
};

type Union = { start: number; end: number; contributors: Detection[] };

const pythonWhitespace = /^[\u0009-\u000d\u001c-\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+|[\u0009-\u000d\u001c-\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000]+$/gu;

function compare(left: Detection, right: Detection) {
  const manual = Number(left.origin !== "manual") - Number(right.origin !== "manual");
  if (manual) return manual;
  const confidence = (right.confidence ?? -1) - (left.confidence ?? -1);
  if (confidence) return confidence;
  if (left.entity_type !== right.entity_type) return left.entity_type < right.entity_type ? -1 : 1;
  return left.start - right.start || left.end - right.end
    || (left.id === right.id ? 0 : left.id < right.id ? -1 : 1);
}

function sourceBoundaries(text: string, unions: Union[]) {
  const wanted = new Set([0]);
  for (const union of unions) { wanted.add(union.start); wanted.add(union.end); }
  const boundaries = new Map([[0, 0]]);
  let source = 0, native = 0;
  for (const character of text) {
    source++;
    native += character.length;
    if (wanted.has(source)) boundaries.set(source, native);
  }
  boundaries.set(source, native);
  return { boundaries, length: source };
}

export function projectReview(original: string, detections: Detection[]): {
  original: ReviewSegment[];
  preview: ReviewSegment[];
  text: string;
} {
  const active = [...detections].sort((left, right) => left.start - right.start || left.end - right.end
    || (left.id === right.id ? 0 : left.id < right.id ? -1 : 1));
  const unions: Union[] = [];
  for (const detection of active) {
    const previous = unions.at(-1);
    if (!previous || detection.start >= previous.end) {
      unions.push({ start: detection.start, end: detection.end, contributors: [detection] });
    } else {
      previous.end = Math.max(previous.end, detection.end);
      previous.contributors.push(detection);
    }
  }

  const { boundaries, length } = sourceBoundaries(original, unions);
  const originalSegments: ReviewSegment[] = [], preview: ReviewSegment[] = [];
  const placeholders = new Map<string, Map<string, string>>(), counters = new Map<string, number>();
  let cursor = 0, output = 0;
  const add = (start: number, end: number, ids: string[]) => {
    if (start === end) return;
    const text = original.slice(boundaries.get(start)!, boundaries.get(end)!);
    originalSegments.push({ text, sourceStart: start, sourceEnd: end, start, end, ids });
    preview.push({ text, sourceStart: start, sourceEnd: end, start: output, end: output + end - start, ids });
    output += end - start;
  };

  for (const union of unions) {
    add(cursor, union.start, []);
    const ids = union.contributors.map(detection => detection.id);
    const text = original.slice(boundaries.get(union.start)!, boundaries.get(union.end)!);
    originalSegments.push({ text, sourceStart: union.start, sourceEnd: union.end,
      start: union.start, end: union.end, ids });
    const winner = union.contributors.reduce((best, candidate) => compare(candidate, best) < 0 ? candidate : best);
    let byValue = placeholders.get(winner.entity_type);
    if (!byValue) { byValue = new Map(); placeholders.set(winner.entity_type, byValue); }
    const key = text.normalize("NFC").replace(pythonWhitespace, "");
    let placeholder = byValue.get(key);
    if (!placeholder) {
      const counter = (counters.get(winner.entity_type) ?? 0) + 1;
      counters.set(winner.entity_type, counter);
      placeholder = `<${winner.entity_type}_${counter}>`;
      byValue.set(key, placeholder);
    }
    preview.push({ text: placeholder, sourceStart: union.start, sourceEnd: union.end,
      start: output, end: output + placeholder.length, ids });
    output += placeholder.length;
    cursor = union.end;
  }
  add(cursor, length, []);
  if (!preview.at(-1)?.text.endsWith("\n")) {
    preview.push({ text: "\n", sourceStart: length, sourceEnd: length,
      start: output, end: output + 1, ids: [] });
  }
  return { original: originalSegments, preview, text: preview.map(segment => segment.text).join("") };
}

export function previewSelection(segments: ReviewSegment[], start: number, end: number): {
  start: number;
  end: number;
} | null {
  const length = segments.at(-1)?.end ?? 0;
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end)
    || start < 0 || start >= end || end > length) return null;
  let sourceStart: number | undefined, sourceEnd: number | undefined;
  for (const segment of segments) {
    const from = Math.max(start, segment.start), to = Math.min(end, segment.end);
    if (from >= to || segment.sourceStart === segment.sourceEnd) continue;
    const mappedStart = segment.ids.length ? segment.sourceStart : segment.sourceStart + from - segment.start;
    const mappedEnd = segment.ids.length ? segment.sourceEnd : segment.sourceStart + to - segment.start;
    sourceStart = sourceStart === undefined ? mappedStart : Math.min(sourceStart, mappedStart);
    sourceEnd = sourceEnd === undefined ? mappedEnd : Math.max(sourceEnd, mappedEnd);
  }
  return sourceStart === undefined || sourceEnd === undefined || sourceStart >= sourceEnd
    ? null : { start: sourceStart, end: sourceEnd };
}
