import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";

import { useSummaries } from "./useAI";
import { aiApi } from "../api";
import type { EventBusModule } from "../test/eventBus";
import type { Summary } from "../types";

vi.mock("@tauri-apps/api/event", async () => {
  const { createEventBus } = await import("../test/eventBus");
  return createEventBus();
});

vi.mock("../api", () => ({
  aiApi: {
    getNoteSummaries: vi.fn(),
    generateSummaryStream: vi.fn(),
    deleteSummary: vi.fn(),
  },
}));

const api = vi.mocked(aiApi, true);
const eventModule = (await import("@tauri-apps/api/event")) as unknown as EventBusModule;
const bus = eventModule.__bus;

const EVENT = "summary-stream";

function makeSummary(overrides: Partial<Summary> = {}): Summary {
  return {
    id: 1,
    note_id: "note-1",
    summary_type: "overview",
    content: "A summary",
    created_at: "2026-07-02T09:00:00.000Z",
    ...overrides,
  } as Summary;
}

beforeEach(() => {
  vi.clearAllMocks();
  bus.reset();
  api.getNoteSummaries.mockResolvedValue([]);
  api.deleteSummary.mockResolvedValue(undefined);
});

describe("useSummaries — loading", () => {
  it("loads summaries for a note", async () => {
    const summaries = [makeSummary()];
    api.getNoteSummaries.mockResolvedValue(summaries);

    const { result } = renderHook(() => useSummaries("note-1"));

    await waitFor(() => expect(result.current.summaries).toEqual(summaries));
  });

  it("does not fetch when there is no note", async () => {
    renderHook(() => useSummaries(null));
    expect(api.getNoteSummaries).not.toHaveBeenCalled();
  });

  it("refetches when refreshKey changes", async () => {
    const { rerender } = renderHook(
      ({ key }: { key: number }) => useSummaries("note-1", key),
      { initialProps: { key: 0 } }
    );
    await waitFor(() => expect(api.getNoteSummaries).toHaveBeenCalledTimes(1));

    rerender({ key: 1 });
    await waitFor(() => expect(api.getNoteSummaries).toHaveBeenCalledTimes(2));
  });

  it("refetches when the note changes", async () => {
    const { rerender } = renderHook(
      ({ id }: { id: string }) => useSummaries(id),
      { initialProps: { id: "note-1" } }
    );
    await waitFor(() => expect(api.getNoteSummaries).toHaveBeenCalledWith("note-1"));

    rerender({ id: "note-2" });
    await waitFor(() => expect(api.getNoteSummaries).toHaveBeenCalledWith("note-2"));
  });

  it("records a load error", async () => {
    api.getNoteSummaries.mockRejectedValue(new Error("db gone"));

    const { result } = renderHook(() => useSummaries("note-1"));

    await waitFor(() => expect(result.current.error).toBe("db gone"));
  });
});

describe("useSummaries — streaming", () => {
  it("ignores stream chunks while nothing is generating", async () => {
    const { result } = renderHook(() => useSummaries("note-1"));
    await waitFor(() => expect(bus.listenerCount(EVENT)).toBe(1));

    act(() => {
      bus.emit(EVENT, { note_id: "note-1", chunk: "stray", is_done: false });
    });

    expect(result.current.streamingContent).toBe("");
  });

  it("accumulates chunks for the generating note", async () => {
    let resolveGenerate!: (s: Summary) => void;
    api.generateSummaryStream.mockImplementation(
      () => new Promise<Summary>((r) => (resolveGenerate = r))
    );

    const { result } = renderHook(() => useSummaries("note-1"));
    await waitFor(() => expect(bus.listenerCount(EVENT)).toBe(1));

    let generating!: Promise<unknown>;
    act(() => {
      generating = result.current.generateSummary("overview");
    });

    act(() => {
      bus.emit(EVENT, { note_id: "note-1", chunk: "Hello ", is_done: false });
      bus.emit(EVENT, { note_id: "note-1", chunk: "world", is_done: false });
    });

    expect(result.current.streamingContent).toBe("Hello world");
    expect(result.current.isGenerating).toBe(true);

    await act(async () => {
      resolveGenerate(makeSummary({ content: "Hello world" }));
      await generating;
    });
  });

  it("drops chunks addressed to a different note", async () => {
    let resolveGenerate!: (s: Summary) => void;
    api.generateSummaryStream.mockImplementation(
      () => new Promise<Summary>((r) => (resolveGenerate = r))
    );

    const { result } = renderHook(() => useSummaries("note-1"));
    await waitFor(() => expect(bus.listenerCount(EVENT)).toBe(1));

    let generating!: Promise<unknown>;
    act(() => {
      generating = result.current.generateSummary("overview");
    });
    act(() => {
      bus.emit(EVENT, { note_id: "other", chunk: "not mine", is_done: false });
    });

    expect(result.current.streamingContent).toBe("");

    await act(async () => {
      resolveGenerate(makeSummary());
      await generating;
    });
  });

  it("clears streaming content on the done event", async () => {
    let resolveGenerate!: (s: Summary) => void;
    api.generateSummaryStream.mockImplementation(
      () => new Promise<Summary>((r) => (resolveGenerate = r))
    );

    const { result } = renderHook(() => useSummaries("note-1"));
    await waitFor(() => expect(bus.listenerCount(EVENT)).toBe(1));

    let generating!: Promise<unknown>;
    act(() => {
      generating = result.current.generateSummary("overview");
    });
    act(() => {
      bus.emit(EVENT, { note_id: "note-1", chunk: "partial", is_done: false });
    });
    act(() => {
      bus.emit(EVENT, { note_id: "note-1", chunk: "", is_done: true });
    });

    expect(result.current.streamingContent).toBe("");

    await act(async () => {
      resolveGenerate(makeSummary());
      await generating;
    });
  });

  it("unsubscribes on unmount", async () => {
    const { unmount } = renderHook(() => useSummaries("note-1"));
    await waitFor(() => expect(bus.listenerCount(EVENT)).toBe(1));

    unmount();
    await waitFor(() => expect(bus.listenerCount(EVENT)).toBe(0));
  });
});

