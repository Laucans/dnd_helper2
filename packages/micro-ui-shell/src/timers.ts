export interface Timers {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export const defaultTimers: Timers = {
  now: () => Date.now(),
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
  clearTimeout: (handle) => {
    globalThis.clearTimeout(handle as ReturnType<typeof globalThis.setTimeout>);
  },
};

/** Bounded exponential backoff: `initialMs * factor^attempt`, capped at `maxMs`. */
export function backoff(initialMs: number, maxMs: number, factor: number): (attempt: number) => number {
  return (attempt) => Math.min(maxMs, initialMs * Math.pow(factor, Math.max(0, attempt)));
}

export interface Diagnostic {
  kind: string;
  detail?: string;
}

/** Resolves `true` once `ms` elapsed, `false` as soon as `signal` aborts. */
export function sleep(timers: Timers, ms: number, signal: AbortSignal): Promise<boolean> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve(false);
      return;
    }
    const onAbort = (): void => {
      timers.clearTimeout(handle);
      resolve(false);
    };
    const handle = timers.setTimeout(() => {
      signal.removeEventListener('abort', onAbort);
      resolve(true);
    }, ms);
    signal.addEventListener('abort', onAbort, { once: true });
  });
}
