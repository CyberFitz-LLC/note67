import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";

import { useOutputDevices } from "./useOutputDevices";
import { audioApi } from "../api";

vi.mock("../api", () => ({
  audioApi: {
    isOutputDeviceSelectable: vi.fn(),
    listOutputDevices: vi.fn(),
    getPreferredOutputDevice: vi.fn(),
    setPreferredOutputDevice: vi.fn(),
  },
}));

const api = vi.mocked(audioApi, true);

const SPEAKERS = { name: "Speakers (Realtek Audio)", isDefault: true };
const HEADSET = { name: "Headset Earphone (Jabra)", isDefault: false };

beforeEach(() => {
  vi.clearAllMocks();
  api.isOutputDeviceSelectable.mockResolvedValue(true);
  api.listOutputDevices.mockResolvedValue([SPEAKERS, HEADSET]);
  api.getPreferredOutputDevice.mockResolvedValue(null);
  api.setPreferredOutputDevice.mockResolvedValue(undefined);
});

async function renderLoaded() {
  const hook = renderHook(() => useOutputDevices());
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  return hook;
}

describe("useOutputDevices", () => {
  it("lists the playback devices and reports the platform supports choosing", async () => {
    const { result } = await renderLoaded();

    await waitFor(() => expect(result.current.selectable).toBe(true));
    expect(result.current.devices).toEqual([SPEAKERS, HEADSET]);
  });

  it("reports not selectable where the platform has no such choice", async () => {
    // macOS captures the whole system mix; there is no output device to pick.
    api.isOutputDeviceSelectable.mockResolvedValue(false);
    api.listOutputDevices.mockResolvedValue([]);

    const { result } = await renderLoaded();

    await waitFor(() => expect(result.current.selectable).toBe(false));
    expect(result.current.devices).toEqual([]);
  });

  it("treats an unanswerable capability check as not selectable", async () => {
    // Hiding the control beats offering one that cannot work.
    api.isOutputDeviceSelectable.mockRejectedValue(new Error("no such command"));

    const { result } = await renderLoaded();

    expect(result.current.selectable).toBe(false);
  });

  it("saves a playback device selection", async () => {
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.selectDevice("Headset Earphone (Jabra)");
    });

    expect(api.setPreferredOutputDevice).toHaveBeenCalledWith(
      "Headset Earphone (Jabra)"
    );
    expect(result.current.selectedDevice).toBe("Headset Earphone (Jabra)");
  });

  it("clears the selection back to the system default", async () => {
    api.getPreferredOutputDevice.mockResolvedValue("Headset Earphone (Jabra)");
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.selectDevice(null);
    });

    expect(api.setPreferredOutputDevice).toHaveBeenCalledWith(null);
    expect(result.current.selectedDevice).toBeNull();
  });

  it("flags a pinned device that is no longer connected", async () => {
    // Undocking takes the headset away; capture silently falls back to the
    // laptop speakers, so the UI has to say so.
    api.getPreferredOutputDevice.mockResolvedValue("Headset Earphone (Jabra)");
    api.listOutputDevices.mockResolvedValue([SPEAKERS]);

    const { result } = await renderLoaded();

    expect(result.current.missingDevice).toBe("Headset Earphone (Jabra)");
  });

  it("keeps the previous selection when saving fails", async () => {
    const { result } = await renderLoaded();
    api.setPreferredOutputDevice.mockRejectedValue(
      new Error("System audio capture is not available on this platform")
    );

    await act(async () => {
      await result.current.selectDevice("Headset Earphone (Jabra)");
    });

    expect(result.current.selectedDevice).toBeNull();
    expect(result.current.error).toContain("not available");
  });

  it("picks up devices connected since the last load", async () => {
    api.listOutputDevices.mockResolvedValue([SPEAKERS]);
    const { result } = await renderLoaded();
    expect(result.current.devices).toEqual([SPEAKERS]);

    api.listOutputDevices.mockResolvedValue([SPEAKERS, HEADSET]);
    await act(async () => {
      await result.current.refresh();
    });

    expect(result.current.devices).toEqual([SPEAKERS, HEADSET]);
  });
});
