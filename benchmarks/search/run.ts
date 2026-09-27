import { Database } from "bun:sqlite";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { join } from "node:path";

type Audience = "human" | "agent";
type QueryGroup = "lexical" | "broad" | "cross-language" | "observed-agent" | "challenge" | "synthetic-input";
type Query = {
  id: string;
  audience: Audience;
  group: QueryGroup;
  task?: string;
  project?: string;
  query: string;
  terms: string[];
  relevantIds: number[];
};
type Note = {
  uuid: string;
  shortId: number | null;
  userId: string;
  title: string | null;
  summary: string | null;
  content: string | null;
  updatedAt: string | null;
  project: string | null;
};
type Hit = { id: number | null; title: string | null; excerpt: string; score?: number };
type SearchResult = { hits: Hit[]; elapsedMs: number; error?: string };
type Engine = "sql" | "fts5" | "fts5-rerank" | "meili";

const root = import.meta.dir;
const [snapshotPath, queriesPath, outputDirectory] = process.argv.slice(2);
const localDir = outputDirectory ?? join(root, ".local");
if (!snapshotPath || !queriesPath) {
  console.error("Usage: bun benchmarks/search/run.ts SNAPSHOT_DB QUERIES_JSON [OUTPUT_DIRECTORY]");
  process.exit(2);
}

const fixture = JSON.parse(await readFile(queriesPath, "utf8")) as { queries: Query[] };
if (!Array.isArray(fixture.queries) || fixture.queries.length === 0) {
  throw new Error("Query fixture must contain a nonempty queries array");
}
for (const query of fixture.queries) {
  if (
    !query.id ||
    !["human", "agent"].includes(query.audience) ||
    !["lexical", "broad", "cross-language", "observed-agent", "challenge", "synthetic-input"].includes(query.group) ||
    (query.project !== undefined && !query.project.trim()) ||
    !query.query.trim() ||
    !Array.isArray(query.terms) ||
    query.terms.length === 0 ||
    query.terms.some((term) => !term.trim()) ||
    !Array.isArray(query.relevantIds)
  ) {
    throw new Error(`Invalid query fixture entry: ${JSON.stringify(query)}`);
  }
}

const source = new Database(snapshotPath, { readonly: true });
const notes = source
  .query(
    `SELECT notes.id AS uuid, short_id AS shortId, notes.user_id AS userId,
            notes.title, notes.summary, notes.content, notes.updated_at AS updatedAt,
            projects.name AS project
     FROM notes LEFT JOIN projects ON projects.id = notes.project_id
     WHERE notes.deleted_at IS NULL`,
  )
  .all() as Note[];
if (notes.length === 0) throw new Error("Snapshot contains no active notes");
const users = new Set(notes.map((note) => note.userId));
if (users.size !== 1) throw new Error("Snapshot must contain active notes for exactly one user");
const userId = notes[0]!.userId;
const byUuid = new Map(notes.map((note) => [note.uuid, note]));
const byShortId = new Map(notes.map((note) => [note.shortId, note]));
for (const query of fixture.queries) {
  for (const id of query.relevantIds) {
    if (!byShortId.has(id)) throw new Error(`${query.id}: relevant ID ${id} is absent from snapshot`);
  }
}

const fts = new Database(":memory:");
fts.exec(
  "CREATE VIRTUAL TABLE notes_fts USING fts5(uuid UNINDEXED, project UNINDEXED, title, summary, content, tokenize='trigram')",
);
const insertFts = fts.query(
  "INSERT INTO notes_fts(uuid, project, title, summary, content) VALUES (?, ?, ?, ?, ?)",
);
const insertAll = fts.transaction((documents: Note[]) => {
  for (const note of documents) {
    insertFts.run(note.uuid, note.project, note.title, note.summary, note.content);
  }
});
insertAll(notes);

