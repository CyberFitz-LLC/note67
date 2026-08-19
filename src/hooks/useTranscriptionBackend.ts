import { useCallback, useEffect, useState } from "react";

import { settingsApi } from "../api";

export type TranscriptionBackend = "local" | "remote" | "streaming";

export const BACKEND_KEY = "transcription_backend";
export const BASE_URL_KEY = "transcription_base_url";
export const API_KEY_KEY = "transcription_api_key";
export const MAX_SPEAKERS_KEY = "transcription_max_speakers";
export const STREAM_URL_KEY = "transcription_stream_url";

export interface TranscriptionConfig {
  backend: TranscriptionBackend;
  baseUrl: string;
  apiKey: string;
  maxSpeakers: string;
  streamUrl: string;
}

export const DEFAULT_CONFIG: TranscriptionConfig = {
  backend: "local",
  baseUrl: "",
  apiKey: "",
  maxSpeakers: "",
  streamUrl: "",
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
  return "local";
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
