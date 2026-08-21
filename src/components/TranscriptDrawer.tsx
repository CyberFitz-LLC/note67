import { useState, type ReactNode } from "react";

/**
 * A bar at the foot of the transcript that expands upward.
 *
 * The panels it holds — merging another recording, and the version history —
 * used to sit below the transcript in the same scrolling column, which gave
 * the page two vertical scrollbars: one for the transcript, one to reach the
 * things underneath it. Two scrollbars on one pane is a reliable way to lose
 * something, because neither of them looks like the one that has the rest.
 *
 * Collapsed by default: the transcript is what the tab is for, and these are
 * occasional.
 */
export function TranscriptDrawer({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(false);

  return (
    <div
      className="sticky bottom-0 mt-2"
      style={{ backgroundColor: "var(--color-bg)" }}
    >
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="w-full flex items-center justify-between px-3 py-2 text-sm rounded-lg"
        style={{
          backgroundColor: "var(--color-bg-subtle)",
          color: "var(--color-text-secondary)",
          borderTop: "1px solid var(--color-border)",
        }}
      >
        <span>{label}</span>
        <svg
          className="w-4 h-4 transition-transform"
          style={{ transform: open ? "rotate(180deg)" : "none" }}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.5}
            d="M5 15l7-7 7 7"
          />
        </svg>
      </button>

      {/* Bounded and scrollable in itself. Left to grow, a long version chain
          would push the transcript off the top of the pane — trading two
          scrollbars for a drawer that swallows the thing it sits under. */}
      {open && (
        <div
          className="max-h-[45vh] overflow-y-auto p-3 space-y-3 rounded-b-lg"
          style={{ backgroundColor: "var(--color-bg-subtle)" }}
        >
          {children}
        </div>
      )}
    </div>
  );
}
