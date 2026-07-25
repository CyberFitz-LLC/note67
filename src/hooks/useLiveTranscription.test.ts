import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";

import { useLiveTranscription } from "./useTranscription";
import { transcriptionApi } from "../api";
import { useWhisperStore } from "../stores/whisperStore";
import type { EventBusModule } from "../test/eventBus";

vi.mock("@tauri-apps/api/event", async () => {
  const { createEventBus } = await import("../test/eventBus");
  return createEventBus();
});

vi.mock("../api", () => ({
  transcriptionApi: {
    startLiveTranscription: vi.fn(),
    stopLiveTranscription: vi.fn(),
    isLiveTranscribing: vi.fn(),
    isTranscribing: vi.fn(),
    getTranscript: vi.fn(),
    transcribeAudio: vi.fn(),
    transcribeDualAudio: vi.fn(),
  },
}));

const api = vi.mocked(transcriptionApi, true);
const eventModule = (await import("@tauri-apps/api/event")) as unknown as EventBusModule;
const bus = eventModule.__bus;

const EVENT = "transcription-update";

/** Build a transcription-update payload. */
function update(
  noteId: string,
  segments: Array<{ start_time: number; end_time: number; text: string }>,
  extra: { is_final?: boolean; audio_source?: "mic" | "system" } = {}
) {
  return {
    note_id: noteId,
    segments,
    is_final: extra.is_final ?? false,
    audio_source: extra.audio_source,
  };
}

const seg = (text: string, start = 0, end = 1) => ({
  start_time: start,
  end_time: end,
  text,
});

beforeEach(() => {
  vi.clearAllMocks();
  bus.reset();
  api.isLiveTranscribing.mockResolvedValue(false);
  api.startLiveTranscription.mockResolvedValue(undefined);
  api.stopLiveTranscription.mockResolvedValue({
    segments: [],
    full_text: "",
    language: null,
  });
  useWhisperStore.setState({ language: "auto" });
});

/** Render and wait until the listener has attached. */
async function renderListening() {
  const view = renderHook(() => useLiveTranscription());
  await waitFor(() => expect(bus.listenerCount(EVENT)).toBeGreaterThan(0));
  return view;
}

describe("useLiveTranscription — event subscription lifecycle", () => {
  it("subscribes on mount and unsubscribes on unmount", async () => {
    const { unmount } = await renderListening();
    expect(bus.listenerCount(EVENT)).toBe(1);

    unmount();
    await waitFor(() => expect(bus.listenerCount(EVENT)).toBe(0));
  });
});

describe("useLiveTranscription — segment accumulation", () => {
  it("ignores events until a note is started", async () => {
    const { result } = await renderListening();

    act(() => {
      bus.emit(EVENT, update("note-1", [seg("stray")]));
    });

    expect(result.current.liveSegments).toEqual([]);
  });

  it("appends segments for the current note", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("hello"), seg("world", 1, 2)]));
    });

    expect(result.current.liveSegments.map((s) => s.text)).toEqual([
      "hello",
      "world",
    ]);
    expect(result.current.isLiveTranscribing).toBe(true);
  });

  it("accumulates across multiple events rather than replacing", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("one")]));
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("two", 1, 2)]));
    });

    expect(result.current.liveSegments.map((s) => s.text)).toEqual(["one", "two"]);
  });

  it("drops events addressed to a different note", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    act(() => {
      bus.emit(EVENT, update("other-note", [seg("not mine")]));
    });

    expect(result.current.liveSegments).toEqual([]);
  });

  it("assigns unique ids so React keys never collide", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("a"), seg("b")]));
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("c")]));
    });

    const ids = result.current.liveSegments.map((s) => s.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("ignores an event carrying no segments", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("kept")]));
    });
    act(() => {
      bus.emit(EVENT, update("note-1", []));
    });

    expect(result.current.liveSegments.map((s) => s.text)).toEqual(["kept"]);
  });
});

describe("useLiveTranscription — speaker labelling", () => {
  it("labels system audio as Others", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1", "Chi");
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("them")], { audio_source: "system" }));
    });

    expect(result.current.liveSegments[0].speaker).toBe("Others");
  });

  it("labels mic audio with the supplied speaker name", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1", "Chi");
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("me")], { audio_source: "mic" }));
    });

    expect(result.current.liveSegments[0].speaker).toBe("Chi");
  });

  it("defaults the mic speaker to 'Me' when no name is given", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("me")]));
    });

    expect(result.current.liveSegments[0].speaker).toBe("Me");
  });

  it("marks live segments with source_type 'live'", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("x")]));
    });

    expect(result.current.liveSegments[0].source_type).toBe("live");
  });
});

describe("useLiveTranscription — start/stop", () => {
  it("seeds initial segments when resuming an existing note", async () => {
    const { result } = await renderListening();
    const existing = [
      {
        id: 99,
        note_id: "note-1",
        start_time: 0,
        end_time: 1,
        text: "earlier",
        speaker: "Me",
        source_type: "live",
        source_id: null,
        created_at: "2026-07-02T09:00:00.000Z",
      },
    ];

    await act(async () => {
      await result.current.startLiveTranscription("note-1", "Me", existing);
    });

    expect(result.current.liveSegments.map((s) => s.text)).toEqual(["earlier"]);
  });

  it("passes an explicit language through and omits 'auto'", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    expect(api.startLiveTranscription).toHaveBeenLastCalledWith("note-1", undefined);

    useWhisperStore.setState({ language: "en" });
    await act(async () => {
      await result.current.startLiveTranscription("note-2");
    });
    expect(api.startLiveTranscription).toHaveBeenLastCalledWith("note-2", "en");
  });

  it("stops transcribing when a final event arrives", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    expect(result.current.isLiveTranscribing).toBe(true);

    act(() => {
      bus.emit(EVENT, update("note-1", [seg("last")], { is_final: true }));
    });

    expect(result.current.isLiveTranscribing).toBe(false);
    expect(result.current.liveSegments.map((s) => s.text)).toEqual(["last"]);
  });

  it("stops listening to the note after stopLiveTranscription", async () => {
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });
    await act(async () => {
      await result.current.stopLiveTranscription("note-1");
    });

    act(() => {
      bus.emit(EVENT, update("note-1", [seg("too late")]));
    });

    expect(result.current.isLiveTranscribing).toBe(false);
    expect(result.current.liveSegments).toEqual([]);
  });

  it("records a start failure and does not latch onto the note", async () => {
    api.startLiveTranscription.mockRejectedValue(new Error("no model"));
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });

    expect(result.current.error).toBe("no model");
    expect(result.current.isLiveTranscribing).toBe(false);

    // currentNoteId was reset, so later events are ignored.
    act(() => {
      bus.emit(EVENT, update("note-1", [seg("ignored")]));
    });
    expect(result.current.liveSegments).toEqual([]);
  });

  it("records a stop failure and returns null", async () => {
    api.stopLiveTranscription.mockRejectedValue(new Error("busy"));
    const { result } = await renderListening();

    await act(async () => {
      await result.current.startLiveTranscription("note-1");
    });

    let returned: unknown = "unset";
    await act(async () => {
      returned = await result.current.stopLiveTranscription("note-1");
    });

    expect(returned).toBeNull();
    expect(result.current.error).toBe("busy");
  });
});
