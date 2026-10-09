-- Where a PC came from, and its id there. Purely additive: the column
-- default gives every existing row, and every insert that names no origin,
-- `manual`; `idExterne` stays null for them. No index or unique constraint on
-- `idExterne`: one id per campaign is a DataGuard invariant, like name
-- uniqueness. The CHECKs are the row's last line: two origins, and a
-- `dndbeyond` row always carries its id (a `manual` row never does).
-- The views `pj_actif` and `campagne_active` and the grants are unchanged,
-- so neither column is readable through them.
ALTER TABLE pj ADD COLUMN origine text NOT NULL DEFAULT 'manual'
  CONSTRAINT pj_origine_valeur CHECK (origine IN ('manual', 'dndbeyond'));

ALTER TABLE pj ADD COLUMN "idExterne" text;

ALTER TABLE pj ADD CONSTRAINT pj_origine_id_externe
  CHECK ((origine = 'dndbeyond') = ("idExterne" IS NOT NULL));
