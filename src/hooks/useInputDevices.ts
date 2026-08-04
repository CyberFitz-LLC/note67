import { useCallback, useEffect, useState } from "react";

import { audioApi } from "../api";
import type { AudioInputDevice } from "../types";

interface InputDeviceData {
  devices: AudioInputDevice[];
  selectedDevice: string | null;
}

// Read the device list and the saved preference. Module-level (no React state)
// so both the mount effect and the manual `refresh` can share it without
// duplicating logic.
async function fetchInputDevices(): Promise<InputDeviceData> {
  const [devices, selectedDevice] = await Promise.all([
    audioApi.listInputDevices(),
    audioApi.getPreferredInputDevice(),
  ]);
  return { devices, selectedDevice };
}

/**
 * The microphones available for recording, plus which one the user pinned.
 *
 * `selectedDevice` is `null` when no device is pinned, which means recording
 * follows whatever the operating system's default input is at the time.
 */
export function useInputDevices() {
  const [data, setData] = useState<InputDeviceData>({
    devices: [],
    selectedDevice: null,
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Initial load. Inlined so setState only runs in the async continuation.
  // There is no hotplug notification from the backend, so this is a snapshot;
  // Refresh in the UI re-takes it.
  useEffect(() => {
    let cancelled = false;
    fetchInputDevices()
      .then((next) => {
        if (cancelled) return;
        setData(next);
        setError(null);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      setData(await fetchInputDevices());
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const selectDevice = useCallback(async (deviceName: string | null) => {
    setSaving(true);
    try {
      await audioApi.setPreferredInputDevice(deviceName);
      // Only reflect the choice once it is persisted — otherwise the settings
      // pane would show a microphone the next recording will not use.
      setData((prev) => ({ ...prev, selectedDevice: deviceName }));
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }, []);

  const { devices, selectedDevice } = data;

  /**
   * The pinned device name when it is not currently connected, otherwise null.
   * Recording falls back to the system default in this state, silently, so the
   * UI needs to say something.
   */
  const missingDevice =
    selectedDevice !== null && !devices.some((d) => d.name === selectedDevice)
      ? selectedDevice
      : null;

  return {
    devices,
    selectedDevice,
    missingDevice,
    loading,
    saving,
    error,
    selectDevice,
    refresh,
  };
}
