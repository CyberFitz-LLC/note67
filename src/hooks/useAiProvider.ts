import { useCallback, useEffect, useState } from "react";

import { aiApi } from "../api";
import { useOllamaStore } from "../stores/ollamaStore";
import type { AiConnectionTest, AiProvider, AiProviderConfig } from "../types";

/**
 * Which model server the app talks to: a local or remote Ollama, or any
 * OpenAI-compatible endpoint (vLLM, llama.cpp server, LM Studio).
 */
export function useAiProvider() {
  const [config, setConfig] = useState<AiProviderConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<AiConnectionTest | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Initial load. Inlined so setState only runs in the async continuation.
  useEffect(() => {
    let cancelled = false;
    aiApi
      .getProviderConfig()
      .then((next) => {
        if (cancelled) return;
        setConfig(next);
        setError(null);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  /**
   * Persist a new backend.
   *
   * `apiKey` left undefined keeps whatever key is stored; `""` clears it. That
   * lets the UI show a masked field which means "unchanged" when blank.
   */
  const saveConfig = useCallback(
    async (provider: AiProvider, baseUrl: string, apiKey?: string) => {
      setSaving(true);
      try {
        const next = await aiApi.setProviderConfig(provider, baseUrl, apiKey);
        setConfig(next);
        // A result from the old server says nothing about the new one.
        setTestResult(null);
        setError(null);
        // The model list belongs to the previous backend; leaving it up would
        // let the user select a model the new server does not have.
        await useOllamaStore.getState().checkStatus();
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setSaving(false);
      }
    },
    []
  );

  const testConnection = useCallback(async () => {
    setTesting(true);
    try {
      setTestResult(await aiApi.testConnection());
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setTesting(false);
    }
  }, []);

  return {
    config,
    loading,
    saving,
    testing,
    testResult,
    error,
    saveConfig,
    testConnection,
  };
}
