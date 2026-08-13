import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface TrackLevel {
  rmsDbfs: number;
  peakDbfs: number;
  verdict: "silent" | "quiet" | "healthy" | "clipping";
}

export interface DeviceTestLevels {
  microphone: TrackLevel;
  system: TrackLevel;
  /** The devices each track actually opened, which a fallback can make
   *  different from what the picker shows. */
  microphoneDevice: string | null;
  systemDevice: string | null;
  /** False when the system capture never started, as opposed to hearing nothing. */
  systemAvailable: boolean;
}

/** How often to read the meters. Fast enough to feel live, slow enough to be free. */
const POLL_MS = 100;

/**
 * Convert a dBFS reading to a 0–1 bar position.
 *
 * The visible range stops at -60: below that is room tone on any real input,
 * and giving it half the bar would waste the half where the useful distinctions
 * live.
 */
export function meterFraction(dbfs: number): number {
  const floor = -60;
  if (!Number.isFinite(dbfs) || dbfs <= floor) return 0;
  if (dbfs >= 0) return 1;
  return (dbfs - floor) / -floor;
}

/**
 * @param boundTo Changing this restarts the test.
 *
 * A capture binds its device when the stream opens, so changing a picker
 * mid-test does nothing — the meters carry on reading the old devices, which
 * looks exactly like a picker that has no effect.
 */
export function useDeviceTest(boundTo?: string) {
  const [running, setRunning] = useState(false);
  const [levels, setLevels] = useState<DeviceTestLevels | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<number | null>(null);

  const stop = useCallback(async () => {
    if (timer.current !== null) {
      clearInterval(timer.current);
      timer.current = null;
    }
    setRunning(false);
    setLevels(null);
    try {
      await invoke("stop_device_test");
    } catch {
      // Stopping is best-effort: the test may already have stopped, and
      // reporting a failure to stop something that is not running would be
      // noise the user cannot act on.
    }
  }, []);

  const start = useCallback(async () => {
    setError(null);
    try {
      await invoke<boolean>("start_device_test");
    } catch (e) {
      setError(String(e));
      return;
    }
    setRunning(true);
    timer.current = window.setInterval(() => {
      invoke<DeviceTestLevels>("get_device_test_levels")
        .then(setLevels)
        .catch(() => {
          // A single failed poll is not worth tearing the test down for.
        });
    }, POLL_MS);
  }, []);

  // Leaving a capture running after the panel closes would hold the microphone
  // open — and on Windows that can stop the next recording from opening it.
  useEffect(() => {
    return () => {
      if (timer.current !== null) clearInterval(timer.current);
      invoke("stop_device_test").catch(() => {});
    };
  }, []);

  // Rebind when the selection changes. Without this the test keeps reading the
  // devices it opened with, so switching to something that could not possibly
  // work leaves the meters moving — and the picker looks broken when it is the
  // test that is stale.
  const rebinding = useRef(false);
  useEffect(() => {
    if (!running || rebinding.current) return;
    rebinding.current = true;
    stop()
      .then(start)
      .finally(() => {
        rebinding.current = false;
      });
    // `running` is deliberately absent: including it would restart the test
    // every time its own restart flipped the flag.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [boundTo]);

  return { running, levels, error, start, stop };
}
