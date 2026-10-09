// The only place the Micro-UI names a Capability or a DataCapability. They
// equal the strings of `micro-ui.json` and of the sibling manifests, character
// for character (`test/manifest.test.ts` compares them).

export const LISTER_PJS = 'campagne.listerPjs';

export const AJOUTER_PJ = { dataCapability: 'campagne.ajouterPj', version: 1, mode: 'relative' } as const;

export const MODIFIER_PJ = {
  dataCapability: 'campagne.modifierPJ',
  version: 1,
  mode: 'confirm_on_stale',
  aggregate: 'PJ',
} as const;

export const ARCHIVER_PJ = {
  dataCapability: 'campagne.archiverPJ',
  version: 1,
  mode: 'overwrite',
  aggregate: 'PJ',
} as const;

/** One row of `lister-pjs`, as the Data layer's view exposes it. */
export interface PcRow {
  id: string;
  name: string;
  class: string;
  level: number;
}
