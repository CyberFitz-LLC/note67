import { create } from "zustand";

export type NoteTab = "note" | "transcript" | "summary" | "tasks";

/**
 * View state for the note pane.
 *
 * Unlike the recording/transcription stores this is not lifted domain state —
 * it is genuinely UI state. It lives here because both App and NoteView drive
 * it: App switches tabs when starting a recording or navigating in from the
 * graph/Tasks views, while NoteView switches them from the tab bar. Threading
 * that through props meant a value and a setter prop for each one.
 */
export interface NoteUiState {
  activeTab: NoteTab;
  editingTitle: boolean;
  showAISidebar: boolean;
  /** Task to focus when the note is opened from the global Tasks view. */
  focusTaskId: number | null;

  setActiveTab: (tab: NoteTab) => void;
  setEditingTitle: (editing: boolean) => void;
  toggleAISidebar: () => void;
  setShowAISidebar: (show: boolean) => void;
  setFocusTaskId: (id: number | null) => void;
}

const initial = {
  activeTab: "summary" as NoteTab,
  editingTitle: false,
  showAISidebar: false,
  focusTaskId: null as number | null,
};

export const useNoteUiStore = create<NoteUiState>((set) => ({
  ...initial,
  setActiveTab: (activeTab) => set({ activeTab }),
  setEditingTitle: (editingTitle) => set({ editingTitle }),
  toggleAISidebar: () => set((state) => ({ showAISidebar: !state.showAISidebar })),
  setShowAISidebar: (showAISidebar) => set({ showAISidebar }),
  setFocusTaskId: (focusTaskId) => set({ focusTaskId }),
}));

/** Restore the initial state — for tests. */
export function resetNoteUiStore() {
  useNoteUiStore.setState({ ...initial });
}
