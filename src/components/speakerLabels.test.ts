import { describe, expect, it } from "vitest";

import { isPlaceholderSpeaker } from "./TranscriptSearch";

/**
 * A diarizer separates voices; it does not identify them. The transcript view
 * has to keep that distinction visible, because the receipt chain makes the
 * same one: `merge::is_generic` in Rust refuses to treat "Speaker 1" as a name,
 * and a receipt over a transcript must not imply the app knows who spoke.
 */
describe("isPlaceholderSpeaker", () => {
  it("recognises a diarizer's placeholders", () => {
    for (const label of ["Speaker 1", "Speaker 2", "Speaker 10", "speaker 3"]) {
      expect(isPlaceholderSpeaker(label), label).toBe(true);
    }
  });

  it("tolerates the spacing and case a service might emit", () => {
    for (const label of ["SPEAKER 1", "Speaker  7", "speaker4", "  Speaker 2  "]) {
      expect(isPlaceholderSpeaker(label), label).toBe(true);
    }
  });

  it("treats a real name as a name", () => {
    // These must stay coloured as a known person, not greyed out as a guess.
    for (const label of ["You", "Me", "John", "Others", "Speaker Jones", "Dr Speaker"]) {
      expect(isPlaceholderSpeaker(label), label).toBe(false);
    }
  });

  it("does not mistake a name that merely contains a number", () => {
    for (const label of ["Room 2", "Agent 47", "Line 1"]) {
      expect(isPlaceholderSpeaker(label), label).toBe(false);
    }
  });
});
