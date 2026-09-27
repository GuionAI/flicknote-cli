# Note search benchmark

This page records the historical Slice AA experiments. Slice B made the FTS5
path the production lexical `find` implementation, removed the diagnostic
socket and Meilisearch runtime, and changed `find` results to dedicated
`SearchHit` objects with segmented snippets. The benchmark fixture and numbers
below remain useful for comparison, but the statements about production `find`
and the diagnostic socket describe the earlier spike.

## PowerSync + better-trigram FTS5 spike (Slice AA)

The Rust spike keeps production `find` unchanged. It statically links the
vendored tokenizer, registers it with SQLite before the PowerSync pool opens,
and installs a local FTS5 table plus triggers on the pinned PowerSync core's
`ps_data__notes` backing table. The triggers, FTS table, and source rows share
one SQLite transaction. Tests exercise independent reader leases, reopen,
local view writes, direct backing-table writes, an actual mocked PowerSync
download `PUT` and `REMOVE`, archive filtering, and rebuild from source rows.
No watcher, index directory, generation swap, or checkpoint is involved.

Run the isolated benchmark with a previously created, private PowerSync backup
and an existing fixture. `NEW_DB` must not exist; the runner copies the backup
and mutates only that new file. Keep all paths under `.scratch/` or `.local/`:

```sh
cargo run --release -p flicknote-sync --example fts_spike_bench -- \
  .scratch/private/flicknote.db \
  .scratch/private/coverage-input-queries.json \
  .scratch/private/fts-spike.db \
  .scratch/private/fts-spike-results.json
```

On the 2026-09-27 isolated 2,445-active-note backup used by the Tantivy
coverage runner, the FTS table added 31.9 MiB to the 163.6 MiB SQLite file.
Initial build was 2.05–12.34 s across several single runs; the spread reflects
local I/O variability, not a controlled latency distribution. Reopen to a
queryable FTS table took 9–10 ms. One synthetic physical-row insert/update
and visibility check took 6.2–7.4 ms; archive and disappearance check took
2.3–3.3 ms. Transaction batches of 100 and 1,000 synthetic physical rows
took 14–20 and 216–295 ms. These timings exclude daemon RPC and the full
note projection.

The original 48-query fixture had no FTS errors. The primary judged groups
matched saved Tantivy coverage targets: Human Miss@5 0/13 and Agent Miss@10
1/21. The 54 shortened-input frames also had no errors. FTS Miss@5 was 2/28
for incomplete inputs of at least two characters, 5/13 for single characters,
and 0/13 for complete controls. Saved Tantivy coverage results were 0/28,
2/13, and 0/13 respectively. In the final release run, FTS query-only medians
were 12.5, 6.8, and 2.5 ms for those groups; medians including set-based
snippets were 15.8, 9.9, and 6.5 ms. Across the original fixture, the FTS
query-only and query-plus-snippet medians were 6.9 and 12.7 ms. The runner
times query-only before query-plus-snippet, so the latter may benefit from
warm caches. These are single samples with
different index implementations and query policies; the saved Tantivy input
runner's overall median including snippets was 10.6 ms. Raw results and note
IDs stay in private `.scratch/` files.

### AA.1: final Latin term as an FTS5 prefix

The spike now emits `"term"*` for the final term when it contains Latin ASCII
letters, while earlier terms and CJK-containing fragments use quoted exact
matches. The terms-only benchmark input does not retain trailing whitespace,
so this policy also treats a complete final Latin word as a prefix. It changes
only the experimental query builder. Coverage scoring remains literal field
coverage at 3/2/1, and snippets use the same FTS match expression.

On the same 54-frame fixture and isolated 2,445-note backup, AA.1 had no query
errors. Incomplete two-plus-character Miss@5 improved from 2/28 to 0/28;
single-character Miss@5 remained 5/13, versus saved Tantivy's 2/13. Complete
controls remained 0/13. The original 48-query fixture also had no errors or
target-rank changes: primary Human Miss@5 remained 0/13 and Agent Miss@10
remained 1/21.

