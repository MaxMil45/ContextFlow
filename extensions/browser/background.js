// ContextFlow browser extension.
//
// Sends the list of open windows and tabs (address, title, order, pinned,
// active) to the local ContextFlow native host. It never reads cookies,
// passwords, history, page content or form data, and it skips private
// (incognito) windows. Data goes only to the program on this computer.

const HOST = "dev.contextflow.host";
const BROWSER = /Edg\//.test(navigator.userAgent) ? "msedge" : "chrome";
const DEBOUNCE_MS = 800;
const HEARTBEAT_MS = 20000;

let port = null;
let debounce = null;

async function profileId() {
  const stored = await chrome.storage.local.get("profileId");
  if (stored.profileId) return stored.profileId;
  const id = crypto.randomUUID();
  await chrome.storage.local.set({ profileId: id });
  return id;
}

async function collect() {
  const windows = await chrome.windows.getAll({
    populate: true,
    windowTypes: ["normal"],
  });
  return windows
    .filter((w) => !w.incognito)
    .map((w) => ({
      focused: !!w.focused,
      state: w.state || "normal",
      // Position and size in CSS pixels; lets ContextFlow tell apart two
      // windows whose titles are identical.
      bounds: {
        left: w.left ?? 0,
        top: w.top ?? 0,
        width: w.width ?? 0,
        height: w.height ?? 0,
      },
      tabs: (w.tabs || []).map((t) => ({
        index: t.index,
        url: t.url || t.pendingUrl || "",
        title: t.title || "",
        pinned: !!t.pinned,
        active: !!t.active,
      })),
    }));
}

async function push() {
  if (!port) return;
  try {
    port.postMessage({
      type: "state",
      browser: BROWSER,
      profile_id: await profileId(),
      sent_at: Date.now(),
      windows: await collect(),
    });
  } catch (e) {
    // The port closed between the check and the send; onDisconnect handles it.
  }
}

function schedulePush() {
  clearTimeout(debounce);
  debounce = setTimeout(push, DEBOUNCE_MS);
}

function connect() {
  if (port) return;
  try {
    port = chrome.runtime.connectNative(HOST);
  } catch (e) {
    port = null;
    return;
  }
  port.onDisconnect.addListener(() => {
    void chrome.runtime.lastError; // host not installed or exited; retry later
    port = null;
  });
  push();
}

for (const ev of [
  chrome.tabs.onCreated,
  chrome.tabs.onRemoved,
  chrome.tabs.onMoved,
  chrome.tabs.onAttached,
  chrome.tabs.onDetached,
  chrome.tabs.onActivated,
  chrome.tabs.onReplaced,
  chrome.windows.onCreated,
  chrome.windows.onRemoved,
]) {
  ev.addListener(schedulePush);
}
chrome.tabs.onUpdated.addListener((_id, change) => {
  if (change.url || change.title || change.pinned !== undefined || change.status === "complete") {
    schedulePush();
  }
});

// While the connection is open the service worker stays alive; this keeps the
// host's copy fresh even when nothing changes, so it can tell "stale" from "idle".
setInterval(push, HEARTBEAT_MS);

// Reconnect if the host was missing or went away.
chrome.alarms.create("reconnect", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener(connect);
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);
connect();
