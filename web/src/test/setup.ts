import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/preact";
import { afterEach, beforeEach, vi } from "vitest";

/**
 * A minimal in-memory `Storage`.
 *
 * jsdom does not expose `localStorage` in every test configuration. The panel
 * only uses it for the language preference; authentication lives in an
 * HttpOnly cookie and is deliberately inaccessible to this code.
 */
function memoryStorage(): Storage {
  let entries = new Map<string, string>();

  return {
    get length() {
      return entries.size;
    },
    clear: () => {
      entries = new Map();
    },
    getItem: (key: string) => entries.get(key) ?? null,
    key: (index: number) => [...entries.keys()][index] ?? null,
    removeItem: (key: string) => {
      entries.delete(key);
    },
    setItem: (key: string, value: string) => {
      entries.set(key, String(value));
    },
  };
}

if (typeof globalThis.localStorage === "undefined") {
  Object.defineProperty(globalThis, "localStorage", {
    value: memoryStorage(),
    configurable: true,
  });
}

// Each test starts from a clean DOM and an empty preference store, so one test
// cannot leave a rendered dialog or language choice behind for the next.
beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  cleanup();
  localStorage.clear();
  vi.restoreAllMocks();
});
