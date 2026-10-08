-- A campaign. Archiving sets "archiveLe" (a tombstone); a row is never removed.
-- Text is refused, not normalised, when it is not NFC: normalising on write is
-- the DataGuard's job.
CREATE TABLE campagne (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  nom text NOT NULL CONSTRAINT campagne_nom_nfc CHECK (nom IS NFC NORMALIZED),
  "creeLe" timestamptz NOT NULL DEFAULT now(),
  "archiveLe" timestamptz
);

REVOKE ALL ON campagne FROM PUBLIC;
