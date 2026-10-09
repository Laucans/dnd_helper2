// The state machine of the view, over an injected host. It holds no DOM. The coalescing of
// dataVersion announcements and the read-after-write floor are the shell's job (`watch`); what is
// the Micro-UI's own is validation, the monotonic `asOf` guard, the campaign switch and the states.

import type { ShellClient, Unsubscribe, WatchEvent } from '@dnd-helper/micro-ui-shell';
import { NIVEAU_DU_GROUPE } from './identifiers';
import { parsePartyLevel } from './party-level';

/** The only part of the shell this Micro-UI calls: it reads and never writes. */
export type PartyLevelHost = Pick<ShellClient, 'watch'>;

export type PartyLevelState =
  /** No `campagneId`: no read. */
  | { kind: 'idle' }
  /** The first read is pending. */
  | { kind: 'loading' }
  | { kind: 'ready'; level: number | null; pcCount: number; asOf: number; possiblyStale: boolean }
  /** Unknown, archived or foreign campaign: one answer, whatever the reason. The value is dropped. */
  | { kind: 'not-found' }
  /** A failed read with no prior value. */
  | { kind: 'unavailable' };

/** What the person can tell apart: `asOf` only orders results. */
function sameVisible(a: PartyLevelState, b: PartyLevelState): boolean {
  if (a.kind !== 'ready' || b.kind !== 'ready') return a.kind === b.kind;
  return a.level === b.level && a.pcCount === b.pcCount && a.possiblyStale === b.possiblyStale;
}

export class PartyLevelController {
  private current: PartyLevelState = { kind: 'idle' };
  private campagneId: string | undefined;
  /** Bumped on every campaign switch and on dispose: an older listener is ignored. */
  private generation = 0;
  private release: Unsubscribe | null = null;
  private disposed = false;

  constructor(
    private readonly host: PartyLevelHost,
    private readonly onChange: (state: PartyLevelState) => void,
  ) {}

  state(): PartyLevelState {
    return this.current;
  }

  /** `undefined` and `''` make no read. The previous campaign's value is dropped at once. */
  setCampagneId(id: string | undefined): void {
    if (this.disposed) return;
    const next = id === '' ? undefined : id;
    if (next === this.campagneId) return;
    this.campagneId = next;
    this.stop();
    if (next === undefined) {
      this.set({ kind: 'idle' });
      return;
    }
    this.set({ kind: 'loading' });
    // The generation is taken before `watch`: it may call the listener before it returns.
    const generation = this.generation;
    const release = this.host.watch<unknown>(NIVEAU_DU_GROUPE, { campagneId: next }, (event) => {
      this.receive(generation, event);
    });
    if (generation === this.generation) this.release = release;
    else release();
  }

  /** The subscription is released and nothing a pending read brings back has any effect. */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.stop();
  }

  private stop(): void {
    this.generation += 1;
    const release = this.release;
    this.release = null;
    release?.();
  }

  private receive(generation: number, event: WatchEvent<unknown>): void {
    if (generation !== this.generation) return;
    switch (event.kind) {
      case 'not-found':
        this.set({ kind: 'not-found' });
        return;
      case 'error':
      case 'stale':
        this.fail();
        return;
      case 'data': {
        const result = parsePartyLevel(event.data);
        if (result === null) {
          this.fail();
          return;
        }
        // The envelope's `asOf` orders results; the result's own stands in when the answer has none.
        const asOf = event.asOf ?? result.asOf;
        if (this.current.kind === 'ready' && asOf < this.current.asOf) return;
        this.set({ kind: 'ready', level: result.level, pcCount: result.pcCount, asOf, possiblyStale: false });
        return;
      }
    }
  }

  /** Never `0` and never "no party level": either would claim a fact the view does not know. */
  private fail(): void {
    this.set(this.current.kind === 'ready' ? { ...this.current, possiblyStale: true } : { kind: 'unavailable' });
  }

  private set(next: PartyLevelState): void {
    const changed = !sameVisible(this.current, next);
    this.current = next;
    if (changed) this.onChange(next);
  }
}
