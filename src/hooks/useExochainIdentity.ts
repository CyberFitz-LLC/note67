import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type Standing = "active" | "expired" | "notForMeetings";

export interface InstalledCredential {
  issuerDid: string;
  issuedAtMs: number;
  expiresAtMs: number;
  tools: string[];
  permissions: string[];
  dataClasses: string[];
  purpose: string;
  standing: Standing;
}

export interface ExochainIdentity {
  did: string;
  publicKeyHex: string;
  deviceId: string;
  serviceId: string;
  createdAt: string;
  /** True once a usable credential naming this DID has been installed. */
  enrolled: boolean;
  credential?: InstalledCredential;
}

/**
 * This installation's ExoChain identity, created on first read.
 *
 * The private key never leaves the machine; only the DID and public key are
 * exposed, because those are what a credential has to name.
 */
export function useExochainIdentity() {
  const [identity, setIdentity] = useState<ExochainIdentity | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Starts true so the panel never renders "no identity" during the first
  // read, which would look like a failure rather than a load.
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    invoke<ExochainIdentity>("get_exochain_identity")
      .then((value) => {
        if (cancelled) return;
        setIdentity(value);
        setError(null);
        setLoading(false);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
        setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  /**
   * Store a credential minted for this installation.
   *
   * Errors are returned rather than thrown: pasting the wrong file is the
   * likely mistake and the message names both DIDs, which is the thing that
   * lets someone find the right one.
   */
  const installCredential = useCallback(async (json: string) => {
    try {
      setIdentity(await invoke<ExochainIdentity>("install_exochain_credential", { json }));
      return null;
    } catch (e) {
      return String(e);
    }
  }, []);

  const removeCredential = useCallback(async () => {
    try {
      setIdentity(await invoke<ExochainIdentity>("remove_exochain_credential"));
      return null;
    } catch (e) {
      return String(e);
    }
  }, []);

  return { identity, error, loading, installCredential, removeCredential };
}

/**
 * The public key as the App Registry wants it.
 *
 * The registry takes base64 and the app stores hex, and it recomputes the DID
 * from whatever it is given — so a wrong encoding is rejected as a DID
 * mismatch, which reads as "your identity is wrong" rather than "your
 * encoding is wrong". Converting here keeps that confusion out of the flow.
 */
export function publicKeyBase64(hex: string): string {
  const clean = hex.trim();
  if (clean.length % 2 !== 0 || /[^0-9a-fA-F]/.test(clean)) return "";
  const bytes = new Uint8Array(clean.length / 2);
  for (let i = 0; i < bytes.length; i += 1) {
    bytes[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  }
  let binary = "";
  bytes.forEach((b) => {
    binary += String.fromCharCode(b);
  });
  return btoa(binary);
}
