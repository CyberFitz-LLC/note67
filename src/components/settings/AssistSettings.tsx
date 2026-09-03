import { useEffect, useState } from "react";

import { settingsApi } from "../../api";

const ENABLED_KEY = "assist_enabled";
const URL_KEY = "assist_hindsight_url";
const BANK_KEY = "assist_hindsight_bank";

/**
 * Live meeting assistance.
 *
 * Off unless switched on here, and the copy says what switching it on does:
 * this is the setting that sends a meeting to a model while people are still
 * talking, which is not something to discover after the fact.
 */
export function AssistSettings() {
  const [enabled, setEnabled] = useState(false);
  const [url, setUrl] = useState("");
  const [bank, setBank] = useState("");
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    settingsApi
      .getMultiple([ENABLED_KEY, URL_KEY, BANK_KEY])
      .then((values) => {
        if (cancelled) return;
        setEnabled(values[ENABLED_KEY] === "true");
        setUrl(values[URL_KEY] ?? "");
        setBank(values[BANK_KEY] ?? "");
      })
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, []);

  const save = async () => {
    setError(null);
    try {
      await settingsApi.set(ENABLED_KEY, enabled ? "true" : "false");
      await settingsApi.set(URL_KEY, url.trim());
      await settingsApi.set(BANK_KEY, bank.trim());
      setSaved(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const memoryUsable =
    bank.trim().length > 0 &&
    (url.trim().startsWith("http://") || url.trim().startsWith("https://"));

  return (
    <div className="space-y-4">
      <div>
        <h3
          className="text-sm font-semibold mb-1"
          style={{ color: "var(--color-text)" }}
        >
          Live meeting assistance
        </h3>
        <p className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
          Two panes beside a running meeting: what is being discussed, and what
          you could say next.
        </p>
      </div>

      <label className="flex gap-3 items-start cursor-pointer">
        <input
          type="checkbox"
          className="mt-1"
          checked={enabled}
          onChange={(e) => {
            setSaved(false);
            setEnabled(e.target.checked);
          }}
        />
        <div>
          <div className="text-sm" style={{ color: "var(--color-text)" }}>
            Assist during meetings
          </div>
          <div className="text-xs mt-0.5" style={{ color: "var(--color-text-secondary)" }}>
            While this is running, the transcript is sent to your configured
            model repeatedly as the meeting goes on. One receipt is recorded for
            the session, not for each pass.
          </div>
        </div>
      </label>

      {enabled && (
        <div className="space-y-3">
          <div>
            <label
              className="block text-xs mb-1"
              style={{ color: "var(--color-text-secondary)" }}
            >
              Memory service (optional)
            </label>
            <input
              value={url}
              placeholder="https://hindsight.jtpa.net"
              onChange={(e) => {
                setSaved(false);
                setUrl(e.target.value);
              }}
              className="w-full p-2 rounded-lg text-sm"
              style={{
                backgroundColor: "var(--color-bg-elevated)",
                color: "var(--color-text)",
                border: "1px solid var(--color-border)",
              }}
            />
          </div>

          <div>
            <label
              className="block text-xs mb-1"
              style={{ color: "var(--color-text-secondary)" }}
            >
              Memory bank
            </label>
            <input
              value={bank}
              placeholder="which bank to ask"
              onChange={(e) => {
                setSaved(false);
                setBank(e.target.value);
              }}
              className="w-full p-2 rounded-lg text-sm"
              style={{
                backgroundColor: "var(--color-bg-elevated)",
                color: "var(--color-text)",
                border: "1px solid var(--color-border)",
              }}
            />
          </div>

          <p className="text-xs" style={{ color: "var(--color-text-secondary)" }}>
            {memoryUsable
              ? "Suggestions will draw on what this bank knows, alongside the conversation."
              : "Without both an address and a bank, suggestions come only from the conversation itself — weaker, but still useful."}
          </p>
        </div>
      )}

      {error && (
        <p className="text-sm" style={{ color: "#ef4444" }}>
          {error}
        </p>
      )}

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={save}
          className="text-sm px-3 py-1.5 rounded-lg"
          style={{ backgroundColor: "var(--color-accent, #3b82f6)", color: "white" }}
        >
          Save
        </button>
        {saved && (
          <span className="text-sm" style={{ color: "#22c55e" }}>
            Saved.
          </span>
        )}
      </div>
    </div>
  );
}
