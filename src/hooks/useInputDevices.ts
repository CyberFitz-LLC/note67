import { audioApi } from "../api";
import { useDeviceSelection, type DeviceSelectionApi } from "./useDeviceSelection";

// Module-level so its identity is stable across renders; the hook treats it as
// an effect dependency.
const INPUT_API: DeviceSelectionApi = {
  list: () => audioApi.listInputDevices(),
  getPreferred: () => audioApi.getPreferredInputDevice(),
  setPreferred: (name) => audioApi.setPreferredInputDevice(name),
};

/** The microphones available for recording, plus which one the user pinned. */
export function useInputDevices() {
  return useDeviceSelection(INPUT_API);
}
