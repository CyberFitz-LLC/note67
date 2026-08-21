import { describe, expect, it } from "vitest";
import { formatOffset, formatStamp } from "./mergeFormatting";

describe("formatOffset", () => {
  it("shows which way the other recording was shifted", () => {
    // The sign is the point: it says whether the other tool started before or
    // after this one, which is how you sanity-check an alignment.
    expect(formatOffset(30000)).toBe("+30s");
    expect(formatOffset(-30000)).toBe("−30s");
  });

  it("uses minutes once there are any", () => {
    expect(formatOffset(90000)).toBe("+1m 30s");
    expect(formatOffset(-605000)).toBe("−10m 5s");
  });

  it("reports a zero offset as zero rather than as nothing", () => {
    // Two tools that happened to start together is a real result, and an empty
    // string would read as a failure to measure.
    expect(formatOffset(0)).toBe("+0s");
  });
});

describe("formatStamp", () => {
  it("pads seconds so timestamps line up in a list", () => {
    expect(formatStamp(65000)).toBe("1:05");
    expect(formatStamp(0)).toBe("0:00");
  });

  it("keeps counting minutes past an hour", () => {
    // A long meeting should not wrap back to 0:00.
    expect(formatStamp(3_600_000)).toBe("60:00");
  });
});
