-- edge_bank: read-only access for the Dmux logins (run on both servers, last).
SET client_min_messages = warning;
-- Nobody but the owner may create objects: the logins must look read-only to Dmux.
REVOKE CREATE ON SCHEMA public FROM PUBLIC;
GRANT USAGE ON SCHEMA public TO edge_reader, edge_reader_rls;
GRANT CONNECT ON DATABASE edge_bank TO edge_reader, edge_reader_rls;
DO $$
DECLARE s text;
BEGIN
  FOREACH s IN ARRAY ARRAY['same', 'differs', 'structure', 'tricky', 'big'] LOOP
    EXECUTE format('GRANT USAGE ON SCHEMA %I TO edge_reader, edge_reader_rls', s);
    EXECUTE format('GRANT SELECT ON ALL TABLES IN SCHEMA %I TO edge_reader, edge_reader_rls', s);
    EXECUTE format('GRANT SELECT ON ALL SEQUENCES IN SCHEMA %I TO edge_reader, edge_reader_rls', s);
  END LOOP;
END $$;
