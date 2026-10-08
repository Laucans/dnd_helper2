-- A player character, in exactly one campaign that never changes. The
-- cascade-to-archive of a campaign belongs to the DataGuard, not to the FK.
-- Structural constraints only: lengths, the 1-20 level range and name
-- uniqueness are DataGuard invariants.
CREATE TABLE pj (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "campagneId" uuid NOT NULL
    CONSTRAINT pj_campagne_fk REFERENCES campagne (id) ON UPDATE RESTRICT ON DELETE RESTRICT,
  nom text NOT NULL CONSTRAINT pj_nom_nfc CHECK (nom IS NFC NORMALIZED),
  classe text NOT NULL CONSTRAINT pj_classe_nfc CHECK (classe IS NFC NORMALIZED),
  niveau integer NOT NULL,
  "creeLe" timestamptz NOT NULL DEFAULT now(),
  "archiveLe" timestamptz
);

CREATE INDEX pj_campagne_idx ON pj ("campagneId");

REVOKE ALL ON pj FROM PUBLIC;