function excerpt(note: Note, terms: string[]): string {
  const pattern = new RegExp(
    terms.map((term) => term.replaceAll(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|"),
    "gi",
  );
  for (const field of [note.title, note.summary, note.content]) {
    if (!field) continue;
    const lower = field.toLocaleLowerCase();
    const offset = terms
      .map((term) => lower.indexOf(term.toLocaleLowerCase()))
      .filter((index) => index >= 0)
      .sort((a, b) => a - b)[0];
    if (offset === undefined) continue;
    const start = Math.max(0, offset - 45);
    const end = Math.min(field.length, offset + 100);
    const passage = field.slice(start, end).replaceAll(/\s+/g, " ");
    return `${start ? "…" : ""}${passage.replace(pattern, "[$&]")}${end < field.length ? "…" : ""}`;
  }
  return (note.summary ?? note.title ?? "").slice(0, 140);
}

function sqlSearch(query: Query): SearchResult {
  const parts = query.terms.map(
    () =>
      "CASE WHEN instr(lower(coalesce(title,'')),lower(?))>0 THEN 3 " +
      "WHEN instr(lower(coalesce(summary,'')),lower(?))>0 THEN 2 " +
      "WHEN instr(lower(coalesce(content,'')),lower(?))>0 THEN 1 ELSE 0 END",
  );
  const sql = `WITH scored AS (
    SELECT notes.id AS uuid, short_id AS shortId, title, summary, content,
           updated_at AS updatedAt, (${parts.join(" + ")}) AS score
    FROM notes LEFT JOIN projects ON projects.id = notes.project_id
    WHERE notes.user_id = ? AND notes.deleted_at IS NULL
      ${query.project ? "AND projects.name = ?" : ""}
  ) SELECT * FROM scored WHERE score > 0
    ORDER BY score DESC, updatedAt DESC, shortId DESC LIMIT 10`;
  const args = [...query.terms.flatMap((term) => [term, term, term]), userId, ...(query.project ? [query.project] : [])];
  const started = performance.now();
  const rows = source.query(sql).all(...args) as (Note & { score: number })[];
  return {
    hits: rows.map((row) => ({
      id: row.shortId,
      title: row.title,
      excerpt: excerpt(row, query.terms),
      score: row.score,
    })),
    elapsedMs: performance.now() - started,
  };
}

function ftsSearch(query: Query): SearchResult {
  const expression = query.terms
    .map((term) => `"${term.replaceAll('"', '""')}"`)
    .join(" OR ");
  const started = performance.now();
  try {
    const rows = fts
      .query(
        `SELECT uuid, bm25(notes_fts, 0, 0, 3, 2, 1) AS score,
                snippet(notes_fts, -1, '[', ']', '…', 96) AS excerpt
         FROM notes_fts WHERE notes_fts MATCH ?
           ${query.project ? "AND project = ?" : ""}
         ORDER BY score LIMIT 10`,
      )
      .all(expression, ...(query.project ? [query.project] : [])) as { uuid: string; score: number; excerpt: string }[];
    return {
      hits: rows.map((row) => {
        const note = byUuid.get(row.uuid)!;
        return {
          id: note.shortId,
          title: note.title,
          excerpt: row.excerpt,
          score: row.score,
        };
      }),
      elapsedMs: performance.now() - started,
    };
  } catch (error) {
    return { hits: [], elapsedMs: performance.now() - started, error: String(error) };
  }
}

function ftsRerankSearch(query: Query): SearchResult {
  const started = performance.now();
  const indexedTerms = query.terms.filter((term) => [...term].length >= 3);
  const shortTerms = query.terms.filter((term) => [...term].length < 3);
  const candidates = new Map<string, { note: Note; bm25: number; rowid?: number }>();
  let expression: string | undefined;
  try {
    if (indexedTerms.length) {
      expression = indexedTerms
        .map((term) => `"${term.replaceAll('"', '""')}"`)
        .join(" OR ");
      const rows = fts
        .query(
          `SELECT rowid, uuid, bm25(notes_fts, 0, 0, 3, 2, 1) AS bm25
           FROM notes_fts WHERE notes_fts MATCH ?`,
        )
        .all(expression) as { rowid: number; uuid: string; bm25: number }[];
      for (const row of rows) {
        candidates.set(row.uuid, {
          note: byUuid.get(row.uuid)!,
          bm25: row.bm25,
          rowid: row.rowid,
        });
      }
    }
    if (shortTerms.length) {
      for (const note of notes) {
        if (query.project && note.project !== query.project) continue;
        const fields = [note.title, note.summary, note.content].map((field) => (field ?? "").toLowerCase());
        if (shortTerms.some((term) => fields.some((field) => field.includes(term.toLowerCase())))) {
          if (!candidates.has(note.uuid)) candidates.set(note.uuid, { note, bm25: 0 });
        }
      }
    }
    const ranked = [...candidates.values()]
      .filter(({ note }) => !query.project || note.project === query.project)
      .map((candidate) => {
        const { note } = candidate;
        const fields = [note.title, note.summary, note.content].map((field) => (field ?? "").toLowerCase());
        const coverage = query.terms.reduce((sum, term) => {
          const needle = term.toLowerCase();
          const field = fields.findIndex((value) => value.includes(needle));
          return sum + (field < 0 ? 0 : 3 - field);
        }, 0);
        return { ...candidate, coverage };
      })
      .filter((candidate) => candidate.coverage > 0)
      .sort(
        (a, b) =>
          b.coverage - a.coverage ||
          (b.note.updatedAt ?? "").localeCompare(a.note.updatedAt ?? "") ||
          a.bm25 - b.bm25 ||
          (b.note.shortId ?? 0) - (a.note.shortId ?? 0),
      )
      .slice(0, 10);
    const snippets = new Map<number, string>();
    const matchedRowids = ranked.flatMap(({ rowid }) => (rowid === undefined ? [] : [rowid]));
    if (expression && matchedRowids.length) {
      const placeholders = matchedRowids.map(() => "?").join(",");
      const rows = fts
        .query(
          `SELECT rowid, snippet(notes_fts, -1, '[', ']', '…', 96) AS excerpt
           FROM notes_fts WHERE notes_fts MATCH ? AND rowid IN (${placeholders})`,
        )
        .all(expression, ...matchedRowids) as { rowid: number; excerpt: string }[];
      for (const row of rows) snippets.set(row.rowid, row.excerpt);
    }
    return {
      hits: ranked.map(({ note, coverage, rowid }) => ({
        id: note.shortId,
        title: note.title,
        excerpt: rowid !== undefined && snippets.get(rowid)?.includes("[")
          ? snippets.get(rowid)!
          : excerpt(note, query.terms),
        score: coverage,
      })),
      elapsedMs: performance.now() - started,
    };
  } catch (error) {
    return { hits: [], elapsedMs: performance.now() - started, error: String(error) };
  }
}

async function freePort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Could not reserve a port");
  await new Promise<void>((resolve) => server.close(() => resolve()));
  return address.port;
}

async function directoryBytes(path: string): Promise<number> {
  let total = 0;
  for (const entry of await readdir(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    total += entry.isDirectory() ? await directoryBytes(child) : (await stat(child)).size;
  }
  return total;
}

const port = await freePort();
await mkdir(localDir, { recursive: true });
const meiliDir = await mkdtemp(join(localDir, "meili-"));
const key = crypto.randomUUID().replaceAll("-", "") + crypto.randomUUID().replaceAll("-", "");
const base = `http://127.0.0.1:${port}`;
const child = spawn(
  "meilisearch",
  ["--http-addr", `127.0.0.1:${port}`, "--db-path", meiliDir, "--no-analytics"],
  {
    stdio: "ignore",
    env: { ...process.env, MEILI_ENV: "production", MEILI_MASTER_KEY: key, MEILI_LOG_LEVEL: "WARN" },
  },
);

async function request(path: string, method = "GET", body?: unknown): Promise<any> {
  const response = await fetch(`${base}${path}`, {
    method,
    headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) throw new Error(`${method} ${path}: ${response.status} ${await response.text()}`);
  return response.json();
}

async function waitTask(uid: number): Promise<void> {
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    const task = await request(`/tasks/${uid}`);
    if (task.status === "succeeded") return;
    if (task.status === "failed" || task.status === "canceled") {
      throw new Error(`Meili task ${uid}: ${JSON.stringify(task.error)}`);
    }
    await Bun.sleep(100);
  }
  throw new Error(`Meili task ${uid} timed out`);
}

async function meiliSearch(query: Query): Promise<SearchResult> {
  const started = performance.now();
  const response = await request("/indexes/flicknote_notes/search", "POST", {
    q: query.query,
    ...(query.project ? { filter: `project = ${JSON.stringify(query.project)}` } : {}),
    limit: 10,
    attributesToRetrieve: ["uuid"],
    attributesToCrop: ["title:24", "summary:24", "content:24"],
    attributesToHighlight: ["title", "summary", "content"],
    highlightPreTag: "[",
    highlightPostTag: "]",
  });
  return {
    hits: response.hits.map((row: any) => {
      const note = byUuid.get(row.uuid)!;
      const formatted = row._formatted ?? {};
      const highlighted = [formatted.title, formatted.summary, formatted.content].find(
        (value) => typeof value === "string" && value.includes("["),
      );
      return {
        id: note.shortId,
        title: note.title,
        excerpt: highlighted ?? excerpt(note, query.terms),
      };
    }),
    elapsedMs: performance.now() - started,
  };
}

const results: { query: Query; engines: Record<Engine, SearchResult> }[] = [];
let meiliBytes = 0;
try {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    try {
      await request("/health");
      break;
    } catch {
      if (child.exitCode !== null) throw new Error(`Meili exited with ${child.exitCode}`);
      await Bun.sleep(100);
    }
  }
  await waitTask((await request("/indexes", "POST", { uid: "flicknote_notes", primaryKey: "uuid" })).taskUid);
  await waitTask(
    (await request("/indexes/flicknote_notes/settings", "PATCH", {
      searchableAttributes: ["title", "summary", "content"],
      rankingRules: ["words", "typo", "proximity", "attributeRank", "sort", "exactness"],
      filterableAttributes: ["project"],
      typoTolerance: { disableOnNumbers: true },
    })).taskUid,
  );
  await waitTask(
    (await request(
      "/indexes/flicknote_notes/documents",
      "POST",
      notes.map((note) => ({
        uuid: note.uuid,
        title: note.title,
        summary: note.summary,
        content: note.content,
        project: note.project,
      })),
    )).taskUid,
  );
  meiliBytes = await directoryBytes(meiliDir);

  for (const query of fixture.queries) {
    results.push({
      query,
      engines: {
        sql: sqlSearch(query),
        fts5: ftsSearch(query),
        "fts5-rerank": ftsRerankSearch(query),
        meili: await meiliSearch(query),
      },
    });
  }
} finally {
  if (child.exitCode === null && child.signalCode === null) {
    child.kill("SIGTERM");
    await new Promise<void>((resolve) => child.once("exit", () => resolve()));
  }
  await rm(meiliDir, { recursive: true, force: true });
  source.close();
  fts.close();
}

function success(result: SearchResult, query: Query): boolean {
  if (query.relevantIds.length === 0) return false;
  const cutoff = query.audience === "human" ? 5 : 10;
  return result.hits.slice(0, cutoff).some((hit) => hit.id !== null && query.relevantIds.includes(hit.id));
}

const aggregate = Object.fromEntries(
  (["lexical", "broad", "cross-language", "observed-agent", "challenge", "synthetic-input"] as QueryGroup[]).map((group) => [
    group,
    Object.fromEntries(
      (["human", "agent"] as Audience[]).map((audience) => [
        audience,
        Object.fromEntries(
          (["sql", "fts5", "fts5-rerank", "meili"] as Engine[]).map((engine) => {
            const judged = results.filter(
              (row) => row.query.group === group && row.query.audience === audience && row.query.relevantIds.length,
            );
            return [engine, { judged: judged.length, misses: judged.filter((row) => !success(row.engines[engine], row.query)).length }];
          }),
        ),
      ]),
    ),
  ]),
);

await writeFile(join(localDir, "results.json"), JSON.stringify({ noteCount: notes.length, meiliBytes, aggregate, results }, null, 2));
const lines = [
  "# Search benchmark results",
  "",
  `Active notes: ${notes.length}; temporary Meili index: ${(meiliBytes / 1048576).toFixed(1)} MiB`,
  "",
  "| Group | Audience | Engine | Judged | Misses | Miss rate |",
  "| --- | --- | --- | ---: | ---: | ---: |",
];
for (const group of ["lexical", "broad", "cross-language", "observed-agent", "challenge", "synthetic-input"] as QueryGroup[]) {
  for (const audience of ["human", "agent"] as Audience[]) {
    for (const engine of ["sql", "fts5", "fts5-rerank", "meili"] as Engine[]) {
      const row = (aggregate as any)[group][audience][engine];
      lines.push(`| ${group} | ${audience} | ${engine} | ${row.judged} | ${row.misses} | ${row.judged ? `${(100 * row.misses / row.judged).toFixed(1)}%` : "n/a"} |`);
    }
  }
}
for (const row of results) {
  lines.push(
    "",
    `## ${row.query.id} (${row.query.audience}, ${row.query.group})`,
    "",
    ...(row.query.task ? [`Task: ${row.query.task}`] : []),
    ...(row.query.project ? [`Project: ${row.query.project}`] : []),
    `Query: ${row.query.query}`,
    `Relevant IDs: ${row.query.relevantIds.join(", ") || "unjudged"}`,
  );
  for (const engine of ["sql", "fts5", "fts5-rerank", "meili"] as Engine[]) {
    const result = row.engines[engine];
    lines.push("", `### ${engine} — ${result.elapsedMs.toFixed(1)} ms`, "");
    if (result.error) lines.push(`Error: ${result.error}`, "");
    for (const [index, hit] of result.hits.entries()) {
      const marker = hit.id !== null && row.query.relevantIds.includes(hit.id) ? " ✓" : "";
      lines.push(`${index + 1}. ${hit.id ?? "-"}${marker} ${hit.title ?? "(untitled)"} — ${hit.excerpt.replaceAll(/\s+/g, " ")}`);
    }
  }
}
await writeFile(join(localDir, "results.md"), `${lines.join("\n")}\n`);
console.log(JSON.stringify({ noteCount: notes.length, meiliMiB: +(meiliBytes / 1048576).toFixed(1), aggregate }, null, 2));
