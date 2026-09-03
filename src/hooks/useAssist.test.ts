import { describe, expect, it } from "vitest";
import { freshnessLabel, stalenessSeconds } from "./useAssist";

describe("staleness", () => {
  it("is unknown before the meeting has produced anything", () => {
    expect(stalenessSeconds(null, 120)).toBeNull();
    expect(stalenessSeconds(30, null)).toBeNull();
  });

  it("is how far the model is behind the room", () => {
    expect(stalenessSeconds(100, 220)).toBe(120);
  });

  it("never reads as ahead of the meeting", () => {
    // Clock skew between the transcript's own timeline and the elapsed
    // recording would otherwise show a negative lag, which reads as nonsense.
    expect(stalenessSeconds(130, 120)).toBe(0);
  });
});

describe("freshnessLabel", () => {
  it("says so plainly when there is nothing yet", () => {
    expect(freshnessLabel(null)).toBe("waiting for the meeting");
  });

  it("treats a short lag as current", () => {
    // The brief runs every 90 seconds, so a lag under a minute is the normal
    // state and calling it "behind" would be alarming about nothing.
    expect(freshnessLabel(10)).toBe("up to date");
    expect(freshnessLabel(44)).toBe("up to date");
  });

  it("names a real lag rather than hiding it", () => {
    // The thing a spinner conceals, and the thing that decides whether a
    // suggestion is worth acting on mid-call.
    expect(freshnessLabel(90)).toContain("minute behind");
    expect(freshnessLabel(300)).toBe("5 minutes behind");
  });
});
