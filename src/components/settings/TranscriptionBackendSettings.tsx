import { useState } from "react";

import {
  DEFAULT_CONFIG,
  willUseRemote,
  type TranscriptionConfig,
} from "../../hooks/useTranscriptionBackend";

/**
 * Which recogniser transcribes uploaded audio.
 *
 * Edits are local until Save, so a half-typed URL is never applied — the same
 * rule the AI provider form follows, and for the same reason.
 */
export function TranscriptionBackendSettings({
  config,
  onSave,
  saving,
  error,
}: {
  config: TranscriptionConfig;
  onSave: (next: TranscriptionConfig) => Promise<boolean>;
  saving: boolean;
  error: string | null;
}) {
  const [draft, setDraft] = useState<TranscriptionConfig>(config);
  const [saved, setSaved] = useState(false);

  const remote = draft.backend === "remote";
  const willSend = willUseRemote(draft);

  const field = (
    label: string,
    value: string,
    onChange: (v: string) => void,
    placeholder: string,
    type: "text" | "password" = "text",
  ) => (
    <div>
      <label
        className="block text-xs mb-1"
        style={{ color: "var(--color-text-secondary)" }}
      >
        {label}
      </label>
      <input
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(e) => {
          setSaved(false);
          onChange(e.target.value);
        }}
        className="w-full p-2 rounded-lg text-sm"
        style={{
          backgroundColor: "var(--color-bg-elevated)",
          color: "var(--color-text)",
          border: "1px solid var(--color-border)",
        }}
      />
    </div>
  );

  return (
    <div className="space-y-4">
      <div>
        <h3
          className="text-sm font-semibold mb-1"
          style={{ color: "var(--color-text)" }}
        >
          Transcribing uploaded audio
        </h3>
        <p className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
          Whisper runs here and hears one voice per track, so a meeting comes
          back as "You" and "Others". A remote recogniser can separate speakers —
          at the cost of sending the recording to it.
        </p>
        <p
          className="text-sm mt-2"
          style={{ color: "var(--color-text-secondary)" }}
        >
          Live transcription always runs here, whatever this is set to.
        </p>
      </div>

      <div className="space-y-2">
        {(
          [
            [
              "local",
              "On this machine (Whisper)",
              "Nothing leaves the device. No speaker separation.",
            ],
            [
              "remote",
              "Send to a recogniser",
              "Separates speakers into Speaker 1, Speaker 2… which you can then rename.",
            ],
          ] as const
        ).map(([value, label, hint]) => (
          <label
            key={value}
            className="flex gap-3 p-3 rounded-lg cursor-pointer"
            style={{
              backgroundColor:
                draft.backend === value
                  ? "var(--color-bg-elevated)"
                  : "var(--color-bg-subtle)",
              border: `1px solid ${
                draft.backend === value
                  ? "var(--color-accent)"
                  : "var(--color-border)"
              }`,
            }}
          >
            <input
              type="radio"
              className="mt-1"
              checked={draft.backend === value}
              onChange={() => {
                setSaved(false);
                setDraft({ ...draft, backend: value });
              }}
            />
            <div>
              <div className="text-sm" style={{ color: "var(--color-text)" }}>
                {label}
              </div>
              <div
                className="text-xs mt-0.5"
                style={{ color: "var(--color-text-secondary)" }}
              >
                {hint}
              </div>
            </div>
          </label>
        ))}
      </div>

      {remote && (
        <div className="space-y-3">
          {field(
            "Service address",
            draft.baseUrl,
            (baseUrl) => setDraft({ ...draft, baseUrl }),
            "http://192.168.32.223:8010",
          )}
          {field(
            "API key (optional)",
            draft.apiKey,
            (apiKey) => setDraft({ ...draft, apiKey }),
            "leave empty if the service needs none",
            "password",
          )}
          {field(
            "Most speakers to expect (optional)",
            draft.maxSpeakers,
            (maxSpeakers) => setDraft({ ...draft, maxSpeakers }),
            "e.g. 8 — leave empty to let it work this out",
          )}

          {/* The Rust side falls back to local for a URL it cannot use. Saying
              so here is the difference between "not finished yet" and a
              diarizer that appears to have stopped working. */}
          {!willSend && (
            <p className="text-sm" style={{ color: "#eab308" }}>
              Not usable yet — an address starting <code>http://</code> or{" "}
              <code>https://</code> is needed. Until then uploads are
              transcribed on this machine.
            </p>
          )}

          {willSend && (
            <p className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
              Uploaded recordings will be sent to this address. Nothing attests
              that the audio left this machine — receipts describe the
              transcript, not where its audio has been.
            </p>
          )}
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
          disabled={saving}
          onClick={async () => {
            if (await onSave(draft)) setSaved(true);
          }}
          className="text-sm px-3 py-1.5 rounded-lg disabled:opacity-50"
          style={{
            backgroundColor: "var(--color-accent, #3b82f6)",
            color: "white",
          }}
        >
          {saving ? "Saving…" : "Save"}
        </button>
        {saved && (
          <span className="text-sm" style={{ color: "#22c55e" }}>
            Saved — applies to the next upload.
          </span>
        )}
        <button
          type="button"
          onClick={() => {
            setSaved(false);
            setDraft(DEFAULT_CONFIG);
          }}
          className="text-sm underline"
          style={{ color: "var(--color-text-secondary)" }}
        >
          Reset
        </button>
      </div>
    </div>
  );
}
