// What the forms send and how a refusal finds its input. No rule lives here:
// a value goes out as typed and the DataGuard judges it. The table below only
// says which input a violation id marks (exact match, never by prefix).

import type { JsonObject, ViolationMap } from '@dnd-helper/micro-ui-shell';

/** The payload keys of the add and edit DataCapabilities. */
export type PcField = 'nom' | 'classe' | 'niveau';

export const PC_FIELDS: readonly PcField[] = ['nom', 'classe', 'niveau'];

export interface PcFormValues {
  nom: string;
  classe: string;
  niveau: string;
}

export const PJ_VIOLATIONS: ViolationMap = {
  'pc-name-required': { field: 'nom', message: 'Le nom est refusé.' },
  'pc-name-unique-in-campaign': { field: 'nom', message: 'Un PJ actif de cette campagne porte déjà ce nom.' },
  'pc-class-required': { field: 'classe', message: 'La classe est refusée.' },
  'pc-level-range': { field: 'niveau', message: 'Le niveau doit être un entier de 1 à 20.' },
};

/** The `field` of a message in the engine's `<Aggregate>.<field>` form; anything else marks no input. */
export function fieldOfMessage(field: string | undefined): PcField | null {
  switch (field) {
    case 'PJ.name':
      return 'nom';
    case 'PJ.class':
      return 'classe';
    case 'PJ.level':
      return 'niveau';
    default:
      return null;
  }
}

/** The field a violation id marks, from the shell's mapped entry. Anything outside the table marks none. */
export function fieldOfViolation(field: string | null): PcField | null {
  return field === 'nom' || field === 'classe' || field === 'niveau' ? field : null;
}

const INTEGER = /^-?\d+$/;

/**
 * A level that parses as an integer goes out as a JSON integer; anything else
 * goes out as typed, so the refusal comes from the DataGuard. An empty input
 * is absent. Nothing is trimmed, clamped, rounded or defaulted.
 */
export function levelPayload(raw: string): number | string | undefined {
  if (raw === '') return undefined;
  if (INTEGER.test(raw)) {
    const n = Number(raw);
    if (Number.isSafeInteger(n)) return n;
  }
  return raw;
}

export function pcPayload(form: PcFormValues): JsonObject {
  const niveau = levelPayload(form.niveau);
  return niveau === undefined ? { nom: form.nom, classe: form.classe } : { nom: form.nom, classe: form.classe, niveau };
}
