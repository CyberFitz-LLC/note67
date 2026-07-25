import type {
  TranscriptSegment,
  AudioSegment,
  UploadedAudio,
  AudioItem,
} from "../types";

/** Grouping of transcript segments by the audio source that produced them.
 *  Lives outside TranscriptSearch.tsx so it can be unit-tested without
 *  rendering, and so that file only exports its component (Fast Refresh). */

export interface AudioSourceSection {
  key: string;
  label: string;
  sourceType: string | null;
  sourceId: number | null;
  displayOrder: number;
  transcripts: TranscriptSegment[];
}

export function groupTranscriptsBySource(
  segments: TranscriptSegment[],
  audioSegments: AudioSegment[],
  uploads: UploadedAudio[]
): AudioSourceSection[] {
  // Build audio items list sorted by display_order
  const audioItems: AudioItem[] = [
    ...audioSegments.map((s) => ({ type: "segment" as const, data: s })),
    ...uploads.map((u) => ({ type: "upload" as const, data: u })),
  ].sort((a, b) => a.data.display_order - b.data.display_order);

  // Create a map for quick lookup of display_order
  const orderMap = new Map<string, { order: number; label: string }>();
  audioItems.forEach((item, index) => {
    if (item.type === "segment") {
      const key = `segment-${item.data.id}`;
      orderMap.set(key, {
        order: index,
        label: `Recording ${item.data.segment_index + 1}`,
      });
    } else {
      const key = `upload-${item.data.id}`;
      orderMap.set(key, {
        order: index,
        label: item.data.original_filename,
      });
    }
  });

  // Group transcripts by source
  const sourceGroups = new Map<string, AudioSourceSection>();

  for (const segment of segments) {
    let key: string;
    let label: string;
    let displayOrder: number;

    if (segment.source_type === "upload" && segment.source_id !== null) {
      key = `upload-${segment.source_id}`;
      const info = orderMap.get(key);
      label = info?.label || "Uploaded Audio";
      displayOrder = info?.order ?? 999;
    } else if (segment.source_type === "segment" && segment.source_id !== null) {
      key = `segment-${segment.source_id}`;
      const info = orderMap.get(key);
      label = info?.label || "Recording";
      displayOrder = info?.order ?? 999;
    } else if (segment.source_type === "live") {
      // Live transcripts - group with the current recording session
      key = "live";
      label = "Live Transcription";
      // Last, not first. What is being recorded now comes after everything
      // already in the note, so continuing a recording appends below rather
      // than pushing new text above the earlier sessions. It also has to agree
      // with the auto-scroll below, which scrolls to the bottom on new content:
      // pinning live to the top made that scroll away from the very text it was
      // trying to follow.
      displayOrder = Number.MAX_SAFE_INTEGER;
    } else {
      // Legacy transcripts without source info
      key = "legacy";
      label = "Transcript";
      displayOrder = 1000;
    }

    if (!sourceGroups.has(key)) {
      sourceGroups.set(key, {
        key,
        label,
        sourceType: segment.source_type,
        sourceId: segment.source_id,
        displayOrder,
        transcripts: [],
      });
    }
    sourceGroups.get(key)!.transcripts.push(segment);
  }

  // Sort sections by display_order, then sort transcripts within each section by start_time
  const sections = Array.from(sourceGroups.values())
    .sort((a, b) => a.displayOrder - b.displayOrder)
    .map((section) => ({
      ...section,
      transcripts: section.transcripts.sort((a, b) => a.start_time - b.start_time),
    }));

  return sections;
}