| Input group | AA exact Miss@5 | AA.1 prefix Miss@5 | AA.1 query-only median | AA.1 query + snippet median | Hits at 100-result cap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Incomplete, 2+ characters (28) | 2 | 0 | 28.6 ms | 36.4 ms | 21/28 |
| Single character (13) | 5 | 5 | 79.6 ms | 319.5 ms | 13/13 |
| Complete controls (13) | 0 | 0 | 2.7 ms | 6.6 ms | 5/13 |

The previous exact-query medians for these groups were 12.5/15.8, 6.8/9.9,
and 2.5/6.5 ms (query-only/query plus snippet). On the original fixture,
AA.1 query-only and query-plus-snippet medians were 6.6 and 15.5 ms, against
AA's 6.9 and 12.7 ms. Each number is one local release-build sample. The
runner caps output at 100 hits and runs query-only before the snippet query;
the cap is not the full candidate count, and the second call may benefit from
cache warmth. The one-character prefix broadens matching enough that the
query and snippet work are too slow for an interactive default on this sample.

### AA.2: daemon production-shape validation

The experimental `FtsSearchService` now leases a PowerSync reader and returns
an independent `SearchHit` with short ID, title, summary, timestamps, project
ID, and segmented snippet. One indexed SQL query retrieves and ranks the
candidates; one set-based FTS query adds snippets for **every returned hit**.
There is no per-hit SQLite hydration. The serialized hit excludes UUID,
score, body, raw source, and FTS markup. A single Latin ASCII letter by itself
returns an empty list without issuing a prefix MATCH. The final Latin term
uses prefix MATCH only at two or more characters; CJK terms use indexed exact
MATCH. Production `NoteFind` routing and its public IPC/MCP DTO remain unchanged.

For measurement only, setting `FLICKNOTE_FTS_SPIKE_SOCKET` on an isolated
foreground daemon enables a separate local socket (mode `0600`). It installs
the existing FTS trigger projection through a PowerSync writer lease, then
executes the same service through reader leases. The socket is opt-in and is
removed at shutdown. The measurements below used a private copy of the same
2,445-note PowerSync backup, synthetic local credentials, and local unreachable
network endpoints. No live daemon or data directory was touched. The current
host had no Meilisearch executable, so this daemon's ordinary production find
backend was SQLite fallback. Raw IDs and measurements stay under `.scratch/`.

The default `NoteFindInput` limit is 20. With all 20 hits eligible for
segmented snippets, the **reopened** daemon run gave these one-sample figures:

| Judged group | Miss | IPC median | IPC p95 sample | Internal median |
| --- | ---: | ---: | ---: | ---: |
| Primary Human (13), @5 | 0 | 12.1 ms | 49.5 ms | 11.6 ms |
| Primary Agent (21), @10 | 1 | 35.0 ms | 93.7 ms | 34.5 ms |
| Incomplete 2+ (28), @5 | 0 | 47.1 ms | 150.0 ms | 46.5 ms |
| Single character (13), @5 | 5 | 14.4 ms | 38.8 ms | 13.9 ms |
| Complete controls (13), @5 | 0 | 12.2 ms | 50.0 ms | 11.7 ms |

The single-character group includes CJK and multi-term inputs; a query that
consists of just one Latin letter returned zero hits in a direct probe with
about 0.2 ms IPC latency. A CJK one-character probe used indexed MATCH and
returned 20 hits. Mixed `中文` + `offi`, project-filtered queries, punctuation,
and snippet spans passed integration tests. The 100-result diagnostic run kept
the same primary and shortened-input recall, but all-hit snippets increased
reopened IPC medians to 44.1 ms Human, 67.3 ms Agent, and 87.3 ms for
incomplete 2+ inputs. These 54 frames share targets and are not independent
judgments; p95 values are order statistics from one warm fixture pass.

In a reopened daemon, a `中` probe at the 100-result cap took 0.52 ms for
indexed candidate count, 9.45 ms for query plus 3/2/1 rerank, and 23.18 ms
for query plus all-hit snippets. `offi` took 0.44, 7.21, and 30.69 ms;
mixed `中文` + `offi` took 0.58, 16.82, and 44.67 ms. These are separate
diagnostic calls on one reader lease, not additive stages. Serialization of
the 20-hit vector had a median below 0.12 ms across the judged groups; the
outer diagnostic envelope is included in IPC timing.

