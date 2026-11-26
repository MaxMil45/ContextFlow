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

beforeEach(() => {
  localStorage.clear();
  installMockBackend();
});

afterEach(() => {
  cleanup();
  clearMocks();
});
