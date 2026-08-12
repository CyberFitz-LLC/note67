/** Formatting for the merge panel.
 *
 * Outside the component file so that file only exports components — the same
 * reason `transcriptGrouping.ts` exists, and what keeps Fast Refresh working.
 */

/** Milliseconds as a signed, human offset. */
export function formatOffset(ms: number): string {
  const sign = ms < 0 ? "−" : "+";
  const total = Math.round(Math.abs(ms) / 1000);
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return minutes > 0 ? `${sign}${minutes}m ${seconds}s` : `${sign}${seconds}s`;
}

/** A segment start as mm:ss, for pointing at a moment in the meeting. */
export function formatStamp(ms: number): string {
  const total = Math.floor(ms / 1000);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}
