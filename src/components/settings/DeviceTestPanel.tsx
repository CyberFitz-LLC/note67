import {
  meterFraction,
  useDeviceTest,
  type TrackLevel,
} from "../../hooks/useDeviceTest";

const VERDICT_COLOR: Record<TrackLevel["verdict"], string> = {
  silent: "var(--color-text-secondary)",
  quiet: "#eab308",
  healthy: "#22c55e",
  clipping: "#ef4444",
};

function Meter({
  label,
  hint,
  level,
}: {
  label: string;
  hint: string;
  level: TrackLevel | null;
}) {
  const fraction = level ? meterFraction(level.rmsDbfs) : 0;
  const peak = level ? meterFraction(level.peakDbfs) : 0;
  const color = level ? VERDICT_COLOR[level.verdict] : VERDICT_COLOR.silent;

  return (
    <div>
      <div className="flex justify-between items-baseline mb-1">
        <span className="text-sm" style={{ color: "var(--color-text)" }}>
          {label}
        </span>
        <span className="text-xs" style={{ color }}>
          {level ? level.verdict : "—"}
        </span>
      </div>
      <div
        className="h-2.5 rounded-full relative overflow-hidden"
        style={{ backgroundColor: "var(--color-bg-subtle)" }}
      >
        <div
          className="h-full rounded-full transition-all duration-75"
          style={{ width: `${fraction * 100}%`, backgroundColor: color }}
        />
        {/* The held peak, so a brief clip is visible at all. */}
        {peak > 0 && (
          <div
            className="absolute top-0 h-full w-0.5"
            style={{ left: `${peak * 100}%`, backgroundColor: color }}
          />
        )}
      </div>
      <p
        className="text-xs mt-1"
        style={{ color: "var(--color-text-secondary)" }}
      >
        {hint}
      </p>
    </div>
  );
}

export function DeviceTestPanel() {
  const { running, levels, error, start, stop } = useDeviceTest();

  const micHeard =
    levels?.microphone.verdict === "healthy" ||
    levels?.microphone.verdict === "quiet";
  const systemHeard =
    levels?.system.verdict === "healthy" || levels?.system.verdict === "quiet";

  return (
    <div
      className="p-4 rounded-xl space-y-4"
      style={{ backgroundColor: "var(--color-bg-subtle)" }}
    >
      <div>
        <h4 className="text-sm font-semibold" style={{ color: "var(--color-text)" }}>
          Test these devices
        </h4>
        <p
          className="text-sm mt-1"
          style={{ color: "var(--color-text-secondary)" }}
        >
          Runs the same capture a recording does, and throws the audio away.
          Device choices apply when a recording starts, so changing them during
          a meeting has no effect until the next one — check here first.
        </p>
      </div>

      <button
        type="button"
        onClick={() => (running ? stop() : start())}
        className="text-sm px-3 py-1.5 rounded-lg"
        style={{
          backgroundColor: running ? "#ef4444" : "var(--color-accent, #3b82f6)",
          color: "white",
        }}
      >
        {running ? "Stop test" : "Start test"}
      </button>

      {error && (
        <p className="text-sm" style={{ color: "#ef4444" }}>
          {error}
        </p>
      )}

      {running && (
        <div className="space-y-3">
          <Meter
            label="Your microphone"
            hint="Speak. This should move."
            level={levels?.microphone ?? null}
          />
          <Meter
            label="Everyone else (system audio)"
            hint="Should move when others speak — and stay flat when you do."
            level={levels?.system ?? null}
          />

          {levels && !levels.systemAvailable && (
            <p className="text-sm" style={{ color: "#eab308" }}>
              System audio capture did not start, so the other participants
              would not be recorded at all. That is different from it being
              silent — check permissions and that a playback device is selected.
            </p>
          )}

          {levels && levels.systemAvailable && micHeard && systemHeard && (
            <p className="text-sm" style={{ color: "#eab308" }}>
              Both tracks are active. If this happened while only <em>you</em>{" "}
              were speaking, your voice is being mixed into the other track —
              which is what makes a transcript attribute everything to one
              person. See the routing note below.
            </p>
          )}
        </div>
      )}

      <details>
        <summary
          className="text-sm cursor-pointer"
          style={{ color: "var(--color-text-secondary)" }}
        >
          Using VoiceMeeter Banana
        </summary>
        <div
          className="text-sm mt-2 space-y-2"
          style={{ color: "var(--color-text-secondary)" }}
        >
          <p>
            VoiceMeeter <em>consumes</em> what an application plays into it and
            re-emits the mix on its own <strong>recording</strong> devices. So
            listening to what is played to its playback endpoint hears nothing,
            however many of them you try — the audio has already been taken.
          </p>
          <ul className="list-disc ml-5 space-y-1">
            <li>
              <strong>Microphone:</strong> your physical mic, directly.
            </li>
            <li>
              <strong>System audio:</strong> one of the{" "}
              <em>— recording device</em> entries, typically{" "}
              <code>VoiceMeeter Out B1</code> or <code>B2</code>. These sit at
              the bottom of the list.
            </li>
            <li>
              <strong>In VoiceMeeter:</strong> route the virtual input carrying
              the meeting to that bus, and make sure your microphone strip is{" "}
              <em>not</em> routed to it — that is what keeps your own voice off
              the other track.
            </li>
          </ul>
          <p>
            Ordinary speakers are different: for those, pick the playback
            device you actually listen through and the loopback works. The
            distinction only bites with a virtual mixer in the path.
          </p>
        </div>
      </details>
    </div>
  );
}
