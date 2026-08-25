import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface Screenshot {
  id: number;
  note_id: string;
  file_path: string;
  captured_at_ms: number;
  caption: string | null;
  extracted_text: string | null;
  created_at: string;
}

/**
 * Screenshots pasted into a meeting.
 *
 * `captured_at_ms` is the position in the note, not a wall clock, so a slide
 * sits in the transcript where it appeared rather than at the end of a list.
 * When a recording is running that is the elapsed time; when it is not, the
 * image lands at the end of what has been recorded so far.
 */
export function useScreenshots(noteId: string | null) {
  const [screenshots, setScreenshots] = useState<Screenshot[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!noteId) {
      setScreenshots([]);
      return;
    }
    try {
      setScreenshots(await invoke<Screenshot[]>("list_screenshots", { noteId }));
    } catch (e) {
      setError(String(e));
    }
  }, [noteId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const add = useCallback(
    async (bytes: Uint8Array, capturedAtMs: number) => {
      if (!noteId) return null;
      setBusy(true);
      setError(null);
      try {
        // Chunked, because spreading a multi-megabyte array into
        // String.fromCharCode in one call overflows the argument limit and
        // throws on exactly the large screenshots this feature is for.
        let binary = "";
        const CHUNK = 0x8000;
        for (let i = 0; i < bytes.length; i += CHUNK) {
          binary += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
        }
        const shot = await invoke<Screenshot>("add_screenshot", {
          noteId,
          imageBase64: btoa(binary),
          capturedAtMs: Math.max(0, Math.round(capturedAtMs)),
          caption: null,
        });
        setScreenshots((prev) =>
          [...prev, shot].sort((a, b) => a.captured_at_ms - b.captured_at_ms),
        );
        return shot;
      } catch (e) {
        setError(String(e));
        return null;
      } finally {
        setBusy(false);
      }
    },
    [noteId],
  );

  const extract = useCallback(async (id: number) => {
    setError(null);
    try {
      const text = await invoke<string>("extract_screenshot_text", { id });
      setScreenshots((prev) =>
        prev.map((s) => (s.id === id ? { ...s, extracted_text: text } : s)),
      );
      return text;
    } catch (e) {
      setError(String(e));
      return null;
    }
  }, []);

  const remove = useCallback(async (id: number) => {
    try {
      await invoke("delete_screenshot", { id });
      setScreenshots((prev) => prev.filter((s) => s.id !== id));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  return { screenshots, add, extract, remove, refresh, busy, error };
}

/**
 * Pull image bytes out of a paste.
 *
 * Returns null for a paste that carries no image, so a normal text paste is
 * left entirely alone — this hook must never swallow ordinary editing.
 */
export async function imageFromClipboard(
  event: ClipboardEvent,
): Promise<Uint8Array | null> {
  const items = event.clipboardData?.items;
  if (!items) return null;
  for (const item of items) {
    if (item.kind === "file" && item.type.startsWith("image/")) {
      const file = item.getAsFile();
      if (!file) continue;
      return new Uint8Array(await file.arrayBuffer());
    }
  }
  return null;
}
