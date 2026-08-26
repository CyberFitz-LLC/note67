import {
  fillPercent as fill,
  verdictOf,
  type Verdict,
} from "../utils/audioLevels";

/**
 * One meter per track, side by side.
 *
 * There used to be a single bar, and it only ever measured the microphone —
 * the system-audio path never wrote to it. A flat bar therefore could not tell
 * you whether the far end was silent or whether the meter simply did not watch
 * the far end, and it was always the second. Two bars, each labelled, make that
 * distinguishable at a glance, which is the whole point of a level meter
 * during a call.
 */

const COLOUR: Record<Verdict, string> = {
  silent: "var(--color-text-tertiary)",
  quiet: "#eab308",
  healthy: "var(--color-accent)",
  clipping: "#ef4444",
};

function Meter({
  label,
  rms,
  peak,
  hint,
}: {
  label: string;
  rms: number;
  peak: number;
  hint?: string;
}) {
  const verdict = verdictOf(rms, peak);
  return (
    <div className="flex-1 min-w-0" title={hint}>
      <div className="flex items-center justify-between gap-2 mb-0.5">
        <span
          className="text-[10px] uppercase tracking-wide truncate"
          style={{ color: "var(--color-text-tertiary)" }}
        >
          {label}
        </span>
        {(verdict === "clipping" || verdict === "silent") && (
          <span className="text-[10px]" style={{ color: COLOUR[verdict] }}>
            {verdict === "clipping" ? "too loud" : "nothing"}
          </span>
        )}
      </div>
      <div
        className="h-1 rounded-full overflow-hidden relative"
        style={{ backgroundColor: "rgba(229, 77, 46, 0.2)" }}
      >
        <div
          className="h-full rounded-full transition-all duration-100"
          style={{ width: `${fill(rms)}%`, backgroundColor: COLOUR[verdict] }}
        />
        {/* The held peak, as a tick. A meter polled a few times a second
            misses the transients that matter; this is what makes a brief clip
            visible at all. */}
        {fill(peak) > 0 && (
          <div
            className="absolute top-0 h-full"
            style={{
              left: `calc(${fill(peak)}% - 1px)`,
              width: 2,
              backgroundColor: COLOUR[verdictOf(rms, peak)],
              opacity: 0.9,
            }}
          />
        )}
      </div>
    </div>
  );
}

export function TrackLevelMeters({
  micRms,
  micPeak,
  systemRms,
  systemPeak,
  showMic,
}: {
  micRms: number;
  micPeak: number;
  systemRms: number;
  systemPeak: number;
  /** False in listen-only mode, where there is no microphone to show. */
  showMic: boolean;
}) {
  return (
    <div className="flex-1 flex items-end gap-3 min-w-0">
      {showMic && (
        <Meter
          label="You"
          rms={micRms}
          peak={micPeak}
          hint="Your microphone"
        />
      )}
      <Meter
        label="Others"
        rms={systemRms}
        peak={systemPeak}
        hint="Meeting audio from this computer"
      />
    </div>
  );
}
