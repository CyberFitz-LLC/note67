import { describe, expect, it } from "vitest";
import { verdictOf } from "../utils/audioLevels";

/** A linear amplitude for a given dBFS, for readable cases. */
const at = (db: number) => Math.pow(10, db / 20);

describe("verdictOf", () => {
  it("calls an empty room silent rather than healthy", () => {
    // Real inputs are never exactly zero, so a threshold of zero would call an
    // empty room fine and defeat the point of the meter.
    expect(verdictOf(at(-70), at(-65))).toBe("silent");
  });

  it("calls a quiet but present track quiet", () => {
    expect(verdictOf(at(-45), at(-30))).toBe("quiet");
  });

  it("calls a normal speaking level healthy", () => {
    expect(verdictOf(at(-20), at(-10))).toBe("healthy");
  });

  it("calls clipping from the peak even when the average looks fine", () => {
    // The case this ordering exists for: a clipping track can sit at a
    // perfectly ordinary average, which is exactly why it goes unnoticed until
    // the transcript is bad.
    expect(verdictOf(at(-20), at(-0.2))).toBe("clipping");
  });

  it("treats true silence as silent rather than dividing by zero", () => {
    expect(verdictOf(0, 0)).toBe("silent");
  });

  it("agrees with the Rust thresholds at the boundaries", () => {
    // These mirror audio::levels — SILENCE −55, QUIET −35, CLIPPING −0.5.
    // Two implementations that disagree would show a different verdict in the
    // meter than the device test panel reports.
    expect(verdictOf(at(-56), at(-56))).toBe("silent");
    expect(verdictOf(at(-54), at(-54))).toBe("quiet");
    expect(verdictOf(at(-34), at(-34))).toBe("healthy");
    expect(verdictOf(at(-20), at(-0.4))).toBe("clipping");
  });
});
