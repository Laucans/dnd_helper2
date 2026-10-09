// The fake sits at the host-shell interface: it records `watch` calls and lets a test emit events.
// It mocks nothing at the Capability call site.

import type { JsonObject, WatchEvent } from '@dnd-helper/micro-ui-shell';
import type { PartyLevelHost } from '../src/controller';

export type Listener = (event: WatchEvent<unknown>) => void;

export interface WatchCall {
  capability: string;
  variables: JsonObject;
  listener: Listener;
  released: boolean;
}

export class FakeHost implements PartyLevelHost {
  readonly calls: WatchCall[] = [];
  /** Called inside `watch`, before it returns: a shell replaying a loaded key does this. */
  onWatch: ((call: WatchCall) => void) | null = null;

  readonly watch: PartyLevelHost['watch'] = <T>(capability: string, variables: JsonObject, listener: (e: WatchEvent<T>) => void) => {
    const call: WatchCall = { capability, variables, listener: listener as Listener, released: false };
    this.calls.push(call);
    this.onWatch?.(call);
    return () => {
      call.released = true;
    };
  };

  emit(index: number, event: WatchEvent<unknown>): void {
    this.calls[index]!.listener(event);
  }

  get active(): WatchCall[] {
    return this.calls.filter((c) => !c.released);
  }
}

export const result = (level: number | null, pcCount: number, asOf: number): unknown => ({ level, pcCount, model: 'v1', asOf });

export const data = (level: number | null, pcCount: number, asOf: number): WatchEvent<unknown> => ({
  kind: 'data',
  data: result(level, pcCount, asOf),
  asOf,
});
