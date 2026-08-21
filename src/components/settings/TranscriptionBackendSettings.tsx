import { useState } from "react";

import {
  DEFAULT_CONFIG,
  willStream,
  willUseRemote,
  type TranscriptionConfig,
} from "../../hooks/useTranscriptionBackend";

/**
 * Which recogniser transcribes audio.
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
  const streaming = draft.backend === "streaming";
  const willSend = willUseRemote(draft);
  const willSendLive = willStream(draft);

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
          Transcription
        </h3>
        <p className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
          Whisper runs here and hears one voice per track, so a meeting comes
          back as "You" and "Others". A remote recogniser can do better — at the
          cost of sending your audio to it.
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
              "Send finished recordings to a recogniser",
              "Separates speakers into Speaker 1, Speaker 2… which you can then rename. Live transcription still runs here.",
            ],
            [
              "streaming",
              "Stream live audio to a recogniser",
              "Better live transcription, but the microphone and meeting audio are sent continuously while you record, and speakers are not identified.",
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

      {streaming && (
        <div className="space-y-3">
          {field(
            "Recogniser address",
            draft.streamUrl,
            (streamUrl) => setDraft({ ...draft, streamUrl }),
            "ws://192.168.32.223:8080",
          )}

          {!willSendLive && (
            <p className="text-sm" style={{ color: "#eab308" }}>
              Not usable yet — an address starting <code>ws://</code> or{" "}
              <code>wss://</code> is needed. Until then everything is
              transcribed on this machine.
            </p>
          )}

          {/* The most sensitive setting in the app, so it is spelled out rather
              than implied. Someone turning this on is agreeing to send the room
              as it is heard, including whatever is said before anyone decides
              the meeting matters. */}
          {willSendLive && (
            <div
              className="p-3 rounded-lg space-y-2 text-sm"
              style={{
                backgroundColor: "var(--color-bg-subtle)",
                border: "1px solid #eab308",
                color: "var(--color-text-secondary)",
              }}
            >
              <p style={{ color: "var(--color-text)" }}>
                While you record, your microphone and the meeting audio are sent
                continuously to this address.
              </p>
              <p>
                That includes anything said before you decide the meeting
                matters. Nothing attests where the audio went — receipts
                describe the transcript, not its audio.
              </p>
              <p>
                This recogniser does not identify speakers. Everything you say
                is labelled "You" and everything from the meeting "Others",
                which is all the app can tell without a diarizer.
              </p>
              <p>
                <code>ws://</code> is unencrypted. Use it only on a network you
                trust.
              </p>
            </div>
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
            Saved — applies to the next recording.
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
