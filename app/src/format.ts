import type { CaptureStatus } from "./bindings/CaptureStatus";
import type { RestoreStatus } from "./bindings/RestoreStatus";

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

/** "3 minutes ago", "yesterday", ... */
export function relativeTime(iso: string, now = Date.now()): string {
  const seconds = Math.round((new Date(iso).getTime() - now) / 1000);
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ["year", 31_536_000],
    ["month", 2_592_000],
    ["week", 604_800],
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60],
  ];
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return rtf.format(Math.round(seconds / size), unit);
  }
  return rtf.format(seconds, "second");
}

export function absoluteTime(iso: string): string {
  return new Date(iso).toLocaleString();
}

export const restoreStatusLabel: Record<RestoreStatus, string> = {
  success: "Restored",
  partial_success: "Partly restored",
  failed: "Failed",
  skipped: "Skipped",
  requires_user_intervention: "Needs you",
};

export const restoreStatusTone: Record<RestoreStatus, Tone> = {
  success: "ok",
  partial_success: "warn",
  failed: "bad",
  skipped: "muted",
  requires_user_intervention: "warn",
};

export const captureStatusTone: Record<CaptureStatus, Tone> = {
  complete: "ok",
  partial: "warn",
  failed: "bad",
};

export type Tone = "ok" | "warn" | "bad" | "muted";

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** "412 MB", "1.3 GB". */
export function formatBytes(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${Math.round(mb)} MB`;
}

/** CPU share in percent: "0.4%", "12%". */
export function formatCpu(percent: number): string {
  return percent < 10 ? `${percent.toFixed(1)}%` : `${Math.round(percent)}%`;
}

const APP_NAMES: Record<string, string> = {
  chrome: "Google Chrome",
  msedge: "Microsoft Edge",
  firefox: "Firefox",
  brave: "Brave",
  code: "VS Code",
  "code - insiders": "VS Code Insiders",
  cursor: "Cursor",
  devenv: "Visual Studio",
  idea64: "IntelliJ IDEA",
  pycharm64: "PyCharm",
  webstorm64: "WebStorm",
  windowsterminal: "Windows Terminal",
  wt: "Windows Terminal",
  powershell: "PowerShell",
  pwsh: "PowerShell",
  cmd: "Command Prompt",
  explorer: "File Explorer",
  discord: "Discord",
  slack: "Slack",
  teams: "Microsoft Teams",
  "ms-teams": "Microsoft Teams",
  spotify: "Spotify",
  notion: "Notion",
  obsidian: "Obsidian",
  winword: "Word",
  excel: "Excel",
  powerpnt: "PowerPoint",
  onenote: "OneNote",
  outlook: "Outlook",
  notepad: "Notepad",
  claude: "Claude",
  steam: "Steam",
  figma: "Figma",
  postman: "Postman",
  docker: "Docker Desktop",
  "github desktop": "GitHub Desktop",
};

/** A readable program name for an executable's id ("windowsterminal" -> "Windows Terminal"). */
export function friendlyAppName(applicationId: string): string {
  const id = applicationId.toLowerCase();
  const known = APP_NAMES[id];
  if (known) return known;
  // Store apps are saved as "Family!App": show the app part.
  const base = id.includes("!") ? id.split("!").pop()! : id;
  const words = base.replace(/[-_.]+/g, " ").trim();
  return words.replace(/\b\w/g, (c) => c.toUpperCase());
}

/** "6.2 s" or "1 min 12 s" for a restore or switch. */
export function formatDuration(ms: number): string {
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} s`;
  return `${Math.floor(s / 60)} min ${Math.round(s % 60)} s`;
}
