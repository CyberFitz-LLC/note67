import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface CompactionFailure {
  path: string;
  reason: string;
}

interface CompactionReport {
  files_examined: number;
  files_compacted: number;
  files_failed: number;
  bytes_before: number;
  bytes_after: number;
  failures: CompactionFailure[];
  orphans: number;
}

function gb(bytes: number): string {
  if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
  if (bytes >= 1_048_576) return `${Math.round(bytes / 1_048_576)} MB`;
  return `${Math.round(bytes / 1024)} KB`;
}

/**
 * Recover the space taken by recordings made before compression.
 *
 * New recordings are compacted as they finish, so this is a one-off for what is
 * already on disk — and for most people that is where all the space actually
 * is.
 */
export function StorageSettings() {
  const [running, setRunning] = useState(false);
  const [report, setReport] = useState<CompactionReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setRunning(true);
    setError(null);
    setReport(null);
    try {
      setReport(await invoke<CompactionReport>("compact_recordings"));
    } catch (e) {
      setError(String(e));
    } finally {
      setRunning(false);
    }
  };

  const saved = report ? report.bytes_before - report.bytes_after : 0;

  return (
    <div className="space-y-4">
      <div>
        <h3
          className="text-sm font-semibold mb-1"
          style={{ color: "var(--color-text)" }}
        >
          Recording storage
        </h3>
        <p className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
          Recordings are stored compressed, at about an eighth of the space they
          used to take. New ones are compressed when they finish; anything
          recorded before that is still full size until you compact it.
        </p>
      </div>

      <div
        className="p-3 rounded-lg text-sm space-y-2"
        style={{
          backgroundColor: "var(--color-bg-subtle)",
          border: "1px solid var(--color-border)",
          color: "var(--color-text-secondary)",
        }}
      >
        <p>
          Compacting rewrites each recording, checks the new file is readable,
          and only then removes the original. It can be stopped and re-run — a
          recording it has already done is skipped.
        </p>
        <p>
          Audio is kept at the quality transcription uses, which is lower than
          it was captured at. The compression itself loses nothing.
        </p>
      </div>

      {error && (
        <p className="text-sm" style={{ color: "#ef4444" }}>
          {error}
        </p>
      )}

      {report && (
        <div className="text-sm space-y-2" style={{ color: "var(--color-text)" }}>
          {report.files_compacted > 0 && (
            <p>
              Compacted {report.files_compacted} of {report.files_examined}{" "}
              recordings — <strong>{gb(saved)} recovered</strong> ({gb(report.bytes_before)}{" "}
              → {gb(report.bytes_after)}).
            </p>
          )}
          {/* Said only when it is true. Reporting "all already compressed"
              while also reporting failures is a claim the numbers contradict,
              and it is the failures that need attention. */}
          {report.files_compacted === 0 && report.files_failed === 0 && (
            <p>
              Nothing to do — all {report.files_examined} recordings were already
              compressed.
            </p>
          )}
          {report.files_compacted === 0 && report.files_failed > 0 && (
            <p>
              Nothing was compacted. {report.files_examined - report.files_failed}{" "}
              of {report.files_examined} were already compressed.
            </p>
          )}

          {report.orphans > 0 && (
            <p style={{ color: "var(--color-text-secondary)" }}>
              {report.orphans} of those belong to notes that no longer exist —
              deleting a note has never removed its audio. They were compressed
              too, but nothing will ever play them.
            </p>
          )}

          {report.files_failed > 0 && (
            <details
              className="rounded-lg p-2"
              style={{
                backgroundColor: "var(--color-bg-subtle)",
                border: "1px solid #eab308",
              }}
            >
              <summary style={{ color: "#eab308", cursor: "pointer" }}>
                {report.files_failed} could not be read and were left untouched
              </summary>
              <ul className="mt-2 space-y-1">
                {report.failures.map((f) => (
                  <li key={f.path} style={{ color: "var(--color-text-secondary)" }}>
                    <code className="text-xs">{f.path.split(/[\\/]/).pop()}</code>
                    <div className="text-xs">{f.reason}</div>
                  </li>
                ))}
              </ul>
            </details>
          )}
        </div>
      )}

      <button
        type="button"
        disabled={running}
        onClick={run}
        className="text-sm px-3 py-1.5 rounded-lg disabled:opacity-50"
        style={{
          backgroundColor: "var(--color-accent, #3b82f6)",
          color: "white",
        }}
      >
        {running ? "Compacting…" : "Compact recordings"}
      </button>
      {running && (
        <p className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
          This can take a while on a large library. It is safe to leave running.
        </p>
      )}
    </div>
  );
}
