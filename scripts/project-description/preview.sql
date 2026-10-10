\set ON_ERROR_STOP on
BEGIN ISOLATION LEVEL REPEATABLE READ;
\ir common.sql
-- One JSON line: every owner row and extraction, including archived rows.
SELECT pg_temp.snapshot();
ROLLBACK;
