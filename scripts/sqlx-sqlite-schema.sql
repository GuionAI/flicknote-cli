CREATE TABLE notes (
  id TEXT PRIMARY KEY,
  short_id INTEGER,
  user_id TEXT,
  type TEXT,
  status TEXT,
  title TEXT,
  content TEXT,
  summary TEXT,
  is_flagged INTEGER,
  project_id TEXT,
  metadata TEXT,
  source TEXT,
  created_at TEXT,
  updated_at TEXT,
  deleted_at TEXT
);

CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  user_id TEXT,
  name TEXT,
  color TEXT,
  prompt_id TEXT,
  keyterm_id TEXT,
  is_archived INTEGER,
  created_at TEXT
);

CREATE TABLE prompts (
  id TEXT PRIMARY KEY,
  user_id TEXT,
  title TEXT,
  description TEXT,
  prompt TEXT,
  created_at TEXT
);

CREATE TABLE keyterms (
  id TEXT PRIMARY KEY,
  user_id TEXT,
  name TEXT,
  description TEXT,
  content TEXT,
  created_at TEXT,
  updated_at TEXT
);

CREATE TABLE note_extractions (
  id TEXT,
  note_id TEXT,
  user_id TEXT,
  type TEXT,
  value TEXT
);
