// What the mock server does with this product's commands: the aggregates'
// invariants as the DataGuard judges them (the violation ids the Micro-UIs
// know), applied to the fixtures `campagnes` and `pjs`.

const active = async (server) => (await server.load('campagnes')).filter((c) => !c.archived);
const pcsOf = async (server, campagneId) => (await server.load('pjs')).filter((p) => p.campagneId === campagneId && !p.archived);

const pcViolations = async (server, payload, campagneId, selfId) => {
  const found = [];
  const name = typeof payload.nom === 'string' ? payload.nom : '';
  if (!name.trim()) found.push('pc-name-required');
  else if ((await pcsOf(server, campagneId)).some((p) => p.id !== selfId && p.name.trim().toLowerCase() === name.trim().toLowerCase())) found.push('pc-name-unique-in-campaign');
  if (typeof payload.classe !== 'string' || !payload.classe.trim()) found.push('pc-class-required');
  if (!Number.isInteger(payload.niveau) || payload.niveau < 1 || payload.niveau > 20) found.push('pc-level-range');
  return found;
};

export const COMMANDS = {
  'campagne.creerCampagne': async (server, { payload }) => {
    const name = typeof payload.name === 'string' ? payload.name : '';
    if (!name.trim()) return ['campaign-name-required'];
    if (name.length > 80) return ['campaign-name-length'];
    (await server.load('campagnes')).push({ id: server.uuid(), name, createdAt: server.next(), archived: false });
  },
  'campagne.archiverCampagne': async (server, { targetId }) => {
    const campagne = (await active(server)).find((c) => c.id === targetId);
    if (!campagne) return 'not-found';
    campagne.archived = true;
  },
  'campagne.ajouterPj': async (server, { payload }) => {
    const campagne = (await active(server)).find((c) => c.id === payload.campagneId);
    if (!campagne) return 'not-found';
    const violations = await pcViolations(server, payload, campagne.id, null);
    if (violations.length) return violations;
    (await server.load('pjs')).push({ id: server.uuid(), campagneId: campagne.id, name: payload.nom, class: payload.classe, level: payload.niveau, createdAt: server.next(), archived: false });
  },
  'campagne.modifierPJ': async (server, { payload, targetId }) => {
    const pc = (await server.load('pjs')).find((p) => p.id === targetId && !p.archived);
    if (!pc) return 'not-found';
    const violations = await pcViolations(server, payload, pc.campagneId, pc.id);
    if (violations.length) return violations;
    pc.name = payload.nom; pc.class = payload.classe; pc.level = payload.niveau;
  },
  'campagne.archiverPJ': async (server, { targetId }) => {
    const pc = (await server.load('pjs')).find((p) => p.id === targetId && !p.archived);
    if (!pc) return 'not-found';
    pc.archived = true;
  },
};