The process reached its ordinary IPC socket in 51 ms without FTS. On a fresh
copy missing FTS, it reached the indexed search socket in 2.28 s; reopening
that indexed copy took 105 ms. Daemon RSS was 17.7 MiB at baseline, 92.3 MiB
shortly after initial build, 35.2 MiB after indexed reopen, and 52.0 MiB
after the query burst. These are process RSS samples, not peak or incremental
ownership accounting. The initial build is not a regular reopen; rare rebuild
was not timed as ordinary startup.

| Decision dimension | FTS5 + better-trigram | Tantivy coverage A | Meilisearch 1.14 |
| --- | --- | --- | --- |
| Primary relevance | Human 0/13, Agent 1/21 | Human 0/13, Agent 1/21 | Human 1/13, Agent 0/21 |
| 2+ prefix Miss@5 | 0/28 | 0/28 | 4/28 |
| CJK single character | Indexed token; daemon probe returned hits | Unsupported in saved candidate | Engine-dependent |
| Ready/startup | 2.28 s first build; 105 ms reopen to search socket | Standalone index opens; daemon startup unmeasured | Isolated child Ready in 7.1 s; daemon projection unmeasured |
| Update consistency | Same SQLite transaction, physical-table triggers | Separate writer/commit/reload lifecycle proposed | Async projection task |
| Index/disk overhead | +31.9 MiB in SQLite | 36.2 MiB merged control; 45.4 MiB native-prefix variant | 209.4 MiB temporary index directory |
| Daemon RSS | 35.2 MiB reopened, 52.0 MiB after burst; baseline 17.7 MiB | Unmeasured; 19.5/27.3 MiB standalone reader only | Unmeasured on this host |
| End-to-end find | Reopened 20-hit IPC medians: Human 12.1 ms, Agent 35.0 ms, 2+ 47.1 ms | Unmeasured through daemon; saved direct input median 10.6 ms | Unmeasured through daemon |
| Maintenance and lifecycle | One FTS table and physical-table triggers | External index, writer, commit/reload and recovery proposed | Child process and async projection |
| Production LOC estimate | 250–500 physical lines | 600–950 physical lines | Current module 623 physical lines, plus full-result work |

FTS has a complete no-N+1 result path and simple transactional maintenance,
but the all-hit snippet cost and absent daemon-level Tantivy/Meili comparison
do not yet support rewriting Slice B as an FTS cutover. Keep B's backend
decision open until equivalent 20-hit daemon measurements exist; the AA.2 path
is ready to be compared without adding a scan or another index.

An isolated Meilisearch 1.14.0 child on the same backup reached an indexed
Ready state 7.1 s after launch (6.98 s from index creation through document
task completion) and occupied 209.4 MiB on disk. Two earlier runs of the same
backup took 6.0 and 9.1 s for index creation through document completion.
The benchmark now records `meiliBuildMs` and `meiliReadyMs`. Meilisearch 1.14
requires ranking rule `attribute` and does not accept `disableOnNumbers`;
the daemon and benchmark settings now use supported values. This is a temporary
child baseline, not the production daemon's projection-to-Ready timing.

| Engine | Primary Human Miss@5 | Primary Agent Miss@10 | Incomplete 2+ Miss@5 | Single-char Miss@5 |
| --- | ---: | ---: | ---: | ---: |
| SQL substring | 0/13 | 1/21 | 0/28 | 5/13 |
| Built-in FTS5 trigram BM25 | 3/13 | 1/21 | 8/28 | 5/13 |
| Built-in FTS5 + coverage and short-term scan | 0/13 | 1/21 | 0/28 | 5/13 |
| Better-trigram FTS5 + coverage, exact AA | 0/13 | 1/21 | 2/28 | 5/13 |
| Better-trigram FTS5 + coverage, prefix AA.1 | 0/13 | 1/21 | 0/28 | 5/13 |
| Meilisearch 1.14 | 1/13 | 0/21 | 4/28 | 4/13 |
| Saved Tantivy coverage | 0/13 | 1/21 | 0/28 | 2/13 |

The built-in FTS5 coverage variant scans the full snapshot for terms shorter
than three characters, so its recall and latency use a different fallback.
The Tantivy rows come from saved coverage-runner output; the other rows were
measured with the current isolated backup and fixtures. The input frames share
targets and are not independent relevance judgments.

