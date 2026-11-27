// Runs before every test file: browser stand-ins the app uses and jsdom lacks,
// and the sample backend (src/dev/mockBackend.ts) in place of the Rust side.

import "@testing-library/jest-dom/vitest";

import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup } from "@testing-library/react";
import { afterEach, beforeEach } from "vitest";

import { installMockBackend } from "../dev/mockBackend";

// jsdom has no layout, so these do nothing; the app only asks politely.
window.HTMLElement.prototype.scrollIntoView = () => {};
window.matchMedia ??= ((query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addEventListener: () => {},
  removeEventListener: () => {},
  addListener: () => {},
  removeListener: () => {},
  dispatchEvent: () => false,
})) as typeof window.matchMedia;
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

// Newer Node versions have their own `localStorage`, which throws unless Node
// is given a file to keep it in, and it hides jsdom's. The app uses it for a
// dismissed tip, so give the tests one that lives in memory, on every version.
const memoryStorage = new Map<string, string>();
const storage = {
  getItem: (k: string) => memoryStorage.get(k) ?? null,
  setItem: (k: string, v: string) => void memoryStorage.set(k, String(v)),
  removeItem: (k: string) => void memoryStorage.delete(k),
  clear: () => memoryStorage.clear(),
  key: (i: number) => [...memoryStorage.keys()][i] ?? null,
  get length() {
    return memoryStorage.size;
  },
};
for (const target of [globalThis, window]) {
  Object.defineProperty(target, "localStorage", { value: storage, configurable: true });
}

beforeEach(() => {
  memoryStorage.clear();
  installMockBackend();
});

afterEach(() => {
  cleanup();
  clearMocks();
});
