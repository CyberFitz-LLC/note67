import { useState } from "react";
import { importApi, type MergeOutcome } from "../api";
import { formatOffset, formatStamp } from "./mergeFormatting";

export function MergeTranscriptPanel({
  noteId,
  onMerged,
}: {
  noteId: string;
  onMerged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<MergeOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);

  const merge = async () => {
    setBusy(true);
    setError(null);
    setOutcome(null);
    try {
      const result = await importApi.selectAndMergeVtt(noteId, "Microsoft Teams");
      if (result) {
        setOutcome(result);
        // Even a refused merge is worth reloading after: it proves nothing
        // changed, and leaving stale segments on screen would suggest it did.
        onMerged();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="p-4 rounded-xl space-y-3"
      style={{ backgroundColor: "var(--color-bg-subtle)" }}
    >
      <div>
        <h4 className="text-sm font-semibold" style={{ color: "var(--color-text)" }}>
          Add another recording of this meeting
        </h4>
        <p
          className="text-sm mt-1"
          style={{ color: "var(--color-text-secondary)" }}
        >
          Teams and Otter know who was speaking, because each participant is on
          their own stream. This note keeps its own text and timings and takes
          only the names.
        </p>
      </div>

      <button
        type="button"
        disabled={busy}
        onClick={merge}
        className="text-sm px-3 py-1.5 rounded-lg disabled:opacity-50"
        style={{ backgroundColor: "var(--color-accent, #3b82f6)", color: "white" }}
      >
        {busy ? "Merging…" : "Choose a transcript…"}
      </button>

      {error && (
        <p className="text-sm" style={{ color: "#ef4444" }}>
          {error}
        </p>
      )}

      {outcome?.rejected && (
        <div className="text-sm" style={{ color: "#eab308" }}>
          <strong>These could not be lined up.</strong>
          <p className="mt-1" style={{ color: "var(--color-text-secondary)" }}>
            Nothing was changed. Merging without an alignment would have
            attributed speech to people who were not here.
          </p>

          {/* The evidence, not just the conclusion. Two recordings started by
              hand overlap partially by nature, and a user needs to be able to
              tell "a different meeting" from "not enough in common to be
              sure". */}
          <p className="mt-2 text-xs" style={{ color: "var(--color-text-tertiary)" }}>
            Compared {outcome.evidence.baseSegments} segments here against{" "}
            {outcome.evidence.otherSegments} in the file.{" "}
            {outcome.evidence.matched === 0
              ? "Nothing in them matched."
              : `${outcome.evidence.matched} passage${
                  outcome.evidence.matched === 1 ? "" : "s"
                } matched, but ${
                  outcome.evidence.agreeing < 3
                    ? "too few agreed on a single point in time"
                    : "they did not agree on a single point in time"
                }.`}
          </p>
          {outcome.evidence.matched > 0 && outcome.evidence.agreeing < 3 && (
            <p className="mt-1 text-xs" style={{ color: "var(--color-text-tertiary)" }}>
              Two recordings only need to share a few minutes to be lined up, so
              a short overlap is fine — but a handful of matching phrases can
              also happen between different meetings, which is what this
              refuses.
            </p>
          )}
        </div>
      )}

      {outcome && !outcome.rejected && (
        <div className="space-y-2">
          <p className="text-sm" style={{ color: "#22c55e" }}>
            Named <strong>{outcome.segmentsNamed}</strong>{" "}
            {outcome.segmentsNamed === 1 ? "segment" : "segments"}
            {outcome.offsetMs !== null && (
              <span style={{ color: "var(--color-text-secondary)" }}>
                {" "}
                — the other recording was {formatOffset(outcome.offsetMs)}{" "}
                against this one
              </span>
            )}
            .
          </p>

          {outcome.segmentsNamed === 0 && (
            <p
              className="text-sm"
              style={{ color: "var(--color-text-secondary)" }}
            >
              The recordings line up, but that source carried no speaker names
              to take.
            </p>
          )}

          {outcome.conflicts.length > 0 && (
            <div>
              <p
                className="text-sm font-medium"
                style={{ color: "var(--color-text)" }}
              >
                {outcome.conflicts.length} disagreement
                {outcome.conflicts.length === 1 ? "" : "s"}
              </p>
              <p
                className="text-xs mb-2"
                style={{ color: "var(--color-text-secondary)" }}
              >
                Kept rather than resolved — one of the two is wrong, and which
                is not something this app can decide.
              </p>
              <ul className="space-y-2">
                {outcome.conflicts.map((c) => (
                  <li
                    key={`${c.startMs}-${c.detail}`}
                    className="text-xs p-2 rounded-lg"
                    style={{ backgroundColor: "var(--color-bg)" }}
                  >
                    <span style={{ color: "var(--color-text-tertiary)" }}>
                      {formatStamp(c.startMs)}
                    </span>{" "}
                    <span style={{ color: "var(--color-text)" }}>{c.text}</span>
                    <div
                      className="mt-1"
                      style={{ color: "var(--color-text-secondary)" }}
                    >
                      {c.detail}
                    </div>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
