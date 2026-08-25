import { useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";

import type { Screenshot } from "../hooks/useScreenshots";

function stamp(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

/**
 * Screenshots taken during a meeting, in the order they appeared.
 *
 * Each carries the point in the call it was pasted at, which is the reason to
 * keep it here rather than in a folder: a slide means something different
 * depending on what was being said when it went up.
 */
export function ScreenshotStrip({
  screenshots,
  onExtract,
  onDelete,
}: {
  screenshots: Screenshot[];
  onExtract: (id: number) => Promise<string | null>;
  onDelete: (id: number) => void;
}) {
  const [reading, setReading] = useState<number | null>(null);
  const [open, setOpen] = useState<number | null>(null);

  if (screenshots.length === 0) return null;

  return (
    <div className="space-y-3">
      <h4 className="text-sm font-semibold" style={{ color: "var(--color-text)" }}>
        Screens shared ({screenshots.length})
      </h4>

      <div className="flex gap-3 overflow-x-auto pb-2">
        {screenshots.map((shot) => (
          <div
            key={shot.id}
            className="flex-shrink-0 rounded-lg overflow-hidden"
            style={{
              width: 200,
              border: "1px solid var(--color-border)",
              backgroundColor: "var(--color-bg-subtle)",
            }}
          >
            <button
              type="button"
              onClick={() => setOpen(open === shot.id ? null : shot.id)}
              className="block w-full"
              title="Show full size"
            >
              <img
                src={convertFileSrc(shot.file_path)}
                alt={`Screen shared at ${stamp(shot.captured_at_ms)}`}
                style={{ width: "100%", height: 110, objectFit: "cover" }}
              />
            </button>

            <div className="p-2 space-y-2">
              <div
                className="text-xs font-mono"
                style={{ color: "var(--color-text-secondary)" }}
              >
                {stamp(shot.captured_at_ms)}
              </div>

              {shot.extracted_text ? (
                <div
                  className="text-xs"
                  style={{ color: "var(--color-text-secondary)" }}
                >
                  Read — in the assistant&apos;s context
                </div>
              ) : (
                <button
                  type="button"
                  disabled={reading === shot.id}
                  onClick={async () => {
                    setReading(shot.id);
                    await onExtract(shot.id);
                    setReading(null);
                  }}
                  className="text-xs underline disabled:opacity-50"
                  style={{ color: "var(--color-accent, #3b82f6)" }}
                >
                  {reading === shot.id ? "Reading…" : "Read with AI"}
                </button>
              )}

              <button
                type="button"
                onClick={() => onDelete(shot.id)}
                className="text-xs underline block"
                style={{ color: "var(--color-text-tertiary)" }}
              >
                Remove
              </button>
            </div>
          </div>
        ))}
      </div>

      {open !== null && (
        <div className="space-y-2">
          {(() => {
            const shot = screenshots.find((s) => s.id === open);
            if (!shot) return null;
            return (
              <>
                <img
                  src={convertFileSrc(shot.file_path)}
                  alt={`Screen shared at ${stamp(shot.captured_at_ms)}`}
                  className="rounded-lg max-w-full"
                  style={{ border: "1px solid var(--color-border)" }}
                />
                {shot.extracted_text && (
                  <div
                    className="text-sm p-3 rounded-lg whitespace-pre-wrap"
                    style={{
                      backgroundColor: "var(--color-bg-subtle)",
                      border: "1px solid var(--color-border)",
                      color: "var(--color-text-secondary)",
                    }}
                  >
                    {/* Said plainly, because this is not part of the
                        transcript and must not be read as something anyone
                        said. */}
                    <div
                      className="text-xs mb-2"
                      style={{ color: "var(--color-text-tertiary)" }}
                    >
                      Read from the image by the AI — displayed, not spoken.
                    </div>
                    {shot.extracted_text}
                  </div>
                )}
              </>
            );
          })()}
        </div>
      )}
    </div>
  );
}
