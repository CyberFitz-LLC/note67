import { useState } from "react";

import type { TranscriptChain, TranscriptVersion } from "../types";

const REASON_LABEL: Record<TranscriptVersion["reason"], string> = {
  initial: "First transcript",
  retranscribe: "Re-transcribed",
  edit: "Edited",
  import: "Imported",
};

function formatWhen(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Enough of the hash to recognise, with the whole thing available on copy. */
function shortHash(hash: string): string {
  return hash.length > 16 ? `${hash.slice(0, 8)}…${hash.slice(-8)}` : hash;
}

interface TranscriptHistoryProps {
  chain: TranscriptChain | null;
  loading: boolean;
  error: string | null;
}

/**
 * The transcript's version history.
 *
 * Every change appends a version carrying the previous one's hash, so an
 * alteration shows up as a new entry rather than silently replacing what came
 * before.
 */
export function TranscriptHistory({
  chain,
  loading,
  error,
}: TranscriptHistoryProps) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);

  // Nothing to show before a transcript exists. A note with no versions is an
  // ordinary state, not something to warn about.
  if (!chain || chain.versions.length === 0) return null;

  const newestFirst = [...chain.versions].sort((a, b) => b.version - a.version);
  const latest = newestFirst[0];

  const copy = (hash: string) => {
    navigator.clipboard?.writeText(hash).then(
      () => {
        setCopied(hash);
        window.setTimeout(() => setCopied(null), 1500);
      },
      () => {}
    );
  };

  return (
    <div
      className="mt-4 rounded-xl overflow-hidden"
      style={{ backgroundColor: "var(--color-bg-subtle)" }}
    >
      <button
        onClick={() => setOpen(!open)}
        className="w-full flex items-center justify-between p-3 text-left"
        aria-expanded={open}
      >
        <div className="flex items-center gap-2">
          <span
            className="text-sm font-medium"
            style={{ color: "var(--color-text)" }}
          >
            Transcript history
          </span>
          <span
            className="px-1.5 py-0.5 text-xs rounded"
            style={{
              backgroundColor: "var(--color-bg-elevated)",
              color: "var(--color-text-secondary)",
            }}
          >
            {chain.versions.length} version
            {chain.versions.length === 1 ? "" : "s"}
          </span>
          {!chain.intact && (
            <span
              className="px-1.5 py-0.5 text-xs font-medium rounded"
              style={{
                backgroundColor: "rgba(239, 68, 68, 0.15)",
                color: "#dc2626",
              }}
            >
              Chain broken
            </span>
          )}
        </div>
        <svg
          className="w-4 h-4 shrink-0"
          style={{
            color: "var(--color-text-tertiary)",
            transform: open ? "rotate(180deg)" : undefined,
          }}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 9l-7 7-7-7"
          />
        </svg>
      </button>

      {open && (
        <div className="px-3 pb-3">
          {loading && (
            <p
              className="text-xs mb-2"
              style={{ color: "var(--color-text-tertiary)" }}
            >
              Loading…
            </p>
          )}

          {error && (
            <div
              className="mb-3 p-2 rounded-lg text-xs"
              style={{
                backgroundColor: "rgba(239, 68, 68, 0.08)",
                color: "#dc2626",
              }}
            >
              {error}
            </div>
          )}

          {!chain.intact && (
            <div
              className="mb-3 p-3 rounded-lg text-xs"
              style={{
                backgroundColor: "rgba(239, 68, 68, 0.08)",
                color: "var(--color-text-secondary)",
              }}
            >
              <p className="mb-1" style={{ color: "#dc2626", fontWeight: 500 }}>
                This history does not verify.
              </p>
              <p>
                A version was removed, reordered or replaced — the record has
                been altered rather than added to.
                {chain.brokenReason ? ` ${chain.brokenReason}.` : ""}
              </p>
            </div>
          )}

          <ol className="space-y-2">
            {newestFirst.map((v) => (
              <li
                key={v.version}
                className="p-2.5 rounded-lg"
                style={{
                  backgroundColor: "var(--color-bg-elevated)",
                  border:
                    v.version === latest.version
                      ? "1px solid rgba(59, 130, 246, 0.25)"
                      : "1px solid transparent",
                }}
              >
                <div className="flex items-center justify-between gap-2 mb-1">
                  <div className="flex items-center gap-2 min-w-0">
                    <span
                      className="text-sm font-medium"
                      style={{ color: "var(--color-text)" }}
                    >
                      v{v.version}
                    </span>
                    <span
                      className="text-xs"
                      style={{ color: "var(--color-text-secondary)" }}
                    >
                      {REASON_LABEL[v.reason]}
                    </span>
                    {v.origin === "imported" && (
                      <span
                        className="px-1.5 py-0.5 text-xs rounded shrink-0"
                        style={{
                          backgroundColor: "rgba(245, 158, 11, 0.15)",
                          color: "#b45309",
                        }}
                        title="Note67 did not produce this transcript; it was imported"
                      >
                        Imported
                      </span>
                    )}
                    {v.version === latest.version && (
                      <span
                        className="text-xs"
                        style={{ color: "var(--color-accent)" }}
                      >
                        current
                      </span>
                    )}
                  </div>
                  <span
                    className="text-xs shrink-0"
                    style={{ color: "var(--color-text-tertiary)" }}
                  >
                    {formatWhen(v.createdAt)}
                  </span>
                </div>

                <div className="flex items-center justify-between gap-2">
                  <button
                    onClick={() => copy(v.contentHash)}
                    className="text-xs font-mono truncate"
                    style={{ color: "var(--color-text-tertiary)" }}
                    title={`${v.contentHash} — click to copy`}
                  >
                    {copied === v.contentHash
                      ? "copied"
                      : shortHash(v.contentHash)}
                  </button>
                  <span
                    className="text-xs shrink-0"
                    style={{ color: "var(--color-text-tertiary)" }}
                  >
                    {v.segmentCount} segment{v.segmentCount === 1 ? "" : "s"}
                  </span>
                </div>
              </li>
            ))}
          </ol>

          {/* PRD section 6 governs this wording. Nothing here has been signed by
              a node yet, so the history must not imply it has been verified by
              anyone — it is a local record of what changed and when. */}
          <p
            className="mt-3 text-xs"
            style={{ color: "var(--color-text-tertiary)" }}
          >
            Each version records the content of the transcript at that moment
            and links to the one before it, so a later change is visible rather
            than silent. These are local records — they are not yet signed by a
            governance node, and they do not attest that a transcript is
            accurate.
          </p>
        </div>
      )}
    </div>
  );
}
