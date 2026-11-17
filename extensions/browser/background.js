// ContextFlow browser extension.
//
// Sends the list of open windows and tabs (address, title, order, pinned,
// active) to the local ContextFlow native host. It never reads cookies,
// passwords, history, page content or form data, and it skips private
// (incognito) windows. Data goes only to the program on this computer.
//
// The one thing it receives is a request to pin tabs: after ContextFlow
// reopens a window whose tabs were pinned (the browser's command line can't
// pin), it asks for those addresses to be pinned again.

const HOST = "dev.contextflow.host";
const BROWSER = /Edg\//.test(navigator.userAgent) ? "msedge" : "chrome";
const DEBOUNCE_MS = 800;
const HEARTBEAT_MS = 20000;
// A pin request is tried for this long: tabs may still be loading.
const PIN_FOR_MS = 60000;
const MAX_PINS = 100;

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

// ---------------------------------------------------------------- pinning

// Requests not finished yet: { id, urls (normalised, in order), until }.
let pinRequests = [];
const seenRequests = new Set();
let pinTimer = null;

// The same normalisation ContextFlow uses (`cf_browser::normalize_url`), so an
// address matches however the browser spells it: no fragment, lower-case
// scheme and host, no trailing slash on the path.
function normalizeUrl(url) {
  url = (url || "").trim().split("#")[0];
  const at = url.indexOf("://");
  if (at < 0) return url;
  const scheme = url.slice(0, at).toLowerCase();
  const rest = url.slice(at + 3);
  const cut = rest.search(/[/?]/);
  const host = (cut < 0 ? rest : rest.slice(0, cut)).toLowerCase();
  let path = cut < 0 ? "" : rest.slice(cut);
  if (path === "/") path = "";
  if (path.length > 1 && path.endsWith("/") && !path.includes("?")) path = path.slice(0, -1);
  return `${scheme}://${host}${path}`;
}

function acceptPinRequest(msg) {
  if (!msg || msg.type !== "pin" || msg.browser !== BROWSER) return;
  if (typeof msg.id !== "string" || seenRequests.has(msg.id) || !Array.isArray(msg.urls)) return;
  seenRequests.add(msg.id);
  const urls = msg.urls
    .filter((u) => typeof u === "string" && /^https?:\/\//i.test(u))
    .slice(0, MAX_PINS)
    .map(normalizeUrl);
  if (urls.length) pinRequests.push({ id: msg.id, urls, until: Date.now() + PIN_FOR_MS });
  schedulePins();
}

// Pin, for each request, one unpinned tab per address, in the saved order.
// Only addresses ContextFlow has just opened are ever asked for (it reopens
// only the tabs that were not open), so an unpinned tab with that address is
// the one it opened.
async function applyPins() {
  pinRequests = pinRequests.filter((r) => r.urls.length && r.until > Date.now());
  if (!pinRequests.length) return;
  let windows;
  try {
    windows = await chrome.windows.getAll({ populate: true, windowTypes: ["normal"] });
  } catch (e) {
    return;
  }
  const tabs = windows.filter((w) => !w.incognito).flatMap((w) => w.tabs || []);
  const taken = new Set();
  for (const request of pinRequests) {
    const left = [];
    for (const want of request.urls) {
      const tab = tabs.find(
        (t) => !t.pinned && !taken.has(t.id) && normalizeUrl(t.pendingUrl || t.url) === want,
      );
      if (!tab) {
        left.push(want);
        continue;
      }
      taken.add(tab.id);
      try {
        await chrome.tabs.update(tab.id, { pinned: true });
      } catch (e) {
        left.push(want); // closed or busy: try again on the next change
      }
    }
    request.urls = left;
  }
}

function schedulePins() {
  if (!pinRequests.length) return;
  clearTimeout(pinTimer);
  pinTimer = setTimeout(applyPins, 300);
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
  port.onMessage.addListener(acceptPinRequest);
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
// New tabs, and tabs that finish loading, may be the ones a request is for.
chrome.tabs.onCreated.addListener(schedulePins);
chrome.tabs.onUpdated.addListener((_id, change) => {
  if (change.url || change.status === "complete") schedulePins();
});
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
