// UI copy (French, roadmap: "Contenu en français"). The violation ids stay
// untranslated in the data: this map only decides what the GM reads.

import type { QueueState, ViolationMap } from '@dnd-helper/micro-ui-shell';

export const VIOLATIONS: ViolationMap = {
  'campaign-name-required': { field: 'name', message: 'Le nom de la campagne est obligatoire.' },
  'campaign-name-length': { field: 'name', message: 'Le nom de la campagne est trop long.' },
};

export const STATE_LABELS: Readonly<Record<QueueState, string>> = {
  queued: 'En file d’attente',
  awaiting_confirmation: 'En attente de confirmation',
  confirmed: 'Confirmée',
  awaiting_review: 'En attente de relecture',
  parked: 'Mise de côté',
  applied: 'Appliquée',
  rejected: 'Refusée',
  cancelled: 'Annulée',
  expired: 'Expirée',
};

/** A pending state the shell reports is always shown, never treated as an error. */
export function pendingLabel(state: string): string {
  return Object.hasOwn(STATE_LABELS, state) ? STATE_LABELS[state as QueueState] : `En cours (${state})`;
}