The spike's FTS query uses OR retrieval and literal field coverage (title,
summary, content = 3/2/1), then generates snippets for the returned hits in
one set-based query. The AA/AA.1 measurements above generated snippets only
for the first ten; the AA.2 measurements cover every returned hit. FTS
`snippet()` markers are converted to text/highlight segments;
tests cover CJK, emoji, and combining text. This is a candidate experiment,
not a production snippet wire path. One-character Latin input still trails
the Tantivy coverage variant. The benchmark does not
measure daemon RSS, repeated p95 query latency, or production Meilisearch
projection-to-Ready time.

The trigger boundary is supported by the pinned PowerSync implementation and
the mocked download test. An SDK upgrade must recheck its physical table name
and local/remote write path before this FTS table becomes a production index.
The architecture checkpoint therefore finds FTS5 viable as a single-file,
trigger-maintained index. Native prefix closes the two-plus-character recall
gap, but it leaves the single-character gap and adds substantial latency on
broad prefixes. Keep the existing production backend and Tantivy candidate in
place. Reconsider a cutover only after one-character recall and interactive
latency improve, and after daemon RSS and production-path latency are measured.

This local experiment compares ranked note retrieval on the same active-note
snapshot. It is for choosing a search implementation, not a performance gate.
The engine families and ranking variants are:

- SQL substring coverage across title, summary, and content;
- SQLite FTS5 trigram matching with field-weighted BM25;
- FTS5 trigram candidate retrieval followed by the SQL field-coverage and
  recency ranking, with substring candidate fallback for terms shorter than
  three Unicode characters;
- the daemon's current Meilisearch searchable attributes and ranking rules;
- Tantivy ngram, native prefix, and indexed field-coverage variants.

The runner uses Bun's SQLite build and a temporary Meilisearch child. It never
contacts FlickNote's running daemon or Meilisearch process. Keep snapshots,
query judgments, and generated results in `.local/`, which is ignored by Git.

## Prepare a snapshot

Make a SQLite online backup of your canonical database into the benchmark-owned
directory. This is a separate data preparation step; all benchmark runs read
only the backup. For the default macOS installation:

```sh
mkdir -p benchmarks/search/.local
sqlite3 'file:'"$HOME"'/.local/share/flicknote/flicknote.db?mode=ro' \
  '.backup benchmarks/search/.local/flicknote.db'
```

Use a different source path if `XDG_DATA_HOME` is set. The snapshot contains
private note content. Do not commit, upload, or share `.local/`.

Create `.local/queries.json` using this shape:

```json
{
  "queries": [
    {
      "id": "human-example",
      "audience": "human",
      "group": "lexical",
      "task": "Find the note about the red office chair",
      "query": "red leather chair",
      "terms": ["red", "leather", "chair"],
      "relevantIds": [101]
    },
    {
      "id": "agent-example",
      "audience": "agent",
      "group": "lexical",
      "task": "Find the design review decision before changing the implementation",
      "query": "design review notes",
      "terms": ["design", "review", "notes"],
      "project": "research",
      "relevantIds": [202]
    }
  ]
}
```

`relevantIds` are numeric FlickNote IDs judged before inspecting the benchmark
results. An empty array means the query is exploratory and contributes no
success metric. `task` records the person's original intent when an agent
rewrites it as `query`. `terms` makes SQL and FTS5 token choices explicit while
Meili receives `query` exactly as written. `group: "cross-language"` separates
translation-like searches from the primary lexical comparison. `group: "broad"`
marks short, underspecified queries with many plausible competing notes.
`group: "observed-agent"` contains Agent search wording recovered from
FlickLog, with the originating session and record index kept in the private
fixture. `project` restricts all candidates to the same project when
the logged call used one. The running daemon currently routes structured
project searches to SQLite; Meili and FTS5 results for these rows are
counterfactual comparisons within the same candidate set.
`group: "challenge"` contains constructed fragment and typo probes. Some
fragment probes were selected after inspecting SQL ranks, so their aggregate
miss count is diagnostic and must not be treated as an estimate of real-query
failure rates. Relevance labels identify intended notes, not every note that
could reasonably satisfy a short query.

Run from the repository root:

