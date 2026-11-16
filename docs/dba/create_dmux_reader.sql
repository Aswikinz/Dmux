-- Creates the SELECT-only login Dmux expects. Works on PostgreSQL 9.4 and later.
-- A database administrator runs this once on the old database and once on the new.
--
--   psql -d <database> -v pw="'choose-a-strong-password'" -v schema=public -f create_dmux_reader.sql
--
-- Run it once per schema that Dmux should compare, changing -v schema=...
-- (the CREATE ROLE line will report "already exists" on later runs; that is fine).

CREATE ROLE dmux_reader LOGIN PASSWORD :pw
  NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION;

-- Belt and braces: sessions start read-only, with limits that protect the server.
ALTER ROLE dmux_reader SET default_transaction_read_only = on;
ALTER ROLE dmux_reader SET lock_timeout = '5s';
ALTER ROLE dmux_reader SET idle_in_transaction_session_timeout = '10min';  -- 9.6 and later; remove this line on 9.4 and 9.5

SELECT format('GRANT CONNECT ON DATABASE %I TO dmux_reader', current_database()) \gexec
GRANT USAGE ON SCHEMA :"schema" TO dmux_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA :"schema" TO dmux_reader;
GRANT SELECT ON ALL SEQUENCES IN SCHEMA :"schema" TO dmux_reader;

-- Dmux needs nothing else. It never needs INSERT, UPDATE, DELETE, CREATE or
-- superuser rights, and it warns if the login it is given has them.
--
-- To remove the login later:
--   REASSIGN OWNED BY dmux_reader TO postgres; DROP OWNED BY dmux_reader; DROP ROLE dmux_reader;
