-- The read side sees only active rows. A PC of an archived campaign is hidden
-- even before the DataGuard's cascade reaches it. The views expose
-- "campagneId" / id so every read scopes itself to one campaign.
-- security_barrier: a caller's predicate never runs on a hidden row, so an
-- error it raises cannot reveal a tombstoned campaign or PC.
CREATE VIEW campagne_active WITH (security_barrier) AS
  SELECT id, nom, "creeLe"
  FROM campagne
  WHERE "archiveLe" IS NULL;

CREATE VIEW pj_actif WITH (security_barrier) AS
  SELECT p.id, p."campagneId", p.nom, p.classe, p.niveau, p."creeLe"
  FROM pj p
  JOIN campagne c ON c.id = p."campagneId"
  WHERE p."archiveLe" IS NULL AND c."archiveLe" IS NULL;
