import { useCallback, useEffect, useState } from "react";

import { settingsApi } from "../api";

export type TranscriptionBackend = "local" | "remote" | "streaming" | "openai" | "local_nemo";

export const BACKEND_KEY = "transcription_backend";
export const BASE_URL_KEY = "transcription_base_url";
export const API_KEY_KEY = "transcription_api_key";
export const MAX_SPEAKERS_KEY = "transcription_max_speakers";
export const STREAM_URL_KEY = "transcription_stream_url";
export const OPENAI_MODEL_KEY = "transcription_openai_model";
export const NEMO_PATH_KEY = "transcription_nemo_path";
export const NEMO_ASR_MODEL_KEY = "transcription_nemo_asr_model";
export const NEMO_DIAR_MODEL_KEY = "transcription_nemo_diar_model";

export interface TranscriptionConfig {
  backend: TranscriptionBackend;
  baseUrl: string;
  apiKey: string;
  maxSpeakers: string;
  streamUrl: string;
  openaiModel: string;
  nemoPath: string;
  nemoAsrModel: string;
  nemoDiarModel: string;
}

export const DEFAULT_CONFIG: TranscriptionConfig = {
  backend: "local",
  baseUrl: "",
  apiKey: "",
  maxSpeakers: "",
  streamUrl: "",
  openaiModel: "",
  nemoPath: "",
  nemoAsrModel: "",
  nemoDiarModel: "",
};

/**
 * Whether a saved config would actually reach a remote recogniser.
 *
 * Mirrors `transcription::backend::resolve` in Rust, which falls back to local
 * for anything it cannot use. Shown in the UI so a half-finished setting says
 * so here, rather than looking configured and quietly transcribing locally —
 * which would present as the diarizer never working.
 */
export function willUseRemote(config: TranscriptionConfig): boolean {
  if (config.backend !== "remote") return false;
  const url = config.baseUrl.trim();
  return url.startsWith("http://") || url.startsWith("https://");
}

/**
 * Whether a saved config would reach an OpenAI-compatible recogniser.
 *
 * Same mirror of `resolve` as `willUseRemote`, and the same reason: a
 * half-finished setting has to read as "not configured" here rather than
 * looking right and silently transcribing on the local model instead.
 */
export function willUseOpenAi(config: TranscriptionConfig): boolean {
  if (config.backend !== "openai") return false;
  const url = config.baseUrl.trim();
  return url.startsWith("http://") || url.startsWith("https://");
}

/**
 * Whether a saved config would actually stream live audio off the device.
 *
 * Same purpose as `willUseRemote`, and the same mirror of `resolve` — but this
 * one is also the honest label for the most sensitive setting in the app, so it
 * has to be exact. A URL that is nearly right must read as "not streaming",
 * never as "streaming".
 */
export function willStream(config: TranscriptionConfig): boolean {
  if (config.backend !== "streaming") return false;
  const url = config.streamUrl.trim();
  return url.startsWith("ws://") || url.startsWith("wss://");
}

function readBackend(value: string | null | undefined): TranscriptionBackend {
  if (value === "remote") return "remote";
  if (value === "streaming") return "streaming";
  if (value === "openai") return "openai";
  if (value === "local_nemo") return "local_nemo";
  return "local";
}

/**
 * Whether to re-run local Whisper over the recording once it stops.
 *
 * The app does this by default "for better quality", replacing the transcript
 * it just built. That is right when the live transcript came from local
 * Whisper, and wrong when a remote recogniser produced it: re-running the
 * weaker recogniser over the same audio would quietly overwrite the better
 * transcript a minute after the meeting ended.
 *
 * Only the streaming backend transcribes live. With `remote`, live
 * transcription is still local Whisper — `remote` governs uploads — so
 * retranscribing there is the improvement it claims to be.
 */
export function shouldAutoRetranscribe(
  backend: string | null | undefined,
  hasLocalModel: boolean,
): boolean {
  if (!hasLocalModel) return false;
  return readBackend(backend ?? undefined) !== "streaming";
}

export function useTranscriptionBackend() {
  const [config, setConfig] = useState<TranscriptionConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    settingsApi
      .getMultiple([
        BACKEND_KEY,
        BASE_URL_KEY,
        API_KEY_KEY,
        MAX_SPEAKERS_KEY,
        STREAM_URL_KEY,
        OPENAI_MODEL_KEY,
        NEMO_PATH_KEY,
        NEMO_ASR_MODEL_KEY,
        NEMO_DIAR_MODEL_KEY,
      ])
      .then((values) => {
        if (cancelled) return;
        setConfig({
          // Anything unrecognised reads as local, matching
          // `BackendKind::from_setting`: a partly-written setting must not
          // present as one that ships audio off the machine.
          backend: readBackend(values[BACKEND_KEY]),
          baseUrl: values[BASE_URL_KEY] ?? "",
          apiKey: values[API_KEY_KEY] ?? "",
          maxSpeakers: values[MAX_SPEAKERS_KEY] ?? "",
          streamUrl: values[STREAM_URL_KEY] ?? "",
          openaiModel: values[OPENAI_MODEL_KEY] ?? "",
          nemoPath: values[NEMO_PATH_KEY] ?? "",
          nemoAsrModel: values[NEMO_ASR_MODEL_KEY] ?? "",
          nemoDiarModel: values[NEMO_DIAR_MODEL_KEY] ?? "",
        });
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
        // A settings read that fails must not leave the panel blank forever;
        // the defaults are also what the backend falls back to.
        setConfig(DEFAULT_CONFIG);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const save = useCallback(async (next: TranscriptionConfig) => {
    setSaving(true);
    setError(null);
    try {
      await settingsApi.set(BACKEND_KEY, next.backend);
      await settingsApi.set(BASE_URL_KEY, next.baseUrl.trim());
      await settingsApi.set(API_KEY_KEY, next.apiKey.trim());
      await settingsApi.set(MAX_SPEAKERS_KEY, next.maxSpeakers.trim());
      await settingsApi.set(STREAM_URL_KEY, next.streamUrl.trim());
      await settingsApi.set(OPENAI_MODEL_KEY, next.openaiModel.trim());
      await settingsApi.set(NEMO_PATH_KEY, next.nemoPath.trim());
      await settingsApi.set(NEMO_ASR_MODEL_KEY, next.nemoAsrModel.trim());
      await settingsApi.set(NEMO_DIAR_MODEL_KEY, next.nemoDiarModel.trim());
      setConfig(next);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    } finally {
      setSaving(false);
    }
  }, []);

  return { config, save, saving, error };
}
