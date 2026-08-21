import { useEffect, useState } from "react";
import { exochainApi, type Verification } from "../api";

/**
 * Whether this transcript is still the one that was recorded.
 *
 * Checked on open rather than behind a button. A tamper check nobody presses
 * catches nothing, and the whole value of the chain is that a change is
 * visible without going looking for it.
 */
export function TranscriptVerification({
  noteId,
  refreshKey,
}: {
  noteId: string;
  refreshKey?: string | number;
}) {
  const [result, setResult] = useState<Verification | null>(null);

  useEffect(() => {
    let cancelled = false;
    exochainApi
      .verifyTranscript(noteId)
      .then((v) => {
        if (!cancelled) setResult(v);
      })
      .catch(() => {
        // A failure to check is not a failure of the check. Saying nothing is
        // better than implying either verdict.
      });
    return () => {
      cancelled = true;
    };
  }, [noteId, refreshKey]);

  // Nothing recorded yet is an ordinary state, and a note with no transcript
  // has nothing to say about integrity.
  if (!result || result.status === "empty" || result.status === "untracked") {
    return null;
  }

  if (result.status === "altered") {
    return (
      <div
        className="p-3 rounded-lg text-sm"
        style={{ backgroundColor: "rgba(239, 68, 68, 0.12)", color: "#ef4444" }}
      >
        <strong>This transcript has changed since it was recorded.</strong>
        <p className="mt-1" style={{ color: "var(--color-text-secondary)" }}>
          It matches no version in its history. Every edit made through Note67
          appends a version, so something changed it another way. Version{" "}
          {result.latestVersion} recorded{" "}
          <code className="font-mono text-xs">
            {result.expectedHash.slice(0, 12)}
          </code>
          ; the text here is{" "}
          <code className="font-mono text-xs">
            {result.actualHash.slice(0, 12)}
          </code>
          .
        </p>
      </div>
    );
  }

  const { version, attested, receiptHash, isLatest } = result;

  return (
    <div
      className="p-3 rounded-lg text-sm"
      style={{
        backgroundColor: attested
          ? "rgba(34, 197, 94, 0.12)"
          : "var(--color-bg-subtle)",
        color: attested ? "#22c55e" : "var(--color-text-secondary)",
      }}
    >
      <strong>
        {attested
          ? "Attested and unchanged"
          : `Unchanged since version ${version}`}
      </strong>
      <p className="mt-1" style={{ color: "var(--color-text-secondary)" }}>
        {attested ? (
          <>
            A node signed this exact text, and it still hashes to what was
            attested.{" "}
            {receiptHash && (
              <code className="font-mono text-xs">
                {receiptHash.slice(0, 16)}…
              </code>
            )}
          </>
        ) : (
          // The distinction the PRD insists on: unchanged is a weaker claim
          // than attested, and must not be dressed up as the same thing.
          "The text still matches what was recorded here. Nothing has signed it, so this is Note67's own record rather than anyone else's."
        )}
      </p>
      {!isLatest && (
        <p className="mt-1" style={{ color: "#eab308" }}>
          Note this is version {version}, not the newest — the transcript has
          gone backwards.
        </p>
      )}
    </div>
  );
}
