import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface AssistOption {
  label: string;
  angle: string;
}

interface AssistUpdate {
  note_id: string;
  brief: string | null;
  questions_open: string[];
  options: AssistOption[];
  raw: string | null;
  as_of_seconds: number;
}

interface AssistStarted {
  running: boolean;
  receipt: string | null;
  attestation_note: string | null;
}

/**
 * Live assistance for one meeting.
 *
 * Updates arrive as events rather than being polled, and each carries how far
 * through the meeting the model had read — kept and shown, because a pane that
 * is two minutes behind is worse than no pane only if you cannot tell.
 */
export function useAssist(noteId: string | null) {
  const [running, setRunning] = useState(false);
  const [brief, setBrief] = useState<string | null>(null);
  const [questions, setQuestions] = useState<string[]>([]);
  const [options, setOptions] = useState<AssistOption[]>([]);
  const [raw, setRaw] = useState<string | null>(null);
  const [asOf, setAsOf] = useState<number | null>(null);
  const [receipt, setReceipt] = useState<string | null>(null);
  const [attestationNote, setAttestationNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  useEffect(() => {
    let cancelled = false;
    listen<AssistUpdate>("assist-update", (event) => {
      if (cancelled) return;
      const u = event.payload;
      if (u.note_id !== noteId) return;
      // A brief-only update must not clear the suggestions beside it, and the
      // reverse. The two panes advance independently.
      if (u.brief !== null) setBrief(u.brief);
      if (u.options.length > 0 || u.questions_open.length > 0 || u.raw !== null) {
        setQuestions(u.questions_open);
        setOptions(u.options);
        setRaw(u.raw);
      }
      setAsOf(u.as_of_seconds);
    })
      .then((un) => {
        if (cancelled) un();
        else unlistenRef.current = un;
      })
      .catch((e) => !cancelled && setError(String(e)));

    return () => {
      cancelled = true;
      unlistenRef.current?.();
      unlistenRef.current = null;
    };
  }, [noteId]);

  const start = useCallback(async () => {
    if (!noteId) return;
    setError(null);
    try {
      const started = await invoke<AssistStarted>("start_assist", { noteId });
      setRunning(started.running);
      setReceipt(started.receipt);
      setAttestationNote(started.attestation_note);
    } catch (e) {
      setError(String(e));
    }
  }, [noteId]);

  const stop = useCallback(async () => {
    try {
      await invoke("stop_assist");
    } finally {
      setRunning(false);
    }
  }, []);

  const expand = useCallback(
    async (option: AssistOption) => {
      if (!noteId) return null;
      try {
        return await invoke<string>("expand_assist_option", {
          noteId,
          label: option.label,
          angle: option.angle,
        });
      } catch (e) {
        setError(String(e));
        return null;
      }
    },
    [noteId],
  );

  return {
    running,
    brief,
    questions,
    options,
    raw,
    asOf,
    receipt,
    attestationNote,
    error,
    start,
    stop,
    expand,
  };
}

/** How far behind the model is, in seconds, or null when it cannot be known. */
export function stalenessSeconds(
  asOfSeconds: number | null,
  meetingSeconds: number | null,
): number | null {
  if (asOfSeconds === null || meetingSeconds === null) return null;
  return Math.max(0, meetingSeconds - asOfSeconds);
}

/** Plain words for how current a pane is. */
export function freshnessLabel(behindSeconds: number | null): string {
  if (behindSeconds === null) return "waiting for the meeting";
  if (behindSeconds < 45) return "up to date";
  if (behindSeconds < 120) return `about a minute behind`;
  return `${Math.round(behindSeconds / 60)} minutes behind`;
}