```sh
bun benchmarks/search/run.ts \
  benchmarks/search/.local/flicknote.db \
  benchmarks/search/.local/queries.json
```

The runner writes `.local/results.json` and `.local/results.md`. Its primary
metric is whether at least one judged-relevant note appears in the first five
human results or first ten agent results. The Markdown report also shows
ordered hits and short match excerpts for inspection. Read the lexical, broad,
cross-language, observed-agent, and challenge rows separately. This first version does not
claim parity between substring, FTS5 trigram, and Meili query semantics.
The `fts5-rerank` candidate deliberately uses the SQL baseline's ranking rule;
it tests whether FTS5 can provide the same ordering with indexed candidate
retrieval and snippets, not whether BM25 is a better relevance rule. Recorded
query times are single samples, not a latency benchmark. The Meili sample is a
direct request to the temporary index; production `fn find` additionally
hydrates every hit through the daemon's canonical SQLite store and formats the
CLI table. FTS5 snippets use a 96-token window because this index's trigram
tokens yield much shorter excerpts than ordinary word tokens at the same
window size.

Record concrete issues and decisions in [ISSUES.md](ISSUES.md). Keep raw note
text and private query judgments out of that tracked file.

## Ngram continuity and unfinished-input audit

`python3 benchmarks/search/audit-ngram.py` reads the snapshot and saved Tantivy
results, then writes private audit results and synthetic input fixtures to
`.local/`. It compares the original ranking with literal-term filtering and
field-coverage reranking (title/summary/content weights 3/2/1, original rank
breaks ties). It uses cached lowercase text: its timer excludes document I/O,
lowercasing, snippets, and RPC. These are post-hoc diagnostics on existing labels.

The fixture truncates only the last term of 13 labeled Human queries at 1, 2,
3, penultimate, and complete character lengths, removing duplicate lengths.
There are 54 frames: 13 one-character, 28 incomplete two-plus-character, and
13 complete controls. Only 4 of the incomplete frames contain CJK characters.
Frames share queries and inherited targets; they are not independent judgments
of what every short query should return. The original ngram audit tests substring-style input; the later native-prefix
variant uses the same frames to exercise word-boundary prefix APIs.

Run the same isolated Tantivy binary with `.local/input-queries.json` and
output `.local/tantivy-input-results.json`; repeat with
`.local/input-queries-100.json` and `.local/tantivy-input-100-results.json`.
Then rerun the audit script. For example, after building the release binary:

```sh
benchmarks/search/tantivy/target/release/flicknote-search-tantivy-bench \
  benchmarks/search/.local/flicknote.db \
  benchmarks/search/.local/input-queries.json \
  benchmarks/search/.local/tantivy-input-results.json
```

The fixture's optional `candidateLimit` accepts 10–1000, default 10 in the original modes. All retrieved
candidate documents are read, but snippets are generated only for the first ten
in the original ranking. The 100-candidate run therefore does not measure final
reranked snippets or production end-to-end latency. Baseline reports stay intact.

The [search options decision memo](search-options-decision.html) summarizes the
current evidence and explains when another engine experiment would be useful.

## Tantivy dependency and measurement versions

