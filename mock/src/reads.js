// What the mock server answers this product's Capabilities with, from the
// fixtures `campagnes` and `pjs` (see server.js for the shape of a read).

const active = async (server) => (await server.load('campagnes')).filter((c) => !c.archived);
const pcsOf = async (server, campagneId) => (await server.load('pjs')).filter((p) => p.campagneId === campagneId && !p.archived).sort((a, b) => a.createdAt - b.createdAt);

/** Concept NiveauDuGroupe@1: round-half-up mean, integer division, null without PCs. */
const partyLevel = (levels) => {
  const n = levels.length;
  if (n === 0) return null;
  const sum = levels.reduce((a, b) => a + b, 0);
  return Math.floor((2 * sum + n) / (2 * n));
};

export const READS = {
  'campagne.listerCampagnes': async (server) => (await active(server)).sort((a, b) => b.createdAt - a.createdAt).map(({ id, name }) => ({ id, name })),
  'campagne.listerPjs': async (server, { campagneId }) => {
    if (!(await active(server)).some((c) => c.id === campagneId)) return undefined;
    return (await pcsOf(server, campagneId)).map(({ id, name, class: cls, level }) => ({ id, name, class: cls, level }));
  },
  'campagne.niveauDuGroupe': async (server, { campagneId }) => {
    if (!(await active(server)).some((c) => c.id === campagneId)) return undefined;
    const rows = await pcsOf(server, campagneId);
    return { level: partyLevel(rows.map((r) => r.level)), pcCount: rows.length, model: 'v1', asOf: server.dataVersion };
  },
};
