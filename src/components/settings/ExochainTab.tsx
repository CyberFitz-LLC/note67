import { useState } from "react";
import {
  publicKeyBase64,
  useExochainIdentity,
} from "../../hooks/useExochainIdentity";

function CopyField({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false);

  return (
    <div>
      <div
        className="text-xs mb-1"
        style={{ color: "var(--color-text-secondary)" }}
      >
        {label}
      </div>
      <div className="flex gap-2 items-center">
        <code
          className="text-xs px-2 py-1.5 rounded-lg flex-1 overflow-x-auto whitespace-nowrap"
          style={{ backgroundColor: "var(--color-bg-subtle)" }}
        >
          {value}
        </code>
        <button
          type="button"
          className="text-xs px-2 py-1.5 rounded-lg shrink-0"
          style={{ backgroundColor: "var(--color-bg-subtle)" }}
          onClick={() => {
            navigator.clipboard.writeText(value).then(
              () => {
                setCopied(true);
                setTimeout(() => setCopied(false), 1500);
              },
              () => setCopied(false),
            );
          }}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
    </div>
  );
}

export function ExochainTab() {
  const { identity, error, loading, installCredential, removeCredential } =
    useExochainIdentity();
  const [paste, setPaste] = useState("");
  const [installError, setInstallError] = useState<string | null>(null);

  return (
    <div className="space-y-6">
      <div>
        <h3
          className="text-sm font-semibold mb-3"
          style={{ color: "var(--color-text)" }}
        >
          Meeting receipts
        </h3>
        <p
          className="text-sm mb-4"
          style={{ color: "var(--color-text-secondary)" }}
        >
          This installation has its own signing key. Its identity is shown below
          so it can be registered for meeting receipts. The key itself never
          leaves this machine, and recording never depends on it.
        </p>
      </div>

      {loading && (
        <p className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
          Reading identity…
        </p>
      )}

      {error && (
        <div
          className="p-4 rounded-xl text-sm"
          style={{ backgroundColor: "var(--color-bg-subtle)" }}
        >
          <strong>Could not read this installation&rsquo;s identity.</strong>
          <div
            className="mt-1"
            style={{ color: "var(--color-text-secondary)" }}
          >
            {error}
          </div>
          <div
            className="mt-2"
            style={{ color: "var(--color-text-secondary)" }}
          >
            Recording and transcription are unaffected.
          </div>
        </div>
      )}

      {identity && (
        <>
          <div
            className="p-4 rounded-xl space-y-3"
            style={{ backgroundColor: "var(--color-bg-subtle)" }}
          >
            <CopyField label="DID" value={identity.did} />
            <CopyField
              label="Public key (base64, for the App Registry)"
              value={publicKeyBase64(identity.publicKeyHex)}
            />
            <CopyField label="Service ID" value={identity.serviceId} />
          </div>

          <div
            className="p-4 rounded-xl text-sm"
            style={{ backgroundColor: "var(--color-bg-subtle)" }}
          >
            <strong>
              {identity.enrolled ? "Enrolled" : "Not enrolled"}
            </strong>
            <div
              className="mt-1"
              style={{ color: "var(--color-text-secondary)" }}
            >
              {identity.enrolled
                ? "Meetings recorded on this device can be attested."
                : "No credential names this identity yet, so meetings are not attested. Transcripts are still versioned and tamper-evident on this device."}
            </div>

            {identity.credential && (
              <div className="mt-3 space-y-1 text-xs">
                {identity.credential.standing !== "active" && (
                  <div style={{ color: "#eab308" }}>
                    {identity.credential.standing === "expired"
                      ? "This credential has expired. Receipts minted while it was valid are unaffected."
                      : "This credential's expiry could not be read, so it is not being relied on."}
                  </div>
                )}
                <div style={{ color: "var(--color-text-secondary)" }}>
                  Issued {identity.credential.issuedAt.slice(0, 10)}, expires{" "}
                  {identity.credential.expiresAt.slice(0, 10)}
                </div>
                <div style={{ color: "var(--color-text-secondary)" }}>
                  Scope: {identity.credential.authorityScope.join(", ") || "none"}
                </div>
                <button
                  type="button"
                  className="mt-1 underline"
                  style={{ color: "var(--color-text-secondary)" }}
                  onClick={async () => setInstallError(await removeCredential())}
                >
                  Remove credential
                </button>
              </div>
            )}
          </div>

          {!identity.credential && (
            <div
              className="p-4 rounded-xl space-y-2"
              style={{ backgroundColor: "var(--color-bg-subtle)" }}
            >
              <h4
                className="text-sm font-semibold"
                style={{ color: "var(--color-text)" }}
              >
                Install a credential
              </h4>
              <p
                className="text-sm"
                style={{ color: "var(--color-text-secondary)" }}
              >
                Register this DID and public key with the AVC App Registry, then
                paste the credential it issues. It must name this installation —
                one issued for another machine is refused, because this device
                could not sign for it.
              </p>
              <textarea
                value={paste}
                onChange={(e) => setPaste(e.target.value)}
                rows={5}
                spellCheck={false}
                placeholder='{"id":"…","issuer_did":"…","subject_did":"…"}'
                className="w-full text-xs p-2 rounded-lg font-mono"
                style={{
                  backgroundColor: "var(--color-bg)",
                  color: "var(--color-text)",
                }}
              />
              <button
                type="button"
                disabled={!paste.trim()}
                className="text-sm px-3 py-1.5 rounded-lg disabled:opacity-50"
                style={{
                  backgroundColor: "var(--color-accent, #3b82f6)",
                  color: "white",
                }}
                onClick={async () => {
                  const failure = await installCredential(paste);
                  setInstallError(failure);
                  if (!failure) setPaste("");
                }}
              >
                Install
              </button>
              {installError && (
                <p className="text-sm" style={{ color: "#ef4444" }}>
                  {installError}
                </p>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
