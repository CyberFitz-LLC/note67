import { describe, it, expect, beforeEach, vi } from "vitest";

import { useLinksStore } from "./linksStore";
import { linksApi } from "../api/links";
import type { BacklinkNote } from "../types";

vi.mock("../api/links", () => ({
  linksApi: {
    getBacklinks: vi.fn(),
  },
}));

const api = vi.mocked(linksApi, true);

const initialState = useLinksStore.getState();

function backlink(id: string): BacklinkNote {
  return {
    id,
    title: `Note ${id}`,
    description: null,
    started_at: "2026-07-02T09:00:00.000Z",
  };
}

/** A promise plus its resolver, so a fetch can be held open mid-flight. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.clearAllMocks();
  useLinksStore.setState(initialState, true);
});

describe("linksStore.fetchBacklinks", () => {
  it("loads backlinks for a note", async () => {
    api.getBacklinks.mockResolvedValue([backlink("a")]);

    await useLinksStore.getState().fetchBacklinks("note-1");

    const state = useLinksStore.getState();
    expect(state.backlinks).toEqual([backlink("a")]);
    expect(state.currentNoteId).toBe("note-1");
    expect(state.loading).toBe(false);
    expect(state.error).toBeNull();
  });

  it("records an error without leaving loading stuck on", async () => {
    api.getBacklinks.mockRejectedValue(new Error("db gone"));

    await useLinksStore.getState().fetchBacklinks("note-1");

    const state = useLinksStore.getState();
    expect(state.error).toContain("db gone");
    expect(state.loading).toBe(false);
  });
});

describe("linksStore — race guard when switching notes", () => {
  it("discards a slow response for a note the user has navigated away from", async () => {
    const slow = deferred<BacklinkNote[]>();
    const fast = deferred<BacklinkNote[]>();

    api.getBacklinks.mockImplementationOnce(() => slow.promise);
    api.getBacklinks.mockImplementationOnce(() => fast.promise);

    // Start note-1, then switch to note-2 before note-1 comes back.
    const firstFetch = useLinksStore.getState().fetchBacklinks("note-1");
    const secondFetch = useLinksStore.getState().fetchBacklinks("note-2");

    fast.resolve([backlink("from-note-2")]);
    await secondFetch;

    // note-1's response lands late and must be ignored.
    slow.resolve([backlink("from-note-1")]);
    await firstFetch;

    const state = useLinksStore.getState();
    expect(state.currentNoteId).toBe("note-2");
    expect(state.backlinks).toEqual([backlink("from-note-2")]);
  });

  it("discards a late error for a note that is no longer current", async () => {
    const slow = deferred<BacklinkNote[]>();
    let rejectSlow!: (e: Error) => void;
    const slowRejectable = new Promise<BacklinkNote[]>((_, reject) => {
      rejectSlow = reject;
    });
    void slow;

    api.getBacklinks.mockImplementationOnce(() => slowRejectable);
    api.getBacklinks.mockImplementationOnce(() =>
      Promise.resolve([backlink("from-note-2")])
    );

    const firstFetch = useLinksStore.getState().fetchBacklinks("note-1");
    await useLinksStore.getState().fetchBacklinks("note-2");

    rejectSlow(new Error("late failure"));
    await firstFetch;

    const state = useLinksStore.getState();
    // The stale failure must not blank out note-2's good data.
    expect(state.error).toBeNull();
    expect(state.backlinks).toEqual([backlink("from-note-2")]);
  });

  it("skips a duplicate in-flight fetch for the same note", async () => {
    const pending = deferred<BacklinkNote[]>();
    api.getBacklinks.mockImplementation(() => pending.promise);

    const first = useLinksStore.getState().fetchBacklinks("note-1");
    // Second call for the same note while the first is still loading.
    await useLinksStore.getState().fetchBacklinks("note-1");

    expect(api.getBacklinks).toHaveBeenCalledTimes(1);

    pending.resolve([backlink("a")]);
    await first;
  });
});

describe("linksStore.clearBacklinks", () => {
  it("resets backlinks, current note and error", async () => {
    api.getBacklinks.mockResolvedValue([backlink("a")]);
    await useLinksStore.getState().fetchBacklinks("note-1");

    useLinksStore.getState().clearBacklinks();

    const state = useLinksStore.getState();
    expect(state.backlinks).toEqual([]);
    expect(state.currentNoteId).toBeNull();
    expect(state.error).toBeNull();
  });
});
