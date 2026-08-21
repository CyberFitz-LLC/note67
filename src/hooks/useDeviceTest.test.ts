import { describe, expect, it } from "vitest";
import { meterFraction } from "./useDeviceTest";

describe("meterFraction", () => {
  it("puts full scale at the top of the bar", () => {
    expect(meterFraction(0)).toBe(1);
  });

  it("puts room tone at the bottom rather than a quarter of the way up", () => {
    // Below -60 dBFS is room tone on any real input. Giving it bar space would
    // waste the half where the useful distinctions are.
    expect(meterFraction(-60)).toBe(0);
    expect(meterFraction(-80)).toBe(0);
  });

  it("places speech in the visible middle", () => {
    const speech = meterFraction(-26);
    expect(speech).toBeGreaterThan(0.4);
    expect(speech).toBeLessThan(0.7);
  });

  it("never leaves the bar", () => {
    for (const db of [10, 0, -30, -100, -1000]) {
      const f = meterFraction(db);
      expect(f).toBeGreaterThanOrEqual(0);
      expect(f).toBeLessThanOrEqual(1);
    }
  });

  it("survives a non-finite reading", () => {
    // dbfs(0) is clamped in Rust, but a bar that rendered NaN width would
    // break the layout rather than showing silence.
    expect(meterFraction(Number.NEGATIVE_INFINITY)).toBe(0);
    expect(meterFraction(Number.NaN)).toBe(0);
  });
});
