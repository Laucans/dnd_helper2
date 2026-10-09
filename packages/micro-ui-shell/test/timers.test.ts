import { describe, expect, it } from 'vitest';

import { backoff, sleep } from '../src/timers';
import { FakeClock } from './fakes';

describe('backoff', () => {
  it('grows by the factor and never exceeds the cap', () => {
    const delay = backoff(100, 500, 2);
    expect([0, 1, 2, 3, 4, 10, 1000].map(delay)).toEqual([100, 200, 400, 500, 500, 500, 500]);
  });

  it('treats a negative attempt as the first one', () => {
    expect(backoff(100, 500, 2)(-3)).toBe(100);
  });
});

describe('sleep', () => {
  it('resolves true once the delay elapsed', async () => {
    const clock = new FakeClock();
    const done = sleep(clock, 50, new AbortController().signal);
    await clock.advance(49);
    expect(clock.pending).toBe(1);
    await clock.advance(1);
    expect(await done).toBe(true);
  });

  it('resolves false on abort and leaves no timer behind', async () => {
    const clock = new FakeClock();
    const ac = new AbortController();
    const done = sleep(clock, 50, ac.signal);
    ac.abort();
    expect(await done).toBe(false);
    expect(clock.pending).toBe(0);
  });

  it('schedules nothing when the signal is already aborted', async () => {
    const clock = new FakeClock();
    const ac = new AbortController();
    ac.abort();
    expect(await sleep(clock, 50, ac.signal)).toBe(false);
    expect(clock.pending).toBe(0);
  });
});
