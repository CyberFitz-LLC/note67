import { useCallback, useEffect, useState } from "react";

import { transcriptionApi } from "../api";
import type { TranscriptChain } from "../types";

/**
 * A note's transcript version history.
 *
 * Reloads when `refreshKey` changes, which is how the caller signals that a
 * transcript was rewritten — the chain gains a version on re-transcription, but
 * only when the text actually changed.
 */
export function useTranscriptChain(
  noteId: string | null,
  refreshKey: string | number = 0
) {
  const [chain, setChain] = useState<TranscriptChain | null>(null);
  // Starts true and is only ever cleared in the async continuation. Setting it
  // in the effect body would cascade a render, and on a refresh the previous
  // chain stays on screen rather than flashing empty.
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (id: string) => {
    try {
      setChain(await transcriptionApi.getTranscriptChain(id));
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    if (!noteId) return;
    let cancelled = false;
    transcriptionApi
      .getTranscriptChain(noteId)
      .then((next) => {
        if (cancelled) return;
        setChain(next);
        setError(null);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [noteId, refreshKey]);

  const refresh = useCallback(async () => {
    if (noteId) await load(noteId);
  }, [noteId, load]);

  // Derived rather than cleared in the effect: with no note there is nothing to
  // show, and setting state synchronously in the effect body would cascade a
  // render for a value we can just compute.
  return { chain: noteId ? chain : null, loading, error, refresh };
}
