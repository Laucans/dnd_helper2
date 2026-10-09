// Violation ids → readable field messages (rules 37–41). The map belongs to
// the Micro-UI; the shell hardcodes no id and no UI copy.

import type { CommandResult } from './contracts';

export type ViolationMap = Readonly<Record<string, { field: string; message: string }>>;

export interface MappedViolation {
  id: string | null;
  field: string | null;
  message: string;
  mapped: boolean;
}

export const defaultGeneric = (id: string | null): string => (id === null ? 'rejected' : `rejected: ${id}`);

/**
 * One entry per violation id, in server order. The result itself is never
 * touched: the mapped entries live beside it. A `rejected` result without
 * ids (a human-review rejection) yields a single generic entry.
 */
export function mapViolations(
  result: CommandResult,
  map: ViolationMap,
  generic: (id: string | null) => string = defaultGeneric,
): MappedViolation[] {
  if (result.status !== 'rejected') return [];
  if (result.violations.length === 0) {
    return [{ id: null, field: null, message: generic(null), mapped: false }];
  }
  return result.violations.map((id) => {
    const entry = Object.hasOwn(map, id) ? map[id] : undefined;
    return entry === undefined
      ? { id, field: null, message: generic(id), mapped: false }
      : { id, field: entry.field, message: entry.message, mapped: true };
  });
}
