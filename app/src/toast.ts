// The quick-capture confirmation card. Its own page with no framework: this
// window exists for the whole life of the app, so it stays tiny.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type ToastMessage = { kind: "saving" | "saved" | "error"; title: string; body: string };

const card = document.getElementById("toast")!;
const icon = document.getElementById("toast-icon")!;
const title = document.getElementById("toast-title")!;
const body = document.getElementById("toast-body")!;

function render(m: ToastMessage) {
  card.className = `toast-${m.kind}`;
  icon.textContent = m.kind === "saved" ? "✓" : m.kind === "error" ? "!" : "";
  title.textContent = m.title;
  body.textContent = m.body;
  body.hidden = !m.body;
  card.hidden = false;
}

// The window is created for a card and closed after it: the card it was opened
// for may already have been sent before this page loaded, so ask for it.
void invoke<ToastMessage | null>("current_toast").then((m) => m && render(m));
void listen<ToastMessage>("toast://show", ({ payload }) => render(payload));
