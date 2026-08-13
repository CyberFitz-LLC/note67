import { invoke } from "@tauri-apps/api/core";

/**
 * What a node said when asked to attest a meeting.
 *
 * `pending` and `denied` are deliberately distinct. Pending means the node was
 * never asked — offline, unreachable, having a bad day — and the transcript is
 * untouched, so retrying is the right move. Denied means it was asked and said
 * no, and retrying an unchanged request would only ask again.
 */
export type Attestation =
  | { status: "attested"; receiptHash: string }
  | { status: "pending"; reason: string }
  | { status: "denied"; reason: string };

export const exochainApi = {
  /**
   * Ask a node to attest this note's current transcript.
   *
   * A receipt is recorded only if one was minted. Nothing here fabricates a
   * hash, and an unreachable node leaves the note exactly as it was.
   */
  attestMeeting: (noteId: string): Promise<Attestation> =>
    invoke<Attestation>("attest_meeting", { noteId }),
};
