import { useCallback, useEffect, useState } from "react";

import type { AudioDevice } from "../types";

/**
 * The three calls a device picker needs. Microphones and playback devices
 * differ only in which backend commands they hit, so they share this shape.
 */
export interface DeviceSelectionApi {
  list: () => Promise<AudioDevice[]>;
  getPreferred: () => Promise<string | null>;
  setPreferred: (deviceName: string | null) => Promise<void>;
}

interface DeviceData {
  devices: AudioDevice[];
  selectedDevice: string | null;
}

/**
 * Devices available in one direction, plus which one the user pinned.
 *
 * `selectedDevice` is `null` when nothing is pinned, meaning the app follows
 * whatever the operating system's default is at the time.
 *
 * Pass a module-level `api` object: it is a dependency of the load effect, so a
 * fresh object each render would re-fetch on every render.
 */
export function useDeviceSelection(api: DeviceSelectionApi) {
  const [data, setData] = useState<DeviceData>({
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
    Promise.all([api.list(), api.getPreferred()])
      .then(([devices, selectedDevice]) => {
        if (cancelled) return;
        setData({ devices, selectedDevice });
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
  }, [api]);

  const refresh = useCallback(async () => {
    try {
      const [devices, selectedDevice] = await Promise.all([
        api.list(),
        api.getPreferred(),
      ]);
      setData({ devices, selectedDevice });
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [api]);

  const selectDevice = useCallback(
    async (deviceName: string | null) => {
      setSaving(true);
      try {
        await api.setPreferred(deviceName);
        // Only reflect the choice once it is persisted — otherwise the settings
        // pane would show a device the next recording will not use.
        setData((prev) => ({ ...prev, selectedDevice: deviceName }));
        setError(null);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setSaving(false);
      }
    },
    [api]
  );

  const { devices, selectedDevice } = data;

  /**
   * The pinned device when it is not currently connected, otherwise null.
   * Recording falls back to the system default in this state, silently, so the
   * UI needs to say something.
   *
   * Matched on id *or* name, because a preference can be either: playback
   * endpoints are pinned by id, microphones by name, and preferences saved
   * before ids existed are names too. Comparing only names reported every
   * id-pinned device as unplugged — while it was selected and working.
   */
  // The id only counts when there is one. Microphones all carry an empty id,
  // so comparing it to an empty preference would match the first mic in the
  // list and pin it at random.
  const matches = (d: { id?: string; name: string }) =>
    (!!d.id && d.id === selectedDevice) || d.name === selectedDevice;

  const missingDevice =
    selectedDevice !== null && !devices.some(matches) ? selectedDevice : null;

  /** What to show for a device the picker knows only by id. */
  const selectedLabel =
    devices.find(matches)?.name ?? selectedDevice ?? null;

  return {
    devices,
    selectedDevice,
    selectedLabel,
    missingDevice,
    loading,
    saving,
    error,
    selectDevice,
    refresh,
  };
}
