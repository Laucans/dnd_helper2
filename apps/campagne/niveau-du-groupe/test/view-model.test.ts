import { describe, expect, it } from 'vitest';
import type { PartyLevelState } from '../src/controller';
import { COPY, toView } from '../src/view-model';

const ready = (level: number | null, pcCount: number, possiblyStale = false): PartyLevelState => ({ kind: 'ready', level, pcCount, asOf: 1, possiblyStale });

const states: PartyLevelState[] = [
  { kind: 'idle' },
  { kind: 'loading' },
  ready(4, 2),
  ready(4, 2, true),
  ready(null, 0),
  ready(null, 0, true),
  { kind: 'not-found' },
  { kind: 'unavailable' },
];

describe('toView', () => {
  it('shows the level and the count of a result as received', () => {
    expect(toView(ready(4, 2))).toEqual({ kind: 'level', level: '4', count: '2 PJ', stale: false });
    expect(toView(ready(7, 1))).toEqual({ kind: 'level', level: '7', count: '1 PJ', stale: false });
    expect(toView(ready(1, 1))).toMatchObject({ level: '1' });
    expect(toView(ready(20, 20))).toMatchObject({ level: '20', count: '20 PJ' });
  });

  it('an empty party is "no party level" with a count of 0, never 0 or an error', () => {
    expect(toView(ready(null, 0))).toEqual({ kind: 'no-level', text: COPY.noLevel, count: '0 PJ', stale: false });
  });

  it('never produces "0" or an empty text where the level should be', () => {
    for (const state of states) {
      const view = toView(state);
      const text = view.kind === 'level' ? view.level : view.text;
      expect(text).not.toBe('0');
      expect(text).not.toBe('');
    }
  });

  it('only "ready" carries a stale mark, and only when the value is possibly stale', () => {
    for (const state of states) {
      const view = toView(state);
      const stale = 'stale' in view && view.stale;
      expect(stale).toBe(state.kind === 'ready' && state.possiblyStale);
    }
  });

  it('"not found" and "no campaign" read as an unavailable campaign, not as an empty party', () => {
    expect(toView({ kind: 'not-found' })).toEqual({ kind: 'unavailable', text: COPY.campaignUnavailable });
    expect(toView({ kind: 'idle' })).toEqual({ kind: 'unavailable', text: COPY.campaignUnavailable });
    expect(toView({ kind: 'not-found' })).not.toEqual(toView(ready(null, 0)));
  });

  it('a failed read with no value is unavailable; the first read is loading', () => {
    expect(toView({ kind: 'unavailable' })).toEqual({ kind: 'unavailable', text: COPY.levelUnavailable });
    expect(toView({ kind: 'loading' })).toEqual({ kind: 'loading', text: COPY.loading });
  });
});
