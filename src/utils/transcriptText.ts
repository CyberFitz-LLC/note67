import type { TranscriptSegment } from "../types";

/**
 * A transcript as plain text, for pasting somewhere else.
 *
 * Speaker and timestamp are included when they are known and left out when
 * they are not, rather than filled with a placeholder — "Unknown 00:00" reads
 * as information and is not.
 */

/**
 * A position in the recording.
 *
 * Hours appear only once there are hours, so an ordinary meeting reads `12:34`
 * and a full-day workshop reads `5:12:34` rather than `312:34`. Minutes-only
 * formatting is fine until a recording runs past an hour, which this app is
 * about to be asked to do for eight of them.
 */
export function stamp(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** Whether a label says anything about who spoke. */
function named(speaker: string | null): speaker is string {
  return typeof speaker === "string" && speaker.trim().length > 0;
}

/**
 * Render a transcript for the clipboard.
 *
 * Consecutive lines from one speaker are grouped under a single heading, the
 * way the transcript is read on screen — a name repeated on every line is
 * noise, and pasted into a document it is worse.
 */
export function transcriptToText(segments: TranscriptSegment[]): string {
  const lines: string[] = [];
  // `undefined` means no heading has been written yet, which is distinct from
  // `null` meaning "the last line had no speaker". Starting at null made an
  // unattributed first segment match it and lose its timestamp entirely.
  let lastSpeaker: string | null | undefined = undefined;

  for (const segment of segments) {
    const text = segment.text.trim();
    if (!text) continue;

    const speaker = named(segment.speaker) ? segment.speaker.trim() : null;
    const time = stamp(segment.start_time);

    if (speaker !== lastSpeaker) {
      if (lines.length > 0) lines.push("");
      lines.push(speaker ? `${speaker}  [${time}]` : `[${time}]`);
      lastSpeaker = speaker;
    }
    lines.push(text);
  }

  return lines.join("\n");
}
