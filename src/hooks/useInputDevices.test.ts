import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";

import { useInputDevices } from "./useInputDevices";
import { audioApi } from "../api";

vi.mock("../api", () => ({
  audioApi: {
    listInputDevices: vi.fn(),
    getPreferredInputDevice: vi.fn(),
    setPreferredInputDevice: vi.fn(),
  },
}));

const api = vi.mocked(audioApi, true);

const BUILT_IN = { id: "", name: "MacBook Pro Microphone", isDefault: true };
const YETI = { id: "", name: "Blue Yeti", isDefault: false };

beforeEach(() => {
  vi.clearAllMocks();
  api.listInputDevices.mockResolvedValue([BUILT_IN, YETI]);
  api.getPreferredInputDevice.mockResolvedValue(null);
  api.setPreferredInputDevice.mockResolvedValue(undefined);
});

/** Render and wait for the initial load to settle. */
async function renderLoaded() {
  const hook = renderHook(() => useInputDevices());
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  return hook;
}

describe("useInputDevices", () => {
  it("loads the device list and the saved preference on mount", async () => {
    api.getPreferredInputDevice.mockResolvedValue("Blue Yeti");

    const { result } = await renderLoaded();

    expect(result.current.devices).toEqual([BUILT_IN, YETI]);
    expect(result.current.selectedDevice).toBe("Blue Yeti");
  });

  it("reports no selection when following the system default", async () => {
    const { result } = await renderLoaded();

    expect(result.current.selectedDevice).toBeNull();
    expect(result.current.missingDevice).toBeNull();
  });

  it("saves a device selection", async () => {
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.selectDevice("Blue Yeti");
    });

    expect(api.setPreferredInputDevice).toHaveBeenCalledWith("Blue Yeti");
    expect(result.current.selectedDevice).toBe("Blue Yeti");
  });

  it("clears the selection back to the system default", async () => {
    api.getPreferredInputDevice.mockResolvedValue("Blue Yeti");
    const { result } = await renderLoaded();

    await act(async () => {
      await result.current.selectDevice(null);
    });

    expect(api.setPreferredInputDevice).toHaveBeenCalledWith(null);
    expect(result.current.selectedDevice).toBeNull();
  });

  it("flags a pinned device that is no longer connected", async () => {
    // The user pinned a USB mic and then unplugged it. Recording silently falls
    // back to the default, so the UI has to be able to say so.
    api.getPreferredInputDevice.mockResolvedValue("Blue Yeti");
    api.listInputDevices.mockResolvedValue([BUILT_IN]);

    const { result } = await renderLoaded();

    expect(result.current.selectedDevice).toBe("Blue Yeti");
    expect(result.current.missingDevice).toBe("Blue Yeti");
  });

  it("clears the missing-device flag once the device is plugged back in", async () => {
    api.getPreferredInputDevice.mockResolvedValue("Blue Yeti");
    api.listInputDevices.mockResolvedValue([BUILT_IN]);

    const { result } = await renderLoaded();
    expect(result.current.missingDevice).toBe("Blue Yeti");

    api.listInputDevices.mockResolvedValue([BUILT_IN, YETI]);
    await act(async () => {
      await result.current.refresh();
    });

    expect(result.current.missingDevice).toBeNull();
  });

  it("keeps the previous selection when saving fails", async () => {
    // Losing the write should not leave the UI claiming a device that the
    // backend is not actually going to record from.
    const { result } = await renderLoaded();
    api.setPreferredInputDevice.mockRejectedValue(new Error("db is locked"));

    await act(async () => {
      await result.current.selectDevice("Blue Yeti");
    });

    expect(result.current.selectedDevice).toBeNull();
    expect(result.current.error).toContain("db is locked");
  });

  it("surfaces a failure to enumerate devices", async () => {
    api.listInputDevices.mockRejectedValue(new Error("no audio host"));

    const { result } = await renderLoaded();

    expect(result.current.devices).toEqual([]);
    expect(result.current.error).toContain("no audio host");
  });

  it("picks up devices connected since the last load", async () => {
    api.listInputDevices.mockResolvedValue([BUILT_IN]);
    const { result } = await renderLoaded();
    expect(result.current.devices).toEqual([BUILT_IN]);

    api.listInputDevices.mockResolvedValue([BUILT_IN, YETI]);
    await act(async () => {
      await result.current.refresh();
    });

    expect(result.current.devices).toEqual([BUILT_IN, YETI]);
  });
});
