-- The read-only role of the Capabilities: SELECT on the views, nothing on any
-- table. Created without login and without password; its login is provisioned
-- outside the repository. Roles are cluster-wide, so creation is a no-op when
-- the role exists, and the handler absorbs a race between two databases.
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'app_lecture') THEN
    BEGIN
      CREATE ROLE app_lecture NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOREPLICATION NOBYPASSRLS;
    EXCEPTION WHEN duplicate_object OR unique_violation THEN
      NULL;
    END;
  END IF;
END
$$;

DO $$
BEGIN
  EXECUTE format('REVOKE TEMPORARY ON DATABASE %I FROM PUBLIC', current_database());
END
$$;

REVOKE CREATE ON SCHEMA public FROM PUBLIC;

GRANT SELECT ON campagne_active, pj_actif TO app_lecture;
