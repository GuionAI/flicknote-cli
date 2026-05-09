CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE notes (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid NOT NULL DEFAULT gen_random_uuid(),
  type text NOT NULL DEFAULT 'normal',
  status text NOT NULL DEFAULT 'ai_queued',
  title text,
  content text,
  summary text,
  is_flagged boolean,
  project_id uuid,
  metadata jsonb,
  source jsonb,
  external_id jsonb,
  created_at timestamptz,
  updated_at timestamptz,
  deleted_at timestamptz
);

CREATE TABLE projects (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid NOT NULL DEFAULT gen_random_uuid(),
  name text NOT NULL,
  color text,
  prompt_id uuid,
  keyterm_id uuid,
  is_archived boolean,
  created_at timestamptz
);

CREATE TABLE prompts (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid NOT NULL DEFAULT gen_random_uuid(),
  title text NOT NULL,
  description text,
  prompt text NOT NULL,
  created_at timestamptz
);

CREATE TABLE keyterms (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id uuid NOT NULL DEFAULT gen_random_uuid(),
  name text NOT NULL,
  description text,
  content text,
  created_at timestamptz,
  updated_at timestamptz
);

CREATE TABLE note_extractions (
  note_id uuid,
  user_id uuid,
  type text,
  value text
);
