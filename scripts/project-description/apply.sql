\set ON_ERROR_STOP on
-- Caller must have loaded exactly one approved JSON snapshot in pg_temp.approved.
-- The generated wrapper starts the transaction. No application startup uses this.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';
LOCK TABLE public.projects, public.notes, public.note_extractions IN SHARE ROW EXCLUSIVE MODE;
\ir common.sql
CREATE TEMP TABLE before_operation ON COMMIT DROP AS SELECT pg_temp.snapshot() AS data;
DO $$
DECLARE approved_image jsonb;
BEGIN
 IF (SELECT count(*) FROM pg_temp.approved) <> 1 THEN RAISE EXCEPTION 'one snapshot required'; END IF;
 SELECT data INTO approved_image FROM pg_temp.approved;
 IF approved_image IS DISTINCT FROM pg_temp.snapshot() THEN
   RAISE EXCEPTION 'owner snapshot changed; preview and inspect again';
 END IF;
 IF EXISTS (SELECT FROM public.projects WHERE user_id='fe4de0a3-0cf4-4d79-92e4-4be3fae2c634'
   AND metadata ? 'summary' AND (metadata ? 'description' OR jsonb_typeof(metadata->'summary') <> 'string')) THEN
   RAISE EXCEPTION 'description conflict or non-string summary; no changes applied';
 END IF;
END $$;
UPDATE public.projects SET metadata=(metadata - 'summary') || jsonb_build_object('description',metadata->'summary')
 WHERE user_id='fe4de0a3-0cf4-4d79-92e4-4be3fae2c634' AND metadata ? 'summary';
UPDATE public.notes SET metadata=metadata - 'project_routing'
 WHERE user_id='fe4de0a3-0cf4-4d79-92e4-4be3fae2c634' AND pg_temp.old_jev(metadata->'project_routing');
DO $$ BEGIN
 IF pg_temp.snapshot() IS DISTINCT FROM pg_temp.postimage((SELECT data FROM pg_temp.approved)) THEN
   RAISE EXCEPTION 'postimage mismatch: unrelated fields, timestamps or extractions changed';
 END IF;
END $$;
SELECT jsonb_build_object('projects_changed',(SELECT count(*) FROM jsonb_array_elements((SELECT data->'projects' FROM before_operation)) r WHERE r->'metadata' ? 'summary'),
 'notes_changed',(SELECT count(*) FROM jsonb_array_elements((SELECT data->'notes' FROM before_operation)) r WHERE pg_temp.old_jev(r->'metadata'->'project_routing')),
 'postimage',pg_temp.snapshot());
-- Default is a dry run, even though assertions and UPDATEs ran inside the transaction.
\if :commit
COMMIT;
\else
ROLLBACK;
\endif
