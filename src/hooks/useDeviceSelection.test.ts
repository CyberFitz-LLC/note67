import { describe, expect, it } from "vitest";

/**
 * The rule the picker uses to decide whether a pinned device is present.
 *
 * A preference can be an id or a name: playback endpoints are pinned by id,
 * microphones by name, and anything saved before ids existed is a name.
 * Comparing only names reported every id-pinned device as unplugged — while it
 * was in fact selected and working.
 */
const matches =
  (selected: string | null) => (d: { id?: string; name: string }) =>
    (!!d.id && d.id === selected) || d.name === selected;

const VAIO_6 = {
  id: "{0.0.0.00000000}.{ef282b32-e3d4-480e-a4e1-484e9c5f0c1c}",
  name: "Speakers (VB-Audio Voicemeeter VAIO) (6)",
};
const MIC = { id: "", name: "Blue Yeti" };

describe("is the pinned device present", () => {
  it("recognises a playback endpoint pinned by id", () => {
    // The regression: this reported "not connected" for a device sitting in
    // the list, because the check only ever compared names.
    expect([VAIO_6].some(matches(VAIO_6.id))).toBe(true);
  });

  it("recognises a microphone pinned by name", () => {
    expect([MIC].some(matches("Blue Yeti"))).toBe(true);
  });

  it("still recognises a preference saved before ids existed", () => {
    expect([VAIO_6].some(matches(VAIO_6.name))).toBe(true);
  });

  it("reports a genuinely absent device as absent", () => {
    expect([VAIO_6, MIC].some(matches("{0.0.0.0}.{departed}"))).toBe(false);
  });

  it("does not match a microphone's empty id against nothing", () => {
    // Every mic carries an empty id, so a null or empty preference must not
    // match the first one and pin it at random.
    expect([MIC].some(matches(""))).toBe(false);
  });
});
