import { create } from "zustand";

/**
 * UI state around summary generation.
 *
 * `refreshKey` is the counter that tells `useSummaries` to refetch. It lives
 * here rather than in App so components that need to react to a regenerated
 * summary can subscribe directly instead of having the counter threaded down.
 *
 * It is still a refresh counter, not a cache: fully retiring the pattern means
 * lifting `useSummaries`' own state into a store so writers can update the list
 * directly. That is the remaining half of this cleanup.
 */
export interface SummaryUiState {
  /** True while an AI title/summary regeneration is in flight. */
  isGeneratingTitle: boolean;
  /** Bumped to force `useSummaries` to refetch. */
  refreshKey: number;

  setGeneratingTitle: (value: boolean) => void;
  bumpRefreshKey: () => void;
}

const initial = {
  isGeneratingTitle: false,
  refreshKey: 0,
};

export const useSummaryUiStore = create<SummaryUiState>((set) => ({
  ...initial,
  setGeneratingTitle: (isGeneratingTitle) => set({ isGeneratingTitle }),
  bumpRefreshKey: () => set((state) => ({ refreshKey: state.refreshKey + 1 })),
}));

/** Restore the initial state — for tests. */
export function resetSummaryUiStore() {
  useSummaryUiStore.setState({ ...initial });
}
