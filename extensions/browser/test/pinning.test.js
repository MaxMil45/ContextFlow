// Loads the extension's background.js with a fake `chrome` and checks that pin
// requests from ContextFlow pin the right tabs. No browser needed.
// Run: node extensions/browser/test/pinning.test.js
const fs = require("fs");
const path = require("path");
const vm = require("vm");
const assert = require("assert");

const src = fs.readFileSync(path.join(__dirname, "..", "background.js"), "utf8");
const listeners = {};
const ev = (name) => ({ addListener: (f) => (listeners[name] = (listeners[name] || []).concat(f)) });
let tabs = [];
const updates = [];
let portListener;
const chrome = {
  storage: { local: { get: async () => ({ profileId: "p" }), set: async () => {} } },
  windows: {
    getAll: async () => [{ incognito: false, tabs }],
    onCreated: ev("wc"),
    onRemoved: ev("wr"),
  },
  tabs: {
    update: async (id, props) => {
      updates.push([id, props.pinned]);
      tabs.find((t) => t.id === id).pinned = true;
    },
    onCreated: ev("tc"), onRemoved: ev("tr"), onMoved: ev("tm"), onAttached: ev("ta"),
    onDetached: ev("td"), onActivated: ev("tac"), onReplaced: ev("trp"), onUpdated: ev("tu"),
  },
  runtime: {
    connectNative: () => ({
      postMessage: () => {},
      onMessage: { addListener: (f) => (portListener = f) },
      onDisconnect: { addListener: () => {} },
    }),
    onStartup: ev("s"), onInstalled: ev("i"), lastError: undefined,
  },
  alarms: { create: () => {}, onAlarm: ev("a") },
};
const ctx = { chrome, navigator: { userAgent: "Chrome/130" }, crypto: { randomUUID: () => "x" },
  setTimeout, clearTimeout, setInterval: () => 0, console, Date };
vm.createContext(ctx);
vm.runInContext(src, ctx);

(async () => {
  // Normalisation matches cf_browser::normalize_url.
  const n = (u) => vm.runInContext(`normalizeUrl(${JSON.stringify(u)})`, ctx);
  assert.strictEqual(n("HTTPS://Mail.Example.com/"), "https://mail.example.com");
  assert.strictEqual(n("https://a.com/x/#frag"), "https://a.com/x");
  assert.strictEqual(n("https://a.com/x/?q=1"), "https://a.com/x/?q=1");
  assert.strictEqual(n("https://a.com?q"), "https://a.com?q");

  // An old unpinned tab elsewhere and the two new ones; a third is still loading.
  tabs = [
    { id: 1, url: "https://chat.example.com/", pinned: true },
    { id: 2, url: "https://mail.example.com/#inbox", pinned: false },
    { id: 3, url: "", pendingUrl: "https://chat.example.com/room", pinned: false },
  ];
  // Wrong browser and bad messages are ignored.
  portListener({ type: "pin", id: "e", browser: "msedge", urls: ["https://mail.example.com/"] });
  portListener({ type: "pin", id: "bad", browser: "chrome", urls: "nope" });
  portListener({ type: "pin", id: "r1", browser: "chrome",
    urls: ["https://mail.example.com/", "https://chat.example.com/room", "https://docs.example.com/", "javascript:x"] });
  await new Promise((r) => setTimeout(r, 400));
  await new Promise((r) => setTimeout(r, 50));
  assert.deepStrictEqual(updates, [[2, true], [3, true]], JSON.stringify(updates));

  // The third tab appears later: pinned when it is created.
  tabs.push({ id: 4, url: "https://docs.example.com/", pinned: false });
  listeners.tc.forEach((f) => f());
  await new Promise((r) => setTimeout(r, 400));
  await new Promise((r) => setTimeout(r, 50));
  assert.deepStrictEqual(updates.at(-1), [4, true]);

  // The same request again does nothing; nothing left to pin.
  const before = updates.length;
  tabs.push({ id: 5, url: "https://mail.example.com/", pinned: false });
  portListener({ type: "pin", id: "r1", browser: "chrome", urls: ["https://mail.example.com/"] });
  listeners.tc.forEach((f) => f());
  await new Promise((r) => setTimeout(r, 450));
  assert.strictEqual(updates.length, before, "a request is applied once");
  console.log("extension pinning: all checks passed");
})().catch((e) => { console.error(e); process.exit(1); });
