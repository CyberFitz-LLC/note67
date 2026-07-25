import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";

import { useTranscription } from "./useTranscription";
import { transcriptionApi } from "../api";
import type { TranscriptSegment } from "../types";

vi.mock("@tauri-apps/api/event", async () => {
  const { createEventBus } = await import("../test/eventBus");
  return createEventBus();
});

vi.mock("../api", () => ({
  transcriptionApi: {
    transcribeAudio: vi.fn(),
    transcribeDualAudio: vi.fn(),
    getTranscript: vi.fn(),
    isTranscribing: vi.fn(),
    isLiveTranscribing: vi.fn(),
    startLiveTranscription: vi.fn(),
    stopLiveTranscription: vi.fn(),
  },
}));

const api = vi.mocked(transcriptionApi, true);

function storedSegment(text: string, speaker: string | null): TranscriptSegment {
  return {
    id: 1,
    note_id: "note-1",
    start_time: 0,
    end_time: 1,
    text,
    speaker,
    source_type: "segment",
    source_id: 1,
    created_at: "2026-07-02T09:00:00.000Z",
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  api.isTranscribing.mockResolvedValue(false);
  api.getTranscript.mockResolvedValue([]);
});

describe("useTranscription — transcribe", () => {
  it("maps raw result segments into TranscriptSegment shape", async () => {
    api.transcribeAudio.mockResolvedValue({
      full_text: "hello world",
      language: "en",
      segments: [
        { start_time: 0, end_time: 1, text: "hello" },
        { start_time: 1, end_time: 2, text: "world" },
      ],
    });

    const { result } = renderHook(() => useTranscription());
    await act(async () => {
      await result.current.transcribe("/a.wav", "note-1");
    });

    expect(result.current.transcript).toHaveLength(2);
    expect(result.current.transcript[0]).toMatchObject({
      id: 0,
      note_id: "note-1",
      text: "hello",
      speaker: null,
      source_type: null,
    });
    expect(result.current.transcript[1].id).toBe(1);
  });

  it("clears isTranscribing even when transcription fails", async () => {
    api.transcribeAudio.mockRejectedValue(new Error("model missing"));

    const { result } = renderHook(() => useTranscription());
    let returned: unknown = "unset";
    await act(async () => {
      returned = await result.current.transcribe("/a.wav", "note-1");
    });

    expect(returned).toBeNull();
    expect(result.current.error).toBe("model missing");
    expect(result.current.isTranscribing).toBe(false);
  });
});

describe("useTranscription — transcribeDual", () => {
  it("reloads the stored transcript so both speakers are present", async () => {
    api.transcribeDualAudio.mockResolvedValue({
      micResult: { segments: [], full_text: "me", language: "en" },
      systemResult: { segments: [], full_text: "them", language: "en" },
      totalSegments: 2,
    });
    api.getTranscript.mockResolvedValue([
      storedSegment("me", "You"),
      storedSegment("them", "Others"),
    ]);

    const { result } = renderHook(() => useTranscription());
    await act(async () => {
      await result.current.transcribeDual("/mic.wav", "/sys.wav", "note-1");
    });

    // The dual result itself has no segments — the transcript comes from the DB.
    expect(api.getTranscript).toHaveBeenCalledWith("note-1");
    expect(result.current.transcript.map((s) => s.speaker)).toEqual([
      "You",
      "Others",
    ]);
  });

  it("records an error and returns null on failure", async () => {
    api.transcribeDualAudio.mockRejectedValue(new Error("bad wav"));

    const { result } = renderHook(() => useTranscription());
    let returned: unknown = "unset";
    await act(async () => {
      returned = await result.current.transcribeDual("/mic.wav", null, "note-1");
    });

    expect(returned).toBeNull();
    expect(result.current.error).toBe("bad wav");
    expect(result.current.isTranscribing).toBe(false);
  });
});

describe("useTranscription — loadTranscript", () => {
  it("loads and returns stored segments", async () => {
    const segments = [storedSegment("stored", "You")];
    api.getTranscript.mockResolvedValue(segments);

    const { result } = renderHook(() => useTranscription());
    let returned: TranscriptSegment[] = [];
    await act(async () => {
      returned = await result.current.loadTranscript("note-1");
    });

    expect(returned).toEqual(segments);
    expect(result.current.transcript).toEqual(segments);
  });

  it("returns an empty array and records the error on failure", async () => {
    api.getTranscript.mockRejectedValue(new Error("no rows"));

    const { result } = renderHook(() => useTranscription());
    let returned: TranscriptSegment[] = [storedSegment("stale", null)];
    await act(async () => {
      returned = await result.current.loadTranscript("note-1");
    });

    expect(returned).toEqual([]);
    expect(result.current.error).toBe("no rows");
  });
});

describe("useTranscription — initial status", () => {
  it("reflects an in-progress transcription found on mount", async () => {
    api.isTranscribing.mockResolvedValue(true);

    const { result } = renderHook(() => useTranscription());
    await waitFor(() => expect(result.current.isTranscribing).toBe(true));
  });
});
