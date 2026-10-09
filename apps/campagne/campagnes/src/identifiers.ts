// The only three identifiers this Micro-UI uses (SPEC rules 1 and 5), with the
// version and mode of each DataCapability as its manifest declares them.
// `test/manifest.test.ts` compares these constants with the counterpart
// manifests, so a drift is a red test and not a runtime surprise.

export const LISTER_CAMPAGNES = 'campagne.listerCampagnes';

export const CREER_CAMPAGNE = {
  id: 'campagne.creerCampagne',
  version: 1,
  mode: 'relative',
  aggregate: 'Campagne',
} as const;

export const ARCHIVER_CAMPAGNE = {
  id: 'campagne.archiverCampagne',
  version: 1,
  mode: 'overwrite',
  aggregate: 'Campagne',
} as const;

/** The name under which the selected campaign id is handed to the host (`$ctx.campagne`). */
export const CONTEXT_NAME = 'campagne';
