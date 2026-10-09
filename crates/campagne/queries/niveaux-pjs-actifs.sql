SELECT CASE WHEN EXISTS (SELECT 1 FROM campagne_active c WHERE c.id = $1) THEN
  json_build_object(
    'levels', COALESCE((
      SELECT json_agg(p.niveau ORDER BY p."creeLe" ASC, p.id ASC)
      FROM pj_actif p WHERE p."campagneId" = $1), '[]'::json),
    'dataVersion', (SELECT v."dataVersion" FROM data_version v))
END
