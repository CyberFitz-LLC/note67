import { describe, expect, it } from "vitest";
import { publicKeyBase64 } from "./useExochainIdentity";

describe("publicKeyBase64", () => {
  it("converts the stored hex to what the registry expects", () => {
    // The Ed25519 public key for the all-zero seed — the same vector the
    // canonical crate pins its DID against.
    expect(
      publicKeyBase64(
        "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29",
      ),
    ).toBe("O2onvM62pC1io6jQKm8Nc2UyFXcd4kOmOsBIoYtZ2ik=");
  });

  it("tolerates surrounding whitespace", () => {
    expect(publicKeyBase64("  3b6a  ")).toBe(publicKeyBase64("3b6a"));
  });

  it("returns nothing for input that is not hex", () => {
    // Better an obviously empty field than a plausible-looking string that the
    // registry rejects as a DID mismatch — which reads as "your identity is
    // wrong" rather than "your encoding is wrong".
    expect(publicKeyBase64("zzzz")).toBe("");
    expect(publicKeyBase64("abc")).toBe("");
  });

  it("round-trips through the browser decoder", () => {
    const hex = "00ff10";
    const decoded = atob(publicKeyBase64(hex));
    const back = Array.from(decoded)
      .map((c) => c.charCodeAt(0).toString(16).padStart(2, "0"))
      .join("");
    expect(back).toBe(hex);
  });
});
