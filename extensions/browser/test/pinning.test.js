// Loads the extension's background.js with a fake `chrome` and checks that pin
// and group requests from ContextFlow pin and group the right tabs, and that
// tab groups are reported. No browser needed.
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
const grouped = []; // [tabIds, groupId or null] for each chrome.tabs.group call (as plain data)
const groupInfo = {}; // groupId -> { title, color }
let nextGroup = 100;
let portListener;
const posted = [];
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
    group: async ({ tabIds, groupId }) => {
      grouped.push([[...tabIds], groupId ?? null]);
      const id = groupId ?? nextGroup++;
      for (const t of tabs) if (tabIds.includes(t.id)) t.groupId = id;
      return id;
    },
    onCreated: ev("tc"), onRemoved: ev("tr"), onMoved: ev("tm"), onAttached: ev("ta"),
    onDetached: ev("td"), onActivated: ev("tac"), onReplaced: ev("trp"), onUpdated: ev("tu"),
  },
  runtime: {
    connectNative: () => ({
      postMessage: (m) => posted.push(m),
      onMessage: { addListener: (f) => (portListener = f) },
      onDisconnect: { addListener: () => {} },
    }),
    onStartup: ev("s"), onInstalled: ev("i"), lastError: undefined,
  },
  tabGroups: {
    get: async (id) => {
      if (!groupInfo[id]) throw new Error("no such group");
      return groupInfo[id];
    },
    update: async (id, props) => {
      groupInfo[id] = { ...(groupInfo[id] || {}), ...props };
    },
    onCreated: ev("gc"),
    onUpdated: ev("gu"),
    onRemoved: ev("gr"),
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
  // ---------------------------------------------------------- groups

  // Capture: tabs report their group (title and colour), others null.
  groupInfo[7] = { title: "Work", color: "blue" };
  tabs.length = 0;
  tabs.push(
    { id: 21, windowId: 1, index: 0, url: "https://a.example/", title: "A", pinned: false, active: true, groupId: 7 },
    { id: 22, windowId: 1, index: 1, url: "https://b.example/", title: "B", pinned: false, active: false, groupId: -1 },
    { id: 23, windowId: 1, index: 2, url: "https://c.example/", title: "C", pinned: false, active: false, groupId: 99 }, // group gone
  );
  posted.length = 0;
  await vm.runInContext("push()", ctx);
  // (Objects made inside the loaded script belong to another realm: compare as JSON.)
  const sent = JSON.parse(JSON.stringify(posted.at(-1).windows[0].tabs.map((t) => t.group)));
  assert.deepStrictEqual(sent, [{ title: "Work", color: "blue" }, null, null]);

  // Restore: a group request groups the right tabs, with title and colour,
  // in one window, skipping tabs that are pinned, already grouped or elsewhere.
  tabs.length = 0;
  tabs.push(
    { id: 31, windowId: 1, url: "https://a.example/", pinned: false, groupId: -1 },
    { id: 32, windowId: 2, url: "https://a.example/", pinned: false, groupId: -1 }, // same address, other window
    { id: 33, windowId: 1, url: "https://b.example/", pinned: false, groupId: -1 },
    { id: 34, windowId: 1, url: "https://mine.example/", pinned: false, groupId: 5 }, // the user's own group
    { id: 35, windowId: 1, url: "https://pinned.example/", pinned: true, groupId: -1 },
  );
  grouped.length = 0;
  portListener({
    type: "pin",
    id: "g1",
    browser: "chrome",
    urls: [],
    groups: [
      { title: "Docs", color: "green", urls: ["https://a.example/", "https://b.example/", "https://later.example/"] },
      { title: "Mine", color: "red", urls: ["https://mine.example/"] }, // already grouped: left alone
      { title: "Odd", color: "ultraviolet", urls: ["https://a.example/"] }, // bad colour: refused
      { title: "Pinned", color: "red", urls: ["https://pinned.example/"] }, // pinned: left alone
    ],
  });
  await new Promise((r) => setTimeout(r, 450));
  assert.deepStrictEqual(grouped, [[[31], null], [[33], 100]], JSON.stringify(grouped));
  assert.deepStrictEqual(groupInfo[100], { title: "Docs", color: "green" });
  assert.strictEqual(tabs.find((t) => t.id === 32).groupId, -1, "a tab of another window is not pulled in");
  assert.strictEqual(tabs.find((t) => t.id === 34).groupId, 5);

  // The third address opens later: it joins the group already made.
  tabs.push({ id: 36, windowId: 1, url: "https://later.example/", pinned: false, groupId: -1 });
  listeners.tc.forEach((f) => f());
  await new Promise((r) => setTimeout(r, 450));
  assert.deepStrictEqual(grouped.at(-1), [[36], 100]);

  // A request that asks for pins and a group does both, pins first.
  tabs.length = 0;
  tabs.push(
    { id: 41, windowId: 1, url: "https://p.example/", pinned: false, groupId: -1 },
    { id: 42, windowId: 1, url: "https://g.example/", pinned: false, groupId: -1 },
  );
  updates.length = 0;
  grouped.length = 0;
  portListener({
    type: "pin",
    id: "g2",
    browser: "chrome",
    urls: ["https://p.example/"],
    groups: [{ title: "G", color: "cyan", urls: ["https://g.example/"] }],
  });
  await new Promise((r) => setTimeout(r, 450));
  assert.deepStrictEqual(updates, [[41, true]]);
  assert.deepStrictEqual(grouped, [[[42], null]]);

  console.log("extension pinning and groups: all checks passed");
})().catch((e) => { console.error(e); process.exit(1); });
