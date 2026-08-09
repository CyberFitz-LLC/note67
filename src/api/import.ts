import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { TranscriptVersion } from "../types";

export interface ImportResult {
  noteId: string;
  title: string;
  segmentCount: number;
  version?: TranscriptVersion;
  /** Speakers found in the file, so the UI can show what was recognised */
  speakers: string[];
}

export const importApi = {
  /**
   * Pick a WebVTT transcript and import it as a new note.
   *
   * Returns null when the picker is dismissed. Anything else — a file that is
   * not VTT, or one with no cues — rejects, because importing it would create
   * a note whose transcript attests nothing.
   */
  selectAndImportVtt: async (
    title?: string,
    sourceTool?: string
  ): Promise<ImportResult | null> => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Transcript", extensions: ["vtt"] }],
    });

    if (!selected) return null;

    // The backend reads the file. The webview's filesystem scope is limited to
    // the app's own data directory on purpose, and a picked transcript lives
    // wherever the user keeps it.
    return invoke<ImportResult>("import_vtt_transcript", {
      path: String(selected),
      title: title ?? "",
      sourceTool: sourceTool ?? null,
    });
  },
};
