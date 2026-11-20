/**
 * Tracks which of a series of async requests is the most recent, so a
 * caller can tell whether a response that just resolved still matches the
 * latest request or is a stale, out-of-order one that should be ignored.
 */
export interface LatestRequestTracker {
  /** Call synchronously before starting a new request. Returns its id. */
  start(): number;
  /** True when `id` is still the most recently started request. */
  isCurrent(id: number): boolean;
}

export function createLatestRequestTracker(): LatestRequestTracker {
  let latest = 0;
  return {
    start() {
      latest += 1;
      return latest;
    },
    isCurrent(id: number) {
      return id === latest;
    },
  };
}
