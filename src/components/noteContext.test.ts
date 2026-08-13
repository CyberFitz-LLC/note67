import { describe, expect, it } from "vitest";
import {
  buildNoteContext,
  trimTranscript,
  withNoteContext,
  TRANSCRIPT_BUDGET,
} from "./noteContext";
import type { ActionItem, Summary, TranscriptSegment } from "../types";

const segment = (
  id: number,
  speaker: string | null,
  text: string,
): TranscriptSegment =>
  ({ id, note_id: "n1", start_time: id, end_time: id + 1, text, speaker }) as TranscriptSegment;

const summary = (content: string): Summary =>
  ({
    id: 1,
    note_id: "n1",
    summary_type: "overview",
    content,
    created_at: "2026-08-13T00:00:00Z",
  }) as Summary;

const task = (text: string, done: boolean, assignee?: string): ActionItem =>
  ({
    id: 1,
    note_id: "n1",
    stable_id: "a1",
    text,
    done,
    assignee: assignee ?? null,
    created_at: "2026-08-13T00:00:00Z",
    updated_at: "2026-08-13T00:00:00Z",
  }) as ActionItem;

describe("buildNoteContext", () => {
  it("includes every part of the meeting, not just the note body", () => {
    // The bug: the assistant saw the note body and nothing else, so asking
    // what was decided in a recorded meeting was answered from nothing.
    const context = buildNoteContext({
      title: "Weekly sync",
      description: "My own notes",
      summaries: [summary("We agreed to ship on Friday.")],
      tasks: [task("Follow up with Bob", false)],
      transcript: [segment(1, "Bob Smith", "Shall we ship Friday?")],
    });

    expect(context).toContain("Weekly sync");
    expect(context).toContain("My own notes");
    expect(context).toContain("We agreed to ship on Friday.");
    expect(context).toContain("Follow up with Bob");
    expect(context).toContain("Bob Smith: Shall we ship Friday?");
  });

  it("omits sections that have nothing in them", () => {
    // Empty headings invite a model to invent something to put under them.
    const context = buildNoteContext({ title: "Empty" });
    expect(context).not.toContain("## Summary");
    expect(context).not.toContain("## Tasks");
    expect(context).not.toContain("## Transcript");
  });

  it("puts the distilled parts before the transcript", () => {
    const context = buildNoteContext({
      title: "Weekly sync",
      summaries: [summary("Agreed to ship.")],
      transcript: [segment(1, "Bob", "hello")],
    });
    expect(context.indexOf("## Summary")).toBeLessThan(
      context.indexOf("## Transcript"),
    );
  });

  it("marks a task as done or not", () => {
    const context = buildNoteContext({
      title: "t",
      tasks: [task("Done thing", true), task("Open thing", false, "Walley")],
    });
    expect(context).toContain("- [x] Done thing");
    expect(context).toContain("- [ ] Open thing — Walley");
  });

  it("says when the transcript was cut", () => {
    // A model that silently received half a meeting answers as though it saw
    // all of it — and so does the person reading the answer.
    const long = Array.from({ length: 4000 }, (_, i) =>
      segment(i, "Bob", "a fairly ordinary sentence of meeting speech"),
    );
    const context = buildNoteContext({ title: "Long", transcript: long });
    expect(context).toContain("the earlier part is not included");
  });

  it("does not say it was cut when it was not", () => {
    const context = buildNoteContext({
      title: "Short",
      transcript: [segment(1, "Bob", "brief")],
    });
    expect(context).toContain("## Transcript");
    expect(context).not.toContain("not included");
  });
});

describe("trimTranscript", () => {
  it("keeps everything when it fits", () => {
    const { text, truncated } = trimTranscript([segment(1, "Bob", "hello")]);
    expect(text).toBe("Bob: hello");
    expect(truncated).toBe(false);
  });

  it("keeps the end rather than the beginning", () => {
    // Decisions and actions land late; the opening is arrivals and small talk.
    const segments = [
      segment(1, "Bob", "x".repeat(200)),
      segment(2, "Bob", "the last thing said"),
    ];
    const { text, truncated } = trimTranscript(segments, 60);
    expect(truncated).toBe(true);
    expect(text).toContain("the last thing said");
    expect(text).not.toContain("x".repeat(200));
  });

  it("stays within its budget", () => {
    const segments = Array.from({ length: 500 }, (_, i) =>
      segment(i, "Bob", "some words of meeting speech"),
    );
    expect(trimTranscript(segments, 500).text.length).toBeLessThanOrEqual(500);
  });

  it("handles a segment with no speaker", () => {
    expect(trimTranscript([segment(1, null, "unattributed")]).text).toBe(
      "unattributed",
    );
  });

  it("has a budget that leaves room for the rest of the context", () => {
    expect(TRANSCRIPT_BUDGET).toBeGreaterThan(0);
    expect(TRANSCRIPT_BUDGET).toBeLessThan(50_000);
  });
});

describe("withNoteContext", () => {
  it("puts the question after the context", () => {
    // A model that truncates should lose the end of the transcript rather than
    // the thing it was asked.
    const prompt = withNoteContext("What did we decide?", "# Weekly sync");
    expect(prompt.indexOf("# Weekly sync")).toBeLessThan(
      prompt.indexOf("What did we decide?"),
    );
  });

  it("sends the question alone when there is no context", () => {
    expect(withNoteContext("Hello", "   ")).toBe("Hello");
  });
});
