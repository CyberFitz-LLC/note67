import { create } from "zustand";
import type { TranscriptSegment } from "../types";

/**
 * Live transcription state, lifted out of `useLiveTranscription` so components
 * can read it without prop drilling.
 *
 * Ownership mirrors the recording store: `useLiveTranscription()` is the
 * **single writer** — it owns the `transcription-update` subscription and must
 * be called in exactly one place (App), or events would be handled twice.
 * Read-only consumers subscribe here instead.
 */
export interface LiveTranscriptionState {
  isLiveTranscribing: boolean;
  liveSegments: TranscriptSegment[];
  error: string | null;
  /**
   * Which note the post-stop auto-retranscribe pass is running for. Like
   * `recordingNoteId`, this is global state driving per-note UI, so consumers
   * compare it against their own note id rather than treating it as a boolean.
   */
  retranscribingNoteId: string | null;

  setLiveTranscribing: (value: boolean) => void;
  setLiveSegments: (
    segments: TranscriptSegment[] | ((prev: TranscriptSegment[]) => TranscriptSegment[])
  ) => void;
  setError: (error: string | null) => void;
  setRetranscribingNoteId: (noteId: string | null) => void;
}

const initial = {
  isLiveTranscribing: false,
  liveSegments: [] as TranscriptSegment[],
  error: null as string | null,
  retranscribingNoteId: null as string | null,
};

export const useLiveTranscriptionStore = create<LiveTranscriptionState>((set) => ({
  ...initial,
  setLiveTranscribing: (isLiveTranscribing) => set({ isLiveTranscribing }),
  setLiveSegments: (segments) =>
    set((state) => ({
      liveSegments:
        typeof segments === "function" ? segments(state.liveSegments) : segments,
    })),
  setError: (error) => set({ error }),
  setRetranscribingNoteId: (retranscribingNoteId) => set({ retranscribingNoteId }),
}));

/** Restore the initial state — for tests. */
export function resetLiveTranscriptionStore() {
  useLiveTranscriptionStore.setState({ ...initial });
}
