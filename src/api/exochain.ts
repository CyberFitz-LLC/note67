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

/**
 * The transcript in hand, compared against every version recorded for it.
 *
 * Distinct from the chain being intact: a chain can verify perfectly against
 * itself while the transcript underneath has been altered. This is the check a
 * receipt exists for, and it needs no node and no network.
 */
export type Verification =
  | { status: "empty" }
  | { status: "untracked" }
  | {
      status: "matches";
      version: number;
      attested: boolean;
      receiptHash?: string;
      isLatest: boolean;
    }
  | {
      status: "altered";
      expectedHash: string;
      actualHash: string;
      latestVersion: number;
    };

export const exochainApi = {
  /** Recompute the transcript's hash and compare it to what was recorded. */
  verifyTranscript: (noteId: string): Promise<Verification> =>
    invoke<Verification>("verify_transcript", { noteId }),

  /**
   * Ask a node to attest this note's current transcript.
   *
   * A receipt is recorded only if one was minted. Nothing here fabricates a
   * hash, and an unreachable node leaves the note exactly as it was.
   */
  attestMeeting: (noteId: string): Promise<Attestation> =>
    invoke<Attestation>("attest_meeting", { noteId }),
};
