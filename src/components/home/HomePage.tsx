import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import { isActive } from "../../lib/connectionText";
import type { Page } from "../../lib/nav";
import type { AppInfo, LiveEvent, Platform, StatusUpdate } from "../../lib/types";
import { EventFeed } from "../EventFeed";
import { SimulatorPanel } from "../SimulatorPanel";
import { ConnectionCard } from "./ConnectionCard";
import { GettingStarted } from "./GettingStarted";
import { TwitchUpgrade } from "./TwitchUpgrade";

interface Props {
  statuses: Record<Platform, StatusUpdate>;
  events: LiveEvent[];
  info: AppInfo | null;
  onClear: () => void;
  onNavigate: (page: Page) => void;
  onInfoChanged: () => Promise<void>;
}

/** Inicio: conectar, ver qué pasa y los primeros pasos. Lo avanzado vive en el resto del menú. */
export function HomePage({ statuses, events, info, onClear, onNavigate, onInfoChanged }: Props) {
  const [rules, setRules] = useState(0);
  useEffect(() => {
    void api.listRules().then((r) => setRules(r.length)).catch(() => undefined);
  }, []);

  const connectedEver = isActive(statuses.tiktok) || isActive(statuses.twitch) || Boolean(info?.lastUsername || info?.lastTwitchChannel) || events.length > 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto">
      <div data-tour="connections" className="grid gap-4 lg:grid-cols-2">
        <ConnectionCard platform="tiktok" status={statuses.tiktok} lastName={info?.lastUsername ?? null} onChanged={() => void onInfoChanged()} />
        <ConnectionCard platform="twitch" status={statuses.twitch} lastName={info?.lastTwitchChannel ?? null} onChanged={() => void onInfoChanged()}>
          <TwitchUpgrade status={statuses.twitch} channel={info?.lastTwitchChannel ?? null} />
        </ConnectionCard>
      </div>
      <GettingStarted connectedEver={connectedEver} rules={rules} onNavigate={onNavigate} />
      <div data-tour="feed" className="flex min-h-[16rem] flex-1 flex-col">
        <EventFeed events={events} onClear={onClear} />
      </div>
      <SimulatorPanel />
    </div>
  );
}
