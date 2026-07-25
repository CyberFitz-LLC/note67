import { vi } from "vitest";

/**
 * A fake Tauri event bus for unit tests.
 *
 * Hooks that stream (`useLiveTranscription`, `useSummaries`) subscribe via
 * `listen()` from `@tauri-apps/api/event`, which does real IPC. This stands in
 * for it: `listen` registers a handler and returns an unlisten fn, and tests
 * drive the hook by calling `emit()`.
 *
 * Usage — the mock factory is hoisted, so build the bus inside it:
 *
 *   vi.mock("@tauri-apps/api/event", async () => {
 *     const { createEventBus } = await import("../test/eventBus");
 *     return createEventBus();
 *   });
 *
 *   const { emit, listenerCount, reset } = await import("@tauri-apps/api/event")
 *     .then((m) => (m as unknown as EventBusModule).__bus);
 */
export interface EventBus {
  /** Deliver a payload to every handler registered for `event`. */
  emit: (event: string, payload: unknown) => void;
  /** How many handlers are currently registered for `event`. */
  listenerCount: (event: string) => number;
  /** Drop all handlers (call between tests). */
  reset: () => void;
}

export interface EventBusModule {
  listen: ReturnType<typeof vi.fn>;
  __bus: EventBus;
}

export function createEventBus(): EventBusModule {
  const handlers = new Map<string, Set<(e: { payload: unknown }) => void>>();

  const listen = vi.fn(
    async (event: string, handler: (e: { payload: unknown }) => void) => {
      const set = handlers.get(event) ?? new Set();
      set.add(handler);
      handlers.set(event, set);
      // The unlisten fn Tauri hands back.
      return () => {
        handlers.get(event)?.delete(handler);
      };
    }
  );

  const bus: EventBus = {
    emit(event, payload) {
      for (const handler of handlers.get(event) ?? []) {
        handler({ payload });
      }
    },
    listenerCount(event) {
      return handlers.get(event)?.size ?? 0;
    },
    reset() {
      handlers.clear();
    },
  };

  return { listen, __bus: bus };
}
