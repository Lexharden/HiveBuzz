import type { LiveEvent, Platform } from "./types";

export type FeedFilter = "all" | Platform;

export const platformOf = (ev: LiveEvent): Platform => ev.platform ?? "tiktok";

export function filterEvents(events: LiveEvent[], filter: FeedFilter): LiveEvent[] {
  return filter === "all" ? events : events.filter((e) => platformOf(e) === filter);
}

export function countByPlatform(events: LiveEvent[]): Record<Platform, number> {
  const out: Record<Platform, number> = { tiktok: 0, twitch: 0 };
  for (const e of events) out[platformOf(e)] += 1;
  return out;
}