describe("useSummaries — generate", () => {
  it("prepends the new summary and clears the streaming buffer", async () => {
    const existing = makeSummary({ id: 1 });
    const fresh = makeSummary({ id: 2, content: "New" });
    api.getNoteSummaries.mockResolvedValue([existing]);
    api.generateSummaryStream.mockResolvedValue(fresh);

    const { result } = renderHook(() => useSummaries("note-1"));
    await waitFor(() => expect(result.current.summaries).toEqual([existing]));

    await act(async () => {
      await result.current.generateSummary("overview");
    });

    expect(result.current.summaries.map((s) => s.id)).toEqual([2, 1]);
    expect(result.current.streamingContent).toBe("");
    expect(result.current.isGenerating).toBe(false);
  });

  it("errors without a note instead of calling the API", async () => {
    const { result } = renderHook(() => useSummaries(null));

    let returned: unknown = "unset";
    await act(async () => {
      returned = await result.current.generateSummary("overview");
    });

    expect(returned).toBeNull();
    expect(result.current.error).toBe("No note selected");
    expect(api.generateSummaryStream).not.toHaveBeenCalled();
  });

  it("clears isGenerating when generation fails", async () => {
    api.generateSummaryStream.mockRejectedValue(new Error("ollama down"));

    const { result } = renderHook(() => useSummaries("note-1"));
    await act(async () => {
      await result.current.generateSummary("overview");
    });

    expect(result.current.error).toBe("ollama down");
    expect(result.current.isGenerating).toBe(false);
  });

  it("passes a custom prompt through", async () => {
    api.generateSummaryStream.mockResolvedValue(makeSummary());

    const { result } = renderHook(() => useSummaries("note-1"));
    await act(async () => {
      await result.current.generateSummary("custom", "Summarise as bullets");
    });

    expect(api.generateSummaryStream).toHaveBeenCalledWith(
      "note-1",
      "custom",
      "Summarise as bullets"
    );
  });
});

describe("useSummaries — delete", () => {
  it("removes the deleted summary from the list", async () => {
    api.getNoteSummaries.mockResolvedValue([
      makeSummary({ id: 1 }),
      makeSummary({ id: 2 }),
    ]);

    const { result } = renderHook(() => useSummaries("note-1"));
    await waitFor(() => expect(result.current.summaries).toHaveLength(2));

    await act(async () => {
      await result.current.deleteSummary(1);
    });

    expect(result.current.summaries.map((s) => s.id)).toEqual([2]);
  });

  it("records an error and keeps the list on failure", async () => {
    api.getNoteSummaries.mockResolvedValue([makeSummary({ id: 1 })]);
    api.deleteSummary.mockRejectedValue(new Error("locked"));

    const { result } = renderHook(() => useSummaries("note-1"));
    await waitFor(() => expect(result.current.summaries).toHaveLength(1));

    await act(async () => {
      await result.current.deleteSummary(1);
    });

    expect(result.current.error).toBe("locked");
    expect(result.current.summaries).toHaveLength(1);
  });
});
