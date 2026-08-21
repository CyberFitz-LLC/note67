import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";

import { useAiProvider } from "./useAiProvider";
import { aiApi } from "../api";

vi.mock("../api", () => ({
  aiApi: {
    getProviderConfig: vi.fn(),
    setProviderConfig: vi.fn(),
    testConnection: vi.fn(),
  },
}));

const checkStatus = vi.fn();
vi.mock("../stores/ollamaStore", () => ({
  useOllamaStore: { getState: () => ({ checkStatus }) },
}));

const api = vi.mocked(aiApi, true);

const LOCAL_OLLAMA = {
  provider: "ollama" as const,
  baseUrl: "http://localhost:11434",
  hasApiKey: false,
};
const SPARK_VLLM = {
  provider: "openai_compat" as const,
  baseUrl: "http://spark:8000/v1",
  hasApiKey: false,
};

beforeEach(() => {
  vi.clearAllMocks();
  api.getProviderConfig.mockResolvedValue(LOCAL_OLLAMA);
  api.setProviderConfig.mockResolvedValue(SPARK_VLLM);
  api.testConnection.mockResolvedValue({
    ok: true,
    message: "Connected. 3 model(s) available.",
    modelCount: 3,
  });
});

async function renderLoaded() {
  const hook = renderHook(() => useAiProvider());
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  return hook;
}

describe("useAiProvider", () => {
  it("loads the saved provider config on mount", async () => {
    const { result } = await renderLoaded();
    expect(result.current.config).toEqual(LOCAL_OLLAMA);
  });

  it("saves a switch to an OpenAI-compatible server", async () => {
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.saveConfig("openai_compat", "http://spark:8000/v1");
    });

    expect(api.setProviderConfig).toHaveBeenCalledWith(
      "openai_compat",
      "http://spark:8000/v1",
      undefined
    );
    expect(result.current.config).toEqual(SPARK_VLLM);
  });

  it("refreshes the model list after the backend changes", async () => {
    // The previous server's models are meaningless against the new one, so a
    // stale list would let the user pick a model that does not exist.
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.saveConfig("openai_compat", "http://spark:8000/v1");
    });

    expect(checkStatus).toHaveBeenCalled();
  });

  it("passes an omitted API key through as undefined to keep the stored one", async () => {
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.saveConfig("ollama", "http://ollama.lan:11434");
    });

    expect(api.setProviderConfig).toHaveBeenCalledWith(
      "ollama",
      "http://ollama.lan:11434",
      undefined
    );
  });

  it("passes an empty API key through so it can be cleared", async () => {
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.saveConfig("openai_compat", "http://spark:8000", "");
    });

    expect(api.setProviderConfig).toHaveBeenCalledWith(
      "openai_compat",
      "http://spark:8000",
      ""
    );
  });

  it("surfaces a rejected config and keeps the previous one", async () => {
    const { result } = await renderLoaded();
    api.setProviderConfig.mockRejectedValue(
      new Error("The server URL must start with http:// or https://")
    );

    await act(async () => {
      await result.current.saveConfig("openai_compat", "spark:8000");
    });

    expect(result.current.error).toContain("must start with http://");
    expect(result.current.config).toEqual(LOCAL_OLLAMA);
    expect(checkStatus).not.toHaveBeenCalled();
  });

  it("reports a successful connection test", async () => {
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.testConnection();
    });

    expect(result.current.testResult).toEqual({
      ok: true,
      message: "Connected. 3 model(s) available.",
      modelCount: 3,
    });
  });

  it("reports a failed connection test without throwing", async () => {
    // The backend reports unreachable as a normal result, not an error, so the
    // UI can show the reason inline.
    api.testConnection.mockResolvedValue({
      ok: false,
      message: "Cannot reach the model server at http://spark:8000/v1",
      modelCount: 0,
    });
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.testConnection();
    });

    expect(result.current.testResult?.ok).toBe(false);
    expect(result.current.testResult?.message).toContain("Cannot reach");
  });

  it("clears a stale test result when the config is saved", async () => {
    // A green tick from the previous server must not sit next to a new URL.
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.testConnection();
    });
    expect(result.current.testResult?.ok).toBe(true);

    await act(async () => {
      await result.current.saveConfig("openai_compat", "http://spark:8000/v1");
    });

    expect(result.current.testResult).toBeNull();
  });

  it("surfaces a failure to load the config", async () => {
    api.getProviderConfig.mockRejectedValue(new Error("db unavailable"));

    const { result } = await renderLoaded();

    expect(result.current.error).toContain("db unavailable");
  });
});
