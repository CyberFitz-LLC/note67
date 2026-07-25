import { create } from "zustand";
import { RecordingPhase } from "../types";

export type RecordingMode = "idle" | "dual" | "mic-only" | "system-only";

/**
 * Recording state, lifted out of `useRecording` so any component can read it
 * without having it threaded down as props.
 *
 * Ownership: `useRecording()` is the **single writer** — it owns the actions and
 * the level-polling effect and is called once, from App. Everything else should
 * read this store directly (e.g. `useRecordingStore((s) => s.isRecording)`) and
 * never call `useRecording()`, or the polling effect would run more than once.
 */
export interface RecordingStoreState {
  isRecording: boolean;
  isPaused: boolean;
  recordingPhase: RecordingPhase;
  audioLevel: number;
  audioPath: string | null;
  error: string | null;
  recordingMode: RecordingMode;
  /**
   * Which note is being recorded. Recording state is global but the UI is
   * per-note, so consumers scope on this (`isRecording && recordingNoteId ===
   * note.id`) rather than reading `isRecording` alone — otherwise every note
   * would look like it was recording.
   */
  recordingNoteId: string | null;

  /** Patch any subset of the state (used by useRecording). */
  patch: (partial: Partial<RecordingSnapshot>) => void;
  /** Set (or clear) the note currently being recorded. */
  setRecordingNoteId: (noteId: string | null) => void;
  /** Return to the idle state after a recording stops. */
  reset: () => void;
}

export type RecordingSnapshot = Omit<
  RecordingStoreState,
  "patch" | "setRecordingNoteId" | "reset"
>;

const initial: RecordingSnapshot = {
  isRecording: false,
  isPaused: false,
  recordingPhase: RecordingPhase.Idle,
  audioLevel: 0,
  audioPath: null,
  error: null,
  recordingMode: "idle",
  recordingNoteId: null,
};

export const useRecordingStore = create<RecordingStoreState>((set) => ({
  ...initial,
  patch: (partial) => set(partial),
  setRecordingNoteId: (recordingNoteId) => set({ recordingNoteId }),
  reset: () => set({ ...initial }),
}));

/** Restore the initial state — for tests. */
export function resetRecordingStore() {
  useRecordingStore.setState({ ...initial });
}
