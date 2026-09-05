import { describe, expect, it } from "vitest";
import { stamp, transcriptToText } from "./transcriptText";
import type { TranscriptSegment } from "../types";

const seg = (
  start: number,
  text: string,
  speaker: string | null = null,
): TranscriptSegment => ({
  id: start,
  note_id: "n",
  start_time: start,
  end_time: start + 5,
  text,
  speaker,
  source_type: "live",
  source_id: null,
  created_at: "",
});

describe("stamp", () => {
  it("reads as minutes and seconds for an ordinary meeting", () => {
    expect(stamp(0)).toBe("00:00");
    expect(stamp(754)).toBe("12:34");
  });

  it("adds hours once there are hours", () => {
    // A full day of workshop would otherwise read "312:34", which nobody can
    // place in an eight-hour recording.
    expect(stamp(3600)).toBe("1:00:00");
    expect(stamp(18_754)).toBe("5:12:34");
    expect(stamp(28_800)).toBe("8:00:00");
  });

  it("never renders a negative position", () => {
    expect(stamp(-5)).toBe("00:00");
  });
});

describe("transcriptToText", () => {
  it("groups consecutive lines from one speaker under a single heading", () => {
    // A name on every line is noise on screen and worse in a pasted document.
    const text = transcriptToText([
      seg(0, "Morning everyone.", "You"),
      seg(6, "Let us start with the agenda.", "You"),
      seg(20, "Sounds good.", "Others"),
    ]);
    expect(text).toBe(
      [
        "You  [00:00]",
        "Morning everyone.",
        "Let us start with the agenda.",
        "",
        "Others  [00:20]",
        "Sounds good.",
      ].join("\n"),
    );
  });

  it("leaves the speaker out when nothing is known", () => {
    // Rather than "Unknown", which reads as information and is not.
    const text = transcriptToText([seg(30, "A line with no attribution.")]);
    expect(text).toBe("[00:30]\nA line with no attribution.");
  });

  it("starts a new heading when the speaker changes back", () => {
    const text = transcriptToText([
      seg(0, "First.", "You"),
      seg(10, "Second.", "Others"),
      seg(20, "Third.", "You"),
    ]);
    expect(text.match(/You {2}\[/g)).toHaveLength(2);
  });

  it("skips empty segments rather than leaving gaps", () => {
    const text = transcriptToText([
      seg(0, "Real text.", "You"),
      seg(5, "   ", "You"),
      seg(10, "More text.", "You"),
    ]);
    expect(text).toBe("You  [00:00]\nReal text.\nMore text.");
  });

  it("returns nothing for an empty transcript", () => {
    expect(transcriptToText([])).toBe("");
  });

  it("uses hours in the timestamps of a long recording", () => {
    const text = transcriptToText([seg(18_754, "After lunch.", "You")]);
    expect(text).toContain("[5:12:34]");
  });
});
