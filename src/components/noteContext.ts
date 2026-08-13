import type { ActionItem, Summary, TranscriptSegment } from "../types";

/**
 * Everything the assistant should know about the meeting it is being asked
 * about.
 *
 * The assistant previously saw the note body and nothing else — and in chat,
 * not even that: a question went to the model with no context at all, so
 * asking "what did we decide?" about a recorded meeting was answered from
 * nothing.
 */
export interface NoteContextParts {
  title: string;
  description?: string | null;
  summaries?: Summary[];
  tasks?: ActionItem[];
  transcript?: TranscriptSegment[];
}

/**
 * How much of the context may be transcript.
 *
 * A meeting transcript is far larger than everything else combined, and a
 * local model's context is small. Left unbounded it would push out the
 * summary and the tasks — the distilled parts, which are usually the ones
 * worth having.
 */
export const TRANSCRIPT_BUDGET = 12_000;

function speakerLine(s: TranscriptSegment): string {
  return s.speaker ? `${s.speaker}: ${s.text}` : s.text;
}

/**
 * Keep the end of a transcript rather than the beginning.
 *
 * Decisions, actions and conclusions land late in a meeting; the opening is
 * usually arrivals and small talk. When only part fits, the end is the part
 * worth keeping.
 */
export function trimTranscript(
  segments: TranscriptSegment[],
  budget = TRANSCRIPT_BUDGET,
): { text: string; truncated: boolean } {
  const lines = segments.map(speakerLine);
  const full = lines.join("\n");
  if (full.length <= budget) return { text: full, truncated: false };

  const kept: string[] = [];
  let size = 0;
  for (let i = lines.length - 1; i >= 0; i -= 1) {
    const line = lines[i];
    if (size + line.length + 1 > budget) break;
    kept.unshift(line);
    size += line.length + 1;
  }
  return { text: kept.join("\n"), truncated: true };
}

/**
 * Assemble the context block sent with a question.
 *
 * Ordered most-distilled first. If anything is cut it is the transcript, and
 * the block says so — a model that silently received half a meeting will
 * answer as though it saw all of it, and so will the person reading the answer.
 */
export function buildNoteContext(parts: NoteContextParts): string {
  const sections: string[] = [`# ${parts.title || "Untitled meeting"}`];

  if (parts.description?.trim()) {
    sections.push(`## Notes\n${parts.description.trim()}`);
  }

  const summaries = parts.summaries ?? [];
  if (summaries.length > 0) {
    const body = summaries
      .map((s) => `### ${s.summary_type}\n${s.content}`)
      .join("\n\n");
    sections.push(`## Summary\n${body}`);
  }

  const tasks = parts.tasks ?? [];
  if (tasks.length > 0) {
    const body = tasks
      .map((t) => `- [${t.done ? "x" : " "}] ${t.text}${t.assignee ? ` — ${t.assignee}` : ""}`)
      .join("\n");
    sections.push(`## Tasks\n${body}`);
  }

  const transcript = parts.transcript ?? [];
  if (transcript.length > 0) {
    const { text, truncated } = trimTranscript(transcript);
    const heading = truncated
      ? `## Transcript (later part of ${transcript.length} segments; the earlier part is not included)`
      : "## Transcript";
    sections.push(`${heading}\n${text}`);
  }

  return sections.join("\n\n");
}

/**
 * The full prompt for a question about this meeting.
 *
 * The context precedes the question so a model that truncates loses the end of
 * the transcript rather than the thing it was asked.
 */
export function withNoteContext(question: string, context: string): string {
  if (!context.trim()) return question;
  return `${context}\n\n---\n\n${question}`;
}
