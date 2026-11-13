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
