import { describe, expect, it } from "vitest";
import {
  willStream,
  willUseRemote,
  DEFAULT_CONFIG,
  type TranscriptionConfig,
} from "./useTranscriptionBackend";

const config = (over: Partial<TranscriptionConfig>): TranscriptionConfig => ({
  ...DEFAULT_CONFIG,
  ...over,
});

describe("willUseRemote", () => {
  it("is false by default, so audio stays on the machine", () => {
    expect(willUseRemote(DEFAULT_CONFIG)).toBe(false);
  });

  it("is true for a configured remote", () => {
    expect(
      willUseRemote(
        config({ backend: "remote", baseUrl: "http://192.168.32.223:8010" }),
      ),
    ).toBe(true);
  });

  it("is false when remote is chosen but the URL is missing", () => {
    // The Rust side falls back to local here. Saying so in the UI is the
    // difference between "not finished" and a diarizer that appears broken.
    expect(willUseRemote(config({ backend: "remote", baseUrl: "  " }))).toBe(
      false,
    );
  });

  it("is false for a URL with no scheme", () => {
    // Mirrors transcription::backend::resolve, which refuses it rather than
    // guessing http and failing later with a confusing request error.
    expect(
      willUseRemote(config({ backend: "remote", baseUrl: "192.168.32.223:8010" })),
    ).toBe(false);
  });

  it("accepts https as well as http", () => {
    expect(
      willUseRemote(config({ backend: "remote", baseUrl: "https://asr.jtpa.net" })),
    ).toBe(true);
  });

  it("ignores a URL while the backend is local", () => {
    // Keeping the URL saved is deliberate — switching back should not make you
    // retype it — but it must not imply anything is being sent.
    expect(
      willUseRemote(config({ backend: "local", baseUrl: "http://x:8010" })),
    ).toBe(false);
  });
});

describe("willStream", () => {
  it("is false by default, so live audio stays on the machine", () => {
    // The default has to be the private one. This is the single most
    // consequential default in the app.
    expect(willStream(DEFAULT_CONFIG)).toBe(false);
  });

  it("is true for a configured streaming recogniser", () => {
    expect(
      willStream(
        config({ backend: "streaming", streamUrl: "ws://192.168.32.223:8080" }),
      ),
    ).toBe(true);
  });

  it("accepts wss for a recogniser reached over TLS", () => {
    expect(
      willStream(config({ backend: "streaming", streamUrl: "wss://asr.example" })),
    ).toBe(true);
  });

  it("is false when streaming is chosen but the URL is missing", () => {
    expect(willStream(config({ backend: "streaming", streamUrl: "   " }))).toBe(
      false,
    );
  });

  it("is false for an http URL, which is not a websocket", () => {
    // Easy to type, and the Rust side refuses it. Reading it as "streaming"
    // here would promise egress that never happens — and, worse, the warning
    // text would appear for a setting that does nothing.
    expect(
      willStream(
        config({ backend: "streaming", streamUrl: "http://192.168.32.223:8080" }),
      ),
    ).toBe(false);
  });

  it("does not stream just because a stream URL is filled in", () => {
    // A leftover URL from a previous experiment must not make the local
    // backend start shipping audio.
    expect(
      willStream(config({ backend: "local", streamUrl: "ws://192.168.32.223:8080" })),
    ).toBe(false);
    expect(
      willStream(config({ backend: "remote", streamUrl: "ws://192.168.32.223:8080" })),
    ).toBe(false);
  });

  it("and a remote upload URL never counts as streaming", () => {
    expect(
      willUseRemote(
        config({ backend: "streaming", streamUrl: "ws://192.168.32.223:8080" }),
      ),
    ).toBe(false);
  });
});
