import { describe, expect, it } from "vitest";
import { imageFromClipboard } from "./useScreenshots";

/** Build a ClipboardEvent-like object with the given items. */
function paste(items: Array<Partial<DataTransferItem> & { kind: string; type: string }>) {
  return {
    clipboardData: { items: items as unknown as DataTransferItemList },
  } as unknown as ClipboardEvent;
}

const imageItem = (bytes: number[], type = "image/png") => ({
  kind: "file",
  type,
  getAsFile: () =>
    ({
      arrayBuffer: async () => new Uint8Array(bytes).buffer,
    }) as unknown as File,
});

describe("imageFromClipboard", () => {
  it("returns the bytes of a pasted image", async () => {
    const out = await imageFromClipboard(paste([imageItem([0x89, 0x50, 0x4e, 0x47])]));
    expect(out).not.toBeNull();
    expect(Array.from(out!)).toEqual([0x89, 0x50, 0x4e, 0x47]);
  });

  it("ignores a plain text paste", async () => {
    // The rule that matters most here: pasting text into a note must keep
    // working exactly as it did. This hook is only ever allowed to act on
    // images.
    const out = await imageFromClipboard(
      paste([{ kind: "string", type: "text/plain" }]),
    );
    expect(out).toBeNull();
  });

  it("ignores a paste with no clipboard data at all", async () => {
    expect(await imageFromClipboard({} as ClipboardEvent)).toBeNull();
  });

  it("takes the image when text and an image are pasted together", async () => {
    // Screenshot tools commonly put both on the clipboard.
    const out = await imageFromClipboard(
      paste([
        { kind: "string", type: "text/plain" },
        imageItem([0xff, 0xd8, 0xff], "image/jpeg"),
      ]),
    );
    expect(Array.from(out!)).toEqual([0xff, 0xd8, 0xff]);
  });

  it("ignores a non-image file", async () => {
    const out = await imageFromClipboard(
      paste([{ kind: "file", type: "application/pdf", getAsFile: () => null as unknown as File }]),
    );
    expect(out).toBeNull();
  });
});
