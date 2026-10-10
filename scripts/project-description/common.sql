-- #3634: session-local helpers only; no persistent migration objects.
CREATE FUNCTION pg_temp.old_jev(value jsonb) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
 SELECT jsonb_typeof(value) = 'object'
    AND value ? 'probability' AND value ? 'routed'
    AND (value - 'probability' - 'routed') = '{}'::jsonb
    AND value->'routed' = 'true'::jsonb
$$;
CREATE FUNCTION pg_temp.snapshot() RETURNS jsonb LANGUAGE sql AS $$
 SELECT jsonb_build_object(
 'owner', 'fe4de0a3-0cf4-4d79-92e4-4be3fae2c634',
 'projects', (SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY id), '[]') FROM public.projects p
              WHERE user_id = 'fe4de0a3-0cf4-4d79-92e4-4be3fae2c634'),
 'notes', (SELECT coalesce(jsonb_agg(to_jsonb(n) ORDER BY id), '[]') FROM public.notes n
           WHERE user_id = 'fe4de0a3-0cf4-4d79-92e4-4be3fae2c634'),
 'extractions', (SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY id), '[]') FROM public.note_extractions e
                 WHERE user_id = 'fe4de0a3-0cf4-4d79-92e4-4be3fae2c634'))
$$;
CREATE FUNCTION pg_temp.postimage(before_image jsonb) RETURNS jsonb LANGUAGE sql IMMUTABLE AS $$
 SELECT before_image || jsonb_build_object(
 'projects', (SELECT coalesce(jsonb_agg(CASE WHEN item->'metadata' ? 'summary'
   THEN jsonb_set(item, '{metadata}', ((item->'metadata') - 'summary') ||
        jsonb_build_object('description',item->'metadata'->'summary')) ELSE item END ORDER BY item->>'id'), '[]')
   FROM jsonb_array_elements(before_image->'projects') item),
 'notes', (SELECT coalesce(jsonb_agg(CASE WHEN pg_temp.old_jev(item->'metadata'->'project_routing')
   THEN jsonb_set(item, '{metadata}', (item->'metadata') - 'project_routing') ELSE item END ORDER BY item->>'id'), '[]')
   FROM jsonb_array_elements(before_image->'notes') item))
$$;
