/**
 * How a track's level reads, mirroring `audio::levels` in Rust.
 *
 * Kept beside the meter rather than inside it so the thresholds can be tested
 * on their own — and because two implementations that drift apart would show a
 * different verdict in the recording meter than the device test panel reports
 * for the same signal.
 */

/** dBFS from a linear amplitude. Mirrors `audio::levels::dbfs`. */
export function dbfs(amplitude: number): number {
  if (amplitude <= 0) return -Infinity;
  return 20 * Math.log10(amplitude);
}

export type Verdict = "silent" | "quiet" | "healthy" | "clipping";

/** Below this a track is carrying nothing. Not zero: a live input never is. */
export const SILENCE_DBFS = -55;
/** Below this, present but quieter than it should be. */
export const QUIET_DBFS = -35;
/** At or above this, peaks are effectively at full scale. */
export const CLIPPING_DBFS = -0.5;

/**
 * Mirrors `audio::levels::verdict`, ordering included.
 *
 * Clipping is judged from the held peak and checked first, because a clipping
 * track can sit at a perfectly ordinary average — which is exactly why it goes
 * unnoticed until the transcript is bad.
 */
export function verdictOf(rms: number, peak: number): Verdict {
  if (dbfs(peak) >= CLIPPING_DBFS) return "clipping";
  const db = dbfs(rms);
  if (db < SILENCE_DBFS) return "silent";
  if (db < QUIET_DBFS) return "quiet";
  return "healthy";
}

/**
 * Where the bar sits, on a dBFS scale rather than a linear one.
 *
 * Linear amplitude spends almost the whole bar in the top few dB and shows
 * nothing at speech level — which is why the old single meter needed a ×400
 * multiplier before it moved at all.
 */
export function fillPercent(amplitude: number): number {
  const db = dbfs(amplitude);
  if (!Number.isFinite(db)) return 0;
  return Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
}
