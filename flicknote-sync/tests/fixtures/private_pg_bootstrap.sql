-- Infrastructure only. All application tables, triggers and policies come from fb.
CREATE ROLE supabase_admin;
CREATE ROLE authenticated;
CREATE ROLE sequin;
CREATE ROLE sequin_replication REPLICATION;
CREATE ROLE api_access_role;
CREATE SCHEMA auth;
CREATE TABLE auth.users (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  email text,
  raw_user_meta_data jsonb,
  raw_app_meta_data jsonb,
  created_at timestamptz DEFAULT now()
);
CREATE FUNCTION auth.uid() RETURNS uuid LANGUAGE sql STABLE AS $$
  SELECT nullif(current_setting('request.jwt.claim.sub', true), '')::uuid
$$;
GRANT USAGE ON SCHEMA public, auth TO authenticated;
