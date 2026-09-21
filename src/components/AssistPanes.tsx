import { useEffect, useState } from "react";

import { settingsApi } from "../api";

import {
  freshnessLabel,
  stalenessSeconds,
  type AssistOption,
} from "../hooks/useAssist";

/**
 * The two panes beside a running meeting.
 *
 * A brief of what is being discussed, and options for what to say next. Both
 * state how current they are rather than showing a spinner: a suggestion you
 * act on is only as good as the moment it was made for, and mid-call that is
 * the first thing worth knowing about it.
 */
export function AssistPanes({
  running,
  status,
  statusIsProblem,
  brief,
  questions,
  options,
  raw,
  asOf,
  meetingSeconds,
  receipt,
  attestationNote,
  error,
  onStart,
  onStop,
  onExpand,
}: {
  running: boolean;
  status: string | null;
  statusIsProblem: boolean;
  brief: string | null;
  questions: string[];
  options: AssistOption[];
  raw: string | null;
  asOf: number | null;
  meetingSeconds: number | null;
  receipt: string | null;
  attestationNote: string | null;
  error: string | null;
  onStart: () => void;
  onStop: () => void;
  onExpand: (option: AssistOption) => Promise<string | null>;
}) {
  const [focus, setFocus] = useState("");
  const [focusSaved, setFocusSaved] = useState(true);
  const [expanding, setExpanding] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    settingsApi
      .get("assist_focus")
      .then((value) => {
        if (!cancelled) setFocus(value ?? "");
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);
  const [expanded, setExpanded] = useState<{ label: string; text: string } | null>(
    null,
  );

  const behind = stalenessSeconds(asOf, meetingSeconds);
  const freshness = freshnessLabel(behind);
  const stale = behind !== null && behind >= 120;

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold" style={{ color: "var(--color-text)" }}>
          Live assistance
        </h3>
        <button
          type="button"
          onClick={running ? onStop : onStart}
          className="text-xs px-2 py-1 rounded-lg"
          style={{
            backgroundColor: running
              ? "var(--color-bg-subtle)"
              : "var(--color-accent, #3b82f6)",
            color: running ? "var(--color-text)" : "white",
            border: running ? "1px solid var(--color-border)" : "none",
          }}
        >
          {running ? "Stop" : "Start"}
        </button>
      </div>

      {error && (
        <p className="text-sm" style={{ color: "#ef4444" }}>
          {error}
        </p>
      )}

      {running && (
        <p
          className="text-xs"
          style={{ color: stale ? "#eab308" : "var(--color-text-tertiary)" }}
        >
          {freshness}
          {brief && status && statusIsProblem && ` · ${status}`}
          {receipt && ` · session receipt ${receipt.slice(0, 12)}…`}
        </p>
      )}

      {/* Said plainly rather than hidden: a session running without a receipt
          is a fact worth being able to see, and pretending otherwise is the
          one thing this app must never do. */}
      {running && attestationNote && (
        <p className="text-xs" style={{ color: "#eab308" }}>
          Unattested — {attestationNote}
        </p>
      )}

      {running && (
        <>
          {/* Read fresh on every pass, so a change takes effect on the next one
              rather than the next meeting — which is when someone realises what
              they actually wanted watched for. */}
          <section className="space-y-1">
            <h4
              className="text-[10px] uppercase tracking-wide"
              style={{ color: "var(--color-text-tertiary)" }}
            >
              Focus on
            </h4>
            <div className="flex gap-2">
              <input
                value={focus}
                placeholder="e.g. budget objections, or the migration timeline"
                onChange={(e) => {
                  setFocus(e.target.value);
                  setFocusSaved(false);
                }}
                className="flex-1 min-w-0 p-1.5 rounded-lg text-xs"
                style={{
                  backgroundColor: "var(--color-bg-elevated)",
                  color: "var(--color-text)",
                  border: "1px solid var(--color-border)",
                }}
              />
              <button
                type="button"
                disabled={focusSaved}
                onClick={async () => {
                  await settingsApi.set("assist_focus", focus.trim());
                  setFocusSaved(true);
                }}
                className="text-xs px-2 py-1 rounded-lg disabled:opacity-40"
                style={{
                  backgroundColor: "var(--color-bg-subtle)",
                  border: "1px solid var(--color-border)",
                  color: "var(--color-text)",
                }}
              >
                {focusSaved ? "Set" : "Apply"}
              </button>
            </div>
          </section>

          <section className="space-y-1">
            <h4
              className="text-[10px] uppercase tracking-wide"
              style={{ color: "var(--color-text-tertiary)" }}
            >
              What is being discussed
            </h4>
            {/* Never a bare "Listening…". When there is no brief the pane
                says which kind of nothing it has — an empty transcript, a pass
                in flight, or a model that did not answer and why. Ten minutes
                of the old message told a user none of those. */}
            <p
              className="text-sm whitespace-pre-wrap"
              style={{
                color: brief
                  ? "var(--color-text-secondary)"
                  : statusIsProblem
                    ? "#eab308"
                    : "var(--color-text-tertiary)",
              }}
            >
              {brief ?? status ?? "Starting…"}
            </p>
          </section>

          {questions.length > 0 && (
            <section className="space-y-1">
              <h4
                className="text-[10px] uppercase tracking-wide"
                style={{ color: "var(--color-text-tertiary)" }}
              >
                Asked and not yet answered
              </h4>
              <ul className="text-sm space-y-1" style={{ color: "var(--color-text)" }}>
                {questions.map((q) => (
                  <li key={q}>· {q}</li>
                ))}
              </ul>
            </section>
          )}

          {options.length > 0 && (
            <section className="space-y-2">
              <h4
                className="text-[10px] uppercase tracking-wide"
                style={{ color: "var(--color-text-tertiary)" }}
              >
                You could
              </h4>
              <div className="flex flex-wrap gap-2">
                {options.map((option) => (
                  <button
                    key={option.label}
                    type="button"
                    title={option.angle}
                    disabled={expanding !== null}
                    onClick={async () => {
                      setExpanding(option.label);
                      const text = await onExpand(option);
                      setExpanding(null);
                      if (text) setExpanded({ label: option.label, text });
                    }}
                    className="text-xs px-2 py-1 rounded-lg disabled:opacity-50"
                    style={{
                      backgroundColor: "var(--color-bg-subtle)",
                      border: "1px solid var(--color-border)",
                      color: "var(--color-text)",
                    }}
                  >
                    {expanding === option.label ? "Thinking…" : option.label}
                  </button>
                ))}
              </div>
            </section>
          )}

          {/* A reply that could not be read as options. Shown as prose, never
              as buttons — a fabricated option gets pressed. */}
          {raw && options.length === 0 && (
            <p
              className="text-sm whitespace-pre-wrap"
              style={{ color: "var(--color-text-secondary)" }}
            >
              {raw}
            </p>
          )}

          {expanded && (
            <section
              className="p-3 rounded-lg space-y-1"
              style={{
                backgroundColor: "var(--color-bg-subtle)",
                border: "1px solid var(--color-border)",
              }}
            >
              <div className="flex items-center justify-between">
                <span className="text-xs" style={{ color: "var(--color-text-tertiary)" }}>
                  {expanded.label}
                </span>
                <button
                  type="button"
                  onClick={() => setExpanded(null)}
                  className="text-xs underline"
                  style={{ color: "var(--color-text-tertiary)" }}
                >
                  Dismiss
                </button>
              </div>
              <p
                className="text-sm whitespace-pre-wrap"
                style={{ color: "var(--color-text)" }}
              >
                {expanded.text}
              </p>
            </section>
          )}
        </>
      )}
    </div>
  );
}
