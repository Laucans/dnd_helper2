SELECT CASE WHEN EXISTS (SELECT 1 FROM campagne_active c WHERE c.id = $1) THEN
  json_build_object(
    'pjs', COALESCE((
      SELECT json_agg(
        json_build_object('id', p.id, 'nom', p.nom, 'classe', p.classe, 'niveau', p.niveau, 'creeLe', p."creeLe")
        ORDER BY p."creeLe" ASC, p.id ASC)
      FROM pj_actif p WHERE p."campagneId" = $1), '[]'::json))
END
