import { useState } from "react";

import type { AiProvider, AiProviderConfig } from "../../types";
import type { useAiProvider } from "../../hooks";

const DEFAULT_URLS: Record<AiProvider, string> = {
  ollama: "http://localhost:11434",
  openai_compat: "http://localhost:8000/v1",
};

const PROVIDER_LABELS: Record<AiProvider, string> = {
  ollama: "Ollama",
  openai_compat: "OpenAI-compatible",
};

interface ProviderSettingsProps {
  provider: ReturnType<typeof useAiProvider>;
  /**
   * The saved config. The parent remounts this component whenever it changes
   * (via `key`), so the form seeds from it once at mount and needs no effect to
   * stay in sync.
   */
  config: AiProviderConfig;
}

/**
 * Which model server to use, and where it lives.
 *
 * Edits are local until Save so a half-typed URL is never applied — the backend
 * rebuilds its client on every save, and the model list is re-read against it.
 */
export function ProviderSettings({ provider, config }: ProviderSettingsProps) {
  const { loading, saving, testing, testResult, error } = provider;

  const [kind, setKind] = useState<AiProvider>(config.provider);
  const [baseUrl, setBaseUrl] = useState(config.baseUrl);
  // Never prefill the key field: the backend does not return the stored key,
  // and a placeholder standing in for it would be saved back verbatim.
  const [apiKey, setApiKey] = useState("");

  const dirty =
    kind !== config.provider || baseUrl !== config.baseUrl || apiKey !== "";

  const handleProviderChange = (next: AiProvider) => {
    setKind(next);
    // An Ollama port on a vLLM box is never right. Offer the new backend's
    // default, unless the user has typed a URL of their own worth keeping.
    if (next === config.provider) {
      setBaseUrl(config.baseUrl);
    } else if (baseUrl === config.baseUrl || baseUrl === DEFAULT_URLS[kind]) {
      setBaseUrl(DEFAULT_URLS[next]);
    }
  };

  const handleSave = () => {
    // An untouched key field means "leave the stored key alone".
    provider.saveConfig(
      kind,
      baseUrl.trim(),
      apiKey === "" ? undefined : apiKey
    );
  };

  return (
    <div
      className="mb-6 p-4 rounded-xl"
      style={{ backgroundColor: "var(--color-bg-subtle)" }}
    >
      <p className="font-medium mb-1" style={{ color: "var(--color-text)" }}>
        Model Server
      </p>
      <p
        className="text-xs mb-4"
        style={{ color: "var(--color-text-tertiary)" }}
      >
        Run models locally with Ollama, or point Note67 at any OpenAI-compatible
        server such as vLLM, llama.cpp, or LM Studio.
      </p>

      <div className="flex gap-2 mb-3">
        {(Object.keys(PROVIDER_LABELS) as AiProvider[]).map((option) => (
          <button
            key={option}
            onClick={() => handleProviderChange(option)}
            disabled={loading || saving}
            className="flex-1 px-3 py-2 text-sm font-medium rounded-lg transition-colors disabled:opacity-50"
            style={{
              backgroundColor:
                kind === option
                  ? "var(--color-accent)"
                  : "var(--color-bg-elevated)",
              color: kind === option ? "white" : "var(--color-text-secondary)",
              border:
                kind === option
                  ? "1px solid transparent"
                  : "1px solid var(--color-border)",
            }}
          >
            {PROVIDER_LABELS[option]}
          </button>
        ))}
      </div>

      <label
        className="block text-xs mb-1"
        style={{ color: "var(--color-text-secondary)" }}
        htmlFor="ai-base-url"
      >
        Server URL
      </label>
      <input
        id="ai-base-url"
        type="text"
        value={baseUrl}
        disabled={loading || saving}
        onChange={(e) => setBaseUrl(e.target.value)}
        placeholder={DEFAULT_URLS[kind]}
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        className="w-full p-2.5 rounded-lg text-sm mb-3 disabled:opacity-50"
        style={{
          backgroundColor: "var(--color-bg-elevated)",
          color: "var(--color-text)",
          border: "1px solid var(--color-border)",
        }}
      />

      {kind === "openai_compat" && (
        <>
          <label
            className="block text-xs mb-1"
            style={{ color: "var(--color-text-secondary)" }}
            htmlFor="ai-api-key"
          >
            API key{" "}
            {config.hasApiKey ? "(saved — leave blank to keep)" : "(optional)"}
          </label>
          <input
            id="ai-api-key"
            type="password"
            value={apiKey}
            disabled={loading || saving}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={
              config.hasApiKey
                ? "••••••••"
                : "Not required on a trusted network"
            }
            spellCheck={false}
            className="w-full p-2.5 rounded-lg text-sm mb-1 disabled:opacity-50"
            style={{
              backgroundColor: "var(--color-bg-elevated)",
              color: "var(--color-text)",
              border: "1px solid var(--color-border)",
            }}
          />
          <p
            className="text-xs mb-3"
            style={{ color: "var(--color-text-tertiary)" }}
          >
            Stored unencrypted in the local app database, like the rest of your
            settings.
          </p>
        </>
      )}

      <div className="flex items-center gap-2">
        <button
          onClick={handleSave}
          disabled={loading || saving || !dirty}
          className="px-4 py-1.5 text-sm font-medium rounded-lg transition-colors disabled:opacity-50"
          style={{ backgroundColor: "var(--color-accent)", color: "white" }}
        >
          {saving ? "Saving..." : "Save"}
        </button>
        <button
          onClick={provider.testConnection}
          disabled={loading || saving || testing || dirty}
          title={dirty ? "Save your changes first" : undefined}
          className="px-3 py-1.5 text-sm font-medium rounded-lg transition-colors disabled:opacity-50"
          style={{
            backgroundColor: "var(--color-bg-elevated)",
            color: "var(--color-text-secondary)",
            border: "1px solid var(--color-border)",
          }}
        >
          {testing ? "Testing..." : "Test connection"}
        </button>
      </div>

      {testResult && (
        <div
          className="mt-3 p-3 rounded-lg text-xs"
          style={{
            backgroundColor: testResult.ok
              ? "rgba(34, 197, 94, 0.08)"
              : "rgba(239, 68, 68, 0.08)",
            color: testResult.ok ? "#16a34a" : "#dc2626",
          }}
        >
          {testResult.message}
        </div>
      )}

      {error && (
        <div
          className="mt-3 p-3 rounded-lg text-xs"
          style={{
            backgroundColor: "rgba(239, 68, 68, 0.08)",
            color: "#dc2626",
          }}
        >
          {error}
        </div>
      )}
    </div>
  );
}
