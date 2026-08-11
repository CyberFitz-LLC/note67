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
  const { identity, error, loading } = useExochainIdentity();

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
          </div>
        </>
      )}
    </div>
  );
}
