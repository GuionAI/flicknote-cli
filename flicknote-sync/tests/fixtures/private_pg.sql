-- Test-owned subset of current fb notes/projects/note_extractions schema.
CREATE EXTENSION pgroonga;
CREATE SCHEMA auth;
CREATE ROLE authenticated NOLOGIN;
CREATE ROLE flicknote_test LOGIN PASSWORD 'test' IN ROLE authenticated;
CREATE FUNCTION auth.uid() RETURNS uuid LANGUAGE sql STABLE AS $$ SELECT nullif(current_setting('request.jwt.claim.sub', true), '')::uuid $$;
CREATE TABLE auth.users (id uuid PRIMARY KEY);
INSERT INTO auth.users VALUES ('11111111-1111-4111-8111-111111111111'), ('22222222-2222-4222-8222-222222222222');
CREATE TABLE projects (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), user_id uuid NOT NULL DEFAULT auth.uid() REFERENCES auth.users(id),
 name text NOT NULL, color text, metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
 is_archived boolean NOT NULL DEFAULT false, created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE notes (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), short_id integer NOT NULL,
 user_id uuid NOT NULL DEFAULT auth.uid() REFERENCES auth.users(id), type text NOT NULL DEFAULT 'normal',
 status text NOT NULL DEFAULT 'ready', title text, content text, summary text,
 flag boolean NOT NULL DEFAULT false, project_id uuid REFERENCES projects(id) ON DELETE SET NULL,
 metadata jsonb, source jsonb, created_at timestamptz NOT NULL DEFAULT now(),
 updated_at timestamptz NOT NULL DEFAULT now(), deleted_at timestamptz,
 UNIQUE(user_id,short_id)
);
CREATE TABLE note_extractions (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(), note_id uuid NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
 user_id uuid NOT NULL DEFAULT auth.uid() REFERENCES auth.users(id), key text NOT NULL,
 value text NOT NULL, UNIQUE(note_id,key,value)
);
CREATE TABLE user_short_id_counters (user_id uuid PRIMARY KEY REFERENCES auth.users(id), next_id integer NOT NULL);
CREATE FUNCTION assign_note_short_id() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER AS $$ BEGIN
  IF NEW.short_id IS NULL THEN
    INSERT INTO user_short_id_counters(user_id,next_id) VALUES(NEW.user_id,2)
    ON CONFLICT(user_id) DO UPDATE SET next_id=user_short_id_counters.next_id+1
    RETURNING next_id-1 INTO NEW.short_id;
  END IF;
  RETURN NEW;
END $$;
ALTER TABLE notes ALTER COLUMN short_id DROP NOT NULL;
CREATE TRIGGER assign_note_short_id BEFORE INSERT ON notes FOR EACH ROW EXECUTE FUNCTION assign_note_short_id();
ALTER TABLE notes ENABLE ROW LEVEL SECURITY;
ALTER TABLE projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE note_extractions ENABLE ROW LEVEL SECURITY;
CREATE POLICY own_notes ON notes TO authenticated USING (user_id=auth.uid()) WITH CHECK (user_id=auth.uid());
CREATE POLICY own_projects ON projects TO authenticated USING (user_id=auth.uid()) WITH CHECK (user_id=auth.uid());
CREATE POLICY own_extractions ON note_extractions TO authenticated USING (user_id=auth.uid()) WITH CHECK (user_id=auth.uid());
GRANT USAGE ON SCHEMA public,auth TO authenticated;
GRANT EXECUTE ON FUNCTION auth.uid() TO authenticated;
GRANT SELECT,INSERT,UPDATE,DELETE ON notes,projects,note_extractions TO authenticated;
CREATE INDEX notes_title_pgroonga ON notes USING pgroonga(title);
CREATE INDEX notes_summary_pgroonga ON notes USING pgroonga(summary);
CREATE INDEX notes_content_pgroonga ON notes USING pgroonga(content);
