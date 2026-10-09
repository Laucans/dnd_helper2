// What the page shows for each state, in French (roadmap: "Contenu en français").
// A pure function: no branch produces the text "0" as a level, and none produces an empty one.

import type { PartyLevelState } from './controller';

export const COPY = {
  loading: 'Chargement…',
  noLevel: 'Pas de niveau de groupe',
  campaignUnavailable: 'Campagne indisponible',
  levelUnavailable: 'Niveau du groupe indisponible',
  stale: 'peut-être pas à jour',
} as const;

export type PartyLevelView =
  | { kind: 'loading'; text: string }
  | { kind: 'unavailable'; text: string }
  | { kind: 'level'; level: string; count: string; stale: boolean }
  | { kind: 'no-level'; text: string; count: string; stale: boolean };

const countText = (pcCount: number): string => `${String(pcCount)} PJ`;

export function toView(state: PartyLevelState): PartyLevelView {
  switch (state.kind) {
    case 'idle':
    case 'not-found':
      return { kind: 'unavailable', text: COPY.campaignUnavailable };
    case 'unavailable':
      return { kind: 'unavailable', text: COPY.levelUnavailable };
    case 'loading':
      return { kind: 'loading', text: COPY.loading };
    case 'ready':
      // `=== null`, never a truthiness check.
      if (state.level === null) return { kind: 'no-level', text: COPY.noLevel, count: countText(state.pcCount), stale: state.possiblyStale };
      return { kind: 'level', level: String(state.level), count: countText(state.pcCount), stale: state.possiblyStale };
  }
}
