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

/** Where two recordings of the same meeting disagree. */
export interface Conflict {
  startMs: number;
  text: string;
  detail: string;
}

export interface MergeEvidence {
  matched: number;
  agreeing: number;
  baseSegments: number;
  otherSegments: number;
  overlapMs: number;
}

export interface MergeOutcome {
  /** What the comparison found, present on a refusal as well as a merge. */
  evidence: MergeEvidence;
  /** How far the other recording's clock was from ours. Null when refused. */
  offsetMs: number | null;
  segmentsNamed: number;
  disagreements: number;
  /** True when the two did not look like the same meeting, so nothing changed. */
  rejected: boolean;
  version?: TranscriptVersion;
  conflicts: Conflict[];
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

  /**
   * Pick a transcript of a meeting this note already holds, and take its
   * speaker names.
   *
   * Distinct from importing: this note keeps its own text and timings — that
   * audio is what the receipts are about — and gains only the attribution it
   * cannot derive on its own.
   */
  selectAndMergeVtt: async (
    noteId: string,
    sourceTool?: string
  ): Promise<MergeOutcome | null> => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Transcript", extensions: ["vtt"] }],
    });

    if (!selected) return null;

    return invoke<MergeOutcome>("merge_transcript_into_note", {
      noteId,
      path: String(selected),
      sourceTool: sourceTool ?? null,
    });
  },

  /**
   * Set one segment's speaker by hand.
   *
   * Returns the chain version this appended, if the transcript changed.
   * Naming a speaker changes what the transcript says, so it is recorded like
   * any other edit.
   */
  setSegmentSpeaker: async (
    noteId: string,
    segmentId: number,
    speaker: string | null
  ): Promise<TranscriptVersion | null> => {
    return invoke<TranscriptVersion | null>("set_segment_speaker", {
      noteId,
      segmentId,
      speaker: speaker && speaker.trim() ? speaker.trim() : null,
    });
  },
};
