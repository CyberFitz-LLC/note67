import { useEffect, useState } from "react";

import { audioApi } from "../api";
import { useDeviceSelection, type DeviceSelectionApi } from "./useDeviceSelection";

// Module-level so its identity is stable across renders; the hook treats it as
// an effect dependency.
const OUTPUT_API: DeviceSelectionApi = {
  list: () => audioApi.listOutputDevices(),
  getPreferred: () => audioApi.getPreferredOutputDevice(),
  setPreferred: (name) => audioApi.setPreferredOutputDevice(name),
};

/**
 * The playback devices whose audio can be captured, plus which one is pinned.
 *
 * `selectable` is false on platforms where the choice does not exist. macOS
 * captures the whole system mix through ScreenCaptureKit — there is no output
 * device to choose between — so the UI hides the control rather than showing an
 * empty dropdown that looks broken.
 */
export function useOutputDevices() {
  const base = useDeviceSelection(OUTPUT_API);
  const [selectable, setSelectable] = useState(false);

  useEffect(() => {
    let cancelled = false;
    audioApi
      .isOutputDeviceSelectable()
      .then((value) => {
        if (!cancelled) setSelectable(value);
      })
      .catch(() => {
        // Treat an unanswerable capability question as "not available": hiding
        // the control is better than offering one that cannot work.
        if (!cancelled) setSelectable(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { ...base, selectable };
}
