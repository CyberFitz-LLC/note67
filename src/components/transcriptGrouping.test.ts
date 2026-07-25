import { describe, it, expect } from "vitest";

import { groupTranscriptsBySource } from "./transcriptGrouping";
import type { TranscriptSegment, AudioSegment, UploadedAudio } from "../types";

function seg(
  overrides: Partial<TranscriptSegment> & Pick<TranscriptSegment, "id">
): TranscriptSegment {
  return {
    note_id: "n1",
    start_time: 0,
    end_time: 1,
    text: "hello",
    speaker: "You",
    source_type: null,
    source_id: null,
    created_at: "2026-07-26T00:00:00.000Z",
    ...overrides,
  } as TranscriptSegment;
}

function audioSeg(id: number, order: number): AudioSegment {
  return {
    id,
    note_id: "n1",
    segment_index: order,
    mic_path: null,
    system_path: null,
    start_offset_ms: 0,
    duration_ms: 1000,
    display_order: order,
    created_at: "2026-07-26T00:00:00.000Z",
  } as AudioSegment;
}

describe("groupTranscriptsBySource — live section placement", () => {
  it("puts the live section after existing recordings", () => {
    // Continuing a recording: two finished sessions plus live audio arriving now.
    const segments = [
      seg({ id: 1, source_type: "segment", source_id: 10, text: "first session" }),
      seg({ id: 2, source_type: "segment", source_id: 11, text: "second session" }),
      seg({ id: 3, source_type: "live", text: "being said right now" }),
    ];
    const audio = [audioSeg(10, 0), audioSeg(11, 1)];

    const sections = groupTranscriptsBySource(segments, audio, []);

    expect(sections.map((s) => s.key)).toEqual([
      "segment-10",
      "segment-11",
      "live",
    ]);
  });

  it("keeps live last even when a legacy section is present", () => {
    // Legacy segments (no source info) previously sorted below everything at
    // 1000; live must still come after them.
    const segments = [
      seg({ id: 1, source_type: null, source_id: null, text: "old import" }),
      seg({ id: 2, source_type: "live", text: "live now" }),
    ];

    const sections = groupTranscriptsBySource(segments, [], []);

    expect(sections[sections.length - 1].key).toBe("live");
  });

  it("is the only section during a fresh recording", () => {
    const sections = groupTranscriptsBySource(
      [seg({ id: 1, source_type: "live" })],
      [],
      []
    );

    expect(sections).toHaveLength(1);
    expect(sections[0].key).toBe("live");
  });

  it("still orders finished recordings by display_order", () => {
    const segments = [
      seg({ id: 1, source_type: "segment", source_id: 11 }),
      seg({ id: 2, source_type: "segment", source_id: 10 }),
    ];
    // Declared out of order; display_order decides.
    const audio = [audioSeg(11, 1), audioSeg(10, 0)];

    const sections = groupTranscriptsBySource(segments, audio, []);

    expect(sections.map((s) => s.key)).toEqual(["segment-10", "segment-11"]);
  });

  it("sorts segments within a section by start_time", () => {
    const segments = [
      seg({ id: 1, source_type: "live", start_time: 5, text: "later" }),
      seg({ id: 2, source_type: "live", start_time: 1, text: "earlier" }),
    ];

    const [live] = groupTranscriptsBySource(segments, [], []);

    expect(live.transcripts.map((t) => t.text)).toEqual(["earlier", "later"]);
  });
});

describe("groupTranscriptsBySource — uploads", () => {
  it("places uploads by display_order alongside recordings", () => {
    const upload: UploadedAudio = {
      id: 5,
      note_id: "n1",
      file_path: "/tmp/meeting.m4a",
      original_filename: "meeting.m4a",
      duration_ms: 1000,
      speaker_label: "Others",
      transcription_status: "completed",
      display_order: 0,
      created_at: "2026-07-26T00:00:00.000Z",
    };

    const segments = [
      seg({ id: 1, source_type: "upload", source_id: 5 }),
      seg({ id: 2, source_type: "segment", source_id: 10 }),
      seg({ id: 3, source_type: "live" }),
    ];

    const sections = groupTranscriptsBySource(segments, [audioSeg(10, 1)], [upload]);

    expect(sections.map((s) => s.key)).toEqual(["upload-5", "segment-10", "live"]);
  });
});
