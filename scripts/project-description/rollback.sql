\set ON_ERROR_STOP on
-- Caller must have loaded exactly one approved JSON snapshot in pg_temp.approved.
-- The generated wrapper starts the transaction. No application startup uses this.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';
LOCK TABLE public.projects, public.notes, public.note_extractions IN SHARE ROW EXCLUSIVE MODE;
\ir common.sql
CREATE TEMP TABLE before_operation ON COMMIT DROP AS SELECT pg_temp.snapshot() AS data;
DO $$ BEGIN
 IF (SELECT count(*) FROM pg_temp.approved) <> 1 OR
    (SELECT data->>'owner' FROM pg_temp.approved) IS DISTINCT FROM 'fe4de0a3-0cf4-4d79-92e4-4be3fae2c634' THEN
   RAISE EXCEPTION 'one snapshot for the approved owner required';
 END IF;
END $$;
-- A row must still equal its complete expected postimage. Later edits cause a skip.
CREATE TEMP TABLE restore_projects ON COMMIT DROP AS
 SELECT r AS preimage FROM jsonb_array_elements((SELECT data->'projects' FROM pg_temp.approved)) r
 JOIN public.projects p ON p.id=(r->>'id')::uuid AND p.user_id='fe4de0a3-0cf4-4d79-92e4-4be3fae2c634'
 WHERE r->'metadata' ? 'summary' AND NOT (r->'metadata' ? 'description')
 AND jsonb_typeof(r->'metadata'->'summary')='string'
 AND to_jsonb(p)=jsonb_set(r,'{metadata}',((r->'metadata') - 'summary') || jsonb_build_object('description',r->'metadata'->'summary'));
CREATE TEMP TABLE restore_notes ON COMMIT DROP AS
 SELECT r AS preimage FROM jsonb_array_elements((SELECT data->'notes' FROM pg_temp.approved)) r
 JOIN public.notes n ON n.id=(r->>'id')::uuid AND n.user_id='fe4de0a3-0cf4-4d79-92e4-4be3fae2c634'
 WHERE pg_temp.old_jev(r->'metadata'->'project_routing') AND to_jsonb(n)=jsonb_set(r,'{metadata}',(r->'metadata') - 'project_routing');
UPDATE public.projects p SET metadata=r.preimage->'metadata' FROM restore_projects r
 WHERE p.id=(r.preimage->>'id')::uuid AND p.user_id='fe4de0a3-0cf4-4d79-92e4-4be3fae2c634';
UPDATE public.notes n SET metadata=r.preimage->'metadata' FROM restore_notes r
 WHERE n.id=(r.preimage->>'id')::uuid AND n.user_id='fe4de0a3-0cf4-4d79-92e4-4be3fae2c634';
DO $$
DECLARE expected jsonb;
BEGIN
 SELECT data || jsonb_build_object(
 'projects',(SELECT coalesce(jsonb_agg(coalesce(r.preimage,j) ORDER BY j->>'id'),'[]')
   FROM jsonb_array_elements(data->'projects') j LEFT JOIN restore_projects r ON r.preimage->>'id'=j->>'id'),
 'notes',(SELECT coalesce(jsonb_agg(coalesce(r.preimage,j) ORDER BY j->>'id'),'[]')
   FROM jsonb_array_elements(data->'notes') j LEFT JOIN restore_notes r ON r.preimage->>'id'=j->>'id'))
 INTO expected FROM before_operation;
 IF pg_temp.snapshot() IS DISTINCT FROM expected THEN RAISE EXCEPTION 'rollback changed unrelated data'; END IF;
END $$;
SELECT jsonb_build_object('projects_restored',(SELECT count(*) FROM restore_projects),
 'notes_restored',(SELECT count(*) FROM restore_notes),
 'projects_skipped',(SELECT coalesce(jsonb_agg(r->>'id'),'[]') FROM jsonb_array_elements((SELECT data->'projects' FROM pg_temp.approved)) r
   WHERE r->'metadata' ? 'summary' AND NOT EXISTS(SELECT FROM restore_projects p WHERE p.preimage->>'id'=r->>'id')),
 'notes_skipped',(SELECT coalesce(jsonb_agg(r->>'id'),'[]') FROM jsonb_array_elements((SELECT data->'notes' FROM pg_temp.approved)) r
   WHERE pg_temp.old_jev(r->'metadata'->'project_routing') AND NOT EXISTS(SELECT FROM restore_notes n WHERE n.preimage->>'id'=r->>'id')),
 'postimage',pg_temp.snapshot());
-- Default is a dry run, even though assertions and UPDATEs ran inside the transaction.
\if :commit
COMMIT;
\else
ROLLBACK;
\endif
