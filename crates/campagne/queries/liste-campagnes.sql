SELECT json_build_object(
  'campagnes', COALESCE((
    SELECT json_agg(
      json_build_object('id', c.id, 'nom', c.nom, 'creeLe', c."creeLe")
      ORDER BY c."creeLe" DESC, c.id ASC)
    FROM campagne_active c), '[]'::json))
