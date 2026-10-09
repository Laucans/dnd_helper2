// The `NiveauDuGroupe@1` result as the Capability answers it, validated and nothing more:
// the level is displayed as received, never rounded, recomputed or derived from PC rows.

export interface PartyLevel {
  /** `null`: an empty party. An integer from 1 to 20 otherwise. */
  level: number | null;
  pcCount: number;
  model: 'v1';
  asOf: number;
}

const isCount = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;

const isLevel = (value: unknown): value is number => typeof value === 'number' && Number.isInteger(value) && value >= 1 && value <= 20;

/**
 * `null` when the result is malformed. A missing `level` is not `null` (the key must be there),
 * and `level` is `null` exactly when `pcCount` is 0.
 */
export function parsePartyLevel(data: unknown): PartyLevel | null {
  if (typeof data !== 'object' || data === null || Array.isArray(data)) return null;
  if (!Object.hasOwn(data, 'level')) return null;
  const { level, pcCount, model, asOf } = data as Record<string, unknown>;
  if (level !== null && !isLevel(level)) return null;
  if (!isCount(pcCount) || !isCount(asOf)) return null;
  if (model !== 'v1') return null;
  if ((level === null) !== (pcCount === 0)) return null;
  return { level, pcCount, model, asOf };
}