The runner is pinned to upstream commit
`5ca39332002c2c87fb5d2abc707cf527b3319d42` (development version 0.27.0),
which includes [Tantivy #3034](https://github.com/quickwit-oss/tantivy/pull/3034).
This removes vulnerable `lru` 0.16.x (RUSTSEC-2026-0253); the lockfile resolves
`lru` 0.18.5. Replace the git pin when a published Tantivy release includes it.
Existing relevance/performance figures in this report were measured with
Tantivy 0.26.2, before this dependency fix. They are historical measurements,
not fresh measurements of the pinned development version.

## Tantivy 2/3-gram experiment

The standalone Rust runner builds a temporary Tantivy index from the same
snapshot. It does not change the daemon or the main Rust workspace:

```sh
cargo run --release --manifest-path benchmarks/search/tantivy/Cargo.toml -- \
  benchmarks/search/.local/flicknote.db \
  benchmarks/search/.local/queries.json \
  benchmarks/search/.local/tantivy-results.json
```

It indexes title, summary, and content with Tantivy's
`NgramTokenizer(2, 3, false)` and stores those original fields for potential
snippets. It reports index bytes, build time, one direct-search and one
search-with-snippet timing sample per query, ordered short IDs, HTML-highlighted
snippets, the first judged target rank, and the standalone reader process's
RSS after opening and after searching. Query terms
are ORed; all grams within a term must occur in one field, weighted
title/summary/content 3/2/1. This is a candidate-ranking experiment, not
the SQL ranking rule or Meili's language-aware tokenization. The gram
conjunction does not require adjacent positions, and it has no typo correction.
The snippet code prefers an excerpt containing a full query term, then falls
back to Tantivy's highlighted gram excerpt. The RSS sample comes from a fresh
read-only child process using `ps -o rss=` on Unix, so it excludes the index
writer; on other platforms the field is null. It is not the
incremental memory that embedding Tantivy would add to the FlickNote daemon.
Neither timing includes daemon RPC or complete `NoteListItem` projection;
do not read them as end-to-end `fn find` latency. The temporary index is
deleted after the run; only the private JSON result remains in `.local/`.

### Native prefix variant

Set the fixture's `mode` to `hybridPrefix` (default: `ngram`). The audit script
generates `.local/native-prefix-queries.json`; run the same binary with that
fixture and `.local/tantivy-native-prefix-results.json` as output, then rerun
`audit-ngram.py`. No production daemon or live database is involved.

This variant adds index-only word fields using `SimpleTokenizer + LowerCaser`.
The final non-CJK term is analyzed with the same tokenizer: complete tokens
use `TermQuery`, and the final token uses the official
`FuzzyTermQuery::new_prefix(term, 0, true)`. Distance zero disables typos.
The final term is required; earlier terms contribute optional relevance.
Native prefix automata use constant scores, so the existing ngram query also
contributes BM25 scores. This deliberately differs from the baseline's OR policy.
CJK final terms retain the ngram route (including its unsupported single-character
case). No dictionary is introduced. `SimpleTokenizer` does not split script
transitions such as Chinese directly adjacent to Latin text, or perform stemming.
The fixture supplies terms; trailing-space/completed-token UI behavior is not modeled.

Prefix automata do not expose expanded terms to `SnippetGenerator::create`.
For the original top ten hits, the variant obtains matching words from each
stored field and supplies them to the public `SnippetGenerator::new` API.
This highlights whole matching words, with no custom HTML generation. CJK uses
the baseline snippets. Original source text is stored only once.

The runner now waits for background merges before measuring disk size and
opening the read process. Earlier 55.3 MiB measurements sampled the directory
before that wait and may include merge intermediates; do not interpret the
difference as a tokenizer compression improvement. On the same 54-frame fixture,
the merged control measured 36.2 MiB and the hybrid 45.4 MiB. Read-process RSS
was 19.5 and 27.3 MiB respectively, not daemon increments or peaks.

For 28 incomplete two-plus-character frames, raw Miss@5 was 7 for the merged
ngram control and 6 for the hybrid; post-hoc field-coverage reranking of the
hybrid's top ten reduced it to 4. For 13 complete controls the hybrid was 3
raw and 1 reranked, so it is not an across-the-board relevance improvement.
Single-sample medians for search + original top-ten snippets were 10.32 ms
(control) and 1.17 ms (hybrid); search/ID alone was 0.30 vs 0.64 ms. Snippet
implementations differ, and neither includes RPC or post-rerank snippets.

An end-to-end temporary-database test covers nine cases: word boundaries,
case folding, one-letter input, requiring the last term, project filtering,
zero typo distance, CJK fragments, unsupported single CJK characters, and
punctuation. Every returned test hit must also have a highlighted snippet.

Official references:
- https://docs.rs/tantivy/0.26.2/tantivy/query/struct.FuzzyTermQuery.html
- https://docs.rs/tantivy/0.26.2/tantivy/snippet/struct.SnippetGenerator.html
- https://sqlite.org/fts5.html#the_trigram_tokenizer (fixed trigram, not configurable 2/3-gram)


## Four-engine input comparison (2026-09-27)

The Bun runner accepts an optional output directory and `synthetic-input`
fixtures. This preserves the original 48-query results:

```sh
bun benchmarks/search/run.ts \
  benchmarks/search/.local/flicknote.db \
  benchmarks/search/.local/native-prefix-queries.json \
  benchmarks/search/.local/prefix-comparison
```

Compare these outputs with `tantivy-native-prefix-results.json` and the audit's
post-hoc coverage ranks. The snapshot, 54 input frames, and labels are identical;
query policies, snippet implementations, and reranking candidate counts differ.
All 54 SQL/FTS5/Meili queries completed without query errors. For 28 incomplete
frames (length >=2), Miss@5 was SQL 0, FTS5-rerank 0, Meili 6, hybrid Tantivy 6
(4 after coverage reranking). Medians including each runner's snippets were
138.09 / 39.08 / 11.24 / 1.17 ms respectively; Tantivy excludes post-hoc rerank.
For 13 complete controls the misses were 0 / 0 / 1 / 3 (Tantivy reranked: 1).
For 13 one-character incomplete frames they were 5 / 5 / 4 / 7 (raw Tantivy).
Only four of the 28 incomplete frames contain CJK. These inherited labels do
not establish which results a human would prefer at every shortened prefix.

Tantivy production estimate, including native prefix: 600–950 physical lines,
excluding tests and common CLI code. Breakdown: schema/query/snippets 300–400;
watcher/upsert/delete/commit/reload/rebuild/state recovery 200–350; daemon API
and full result projection 100–200. This replaces the earlier 450–900 estimate.
The 554-line experiment includes benchmark orchestration and cannot simply be
added to the estimate. Meili's current module is 623 physical production lines,
not a completed implementation of the proposed full-projection optimization.
SQL 100–250 and FTS5 250–500 remain planning estimates; FTS5 excludes an extra
word-tokenized prefix index. All physical counts include blank lines/comments.
Production memory increments, writer peaks, and full RPC/projection timings
require integration and remain unmeasured.

## Indexed coverage variant

Set `mode: "coverage"`; `candidateLimit` defaults to 100 for this mode (10 for
older modes), with an accepted range of 10–1000. Run the same Rust binary with
`coverage-queries.json` or `coverage-input-queries.json` and a separate output
path, then use `summarize-coverage.py` for aggregate reporting.

The query uses native `ConstScoreQuery` (3/2/1 field weights),
`DisjunctionMaxQuery` (best field per term), and Boolean OR (sum across terms).
Two-plus-character terms retain fragment semantics. One Latin character uses
the native word-prefix route; a standalone CJK character is still unsupported.
The final term is no longer mandatory in this variant, matching the broad
candidate policy of the SQL/FTS5 coverage baseline. `hybridPrefix` remains a
separate strict word-prefix experiment.

`TopDocs::tweak_score` uses coverage, UTC update milliseconds, then short ID,
with fast fields for the tie-breakers. Recency is applied BEFORE truncation:
without it, two short-query targets were lost among equal-coverage candidates.
Only the selected candidates have their stored bodies read and lowercased.
Literal coverage is then verified and reranked by score/date/ID, returning ten
results with freshly generated snippets. Timings include this complete local
path; they exclude daemon RPC and production result projection.

Ngram conjunctions remain an approximation to continuous substring matching.
A 100-candidate cap is deliberate, not a proof that every relevant document
survives. The collector visits matching postings; it is not constant-time.
No full-corpus text scan or SQL fallback is used in this query mode.

The growth run uses a benchmark-owned `coverage-growth.db` with four copies of
the 2,445 active notes and the same projects. Each copy receives distinct
numeric IDs; all copies of a labeled target count as relevant. Original text,
update dates, and field lengths are unchanged. This measures extra postings
and index size, not realistic new-topic diversity; duplicates can crowd out
other results. It must not be used as evidence of relevance at real 10k scale.
Fixtures and raw output remain private in `.local/`.

Latest coverage results: original Human Miss@5 0/13, Agent Miss@10 1/21;
incomplete input Miss@5 0/28 and complete controls 0/13. Incomplete-input
median including final snippets: 11.37 ms at 2,445 notes and
13.33 ms at 9,780 duplicated notes. Single-character misses were
2/13 and 5/13 respectively; duplicate crowding is visible here. All returned
snippets in these labeled groups were non-null. The current experiment is
715 production-path/script lines and 176 integration-test lines, counted
physically including whitespace/comments; this is not the production daemon LOC.
