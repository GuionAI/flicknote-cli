# Note search benchmark

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
snippets in these labeled groups were non-null. Before Slice A extraction, the
experiment was 715 production-path/script lines and 176 integration-test lines,
counted physically including whitespace/comments; this is not the production
daemon LOC.

## Slice A: embedded projection validation (2026-09-27)

The daemon now owns a disposable Tantivy projection. This measurement used an
isolated SQLite backup of the then-current KB (2,463 active notes, 16.86 MiB of
UTF-8 title, summary, and content). The release daemon ran with a synthetic
session and loopback-only service endpoints; Meilisearch was disabled. The real
KB, session, service, and installed executable were not changed. The index was
rebuilt at every daemon start, and the query code used by the benchmark and
daemon is now shared in `flicknote-tantivy`.

| Observation | Release build on isolated KB backup |
| --- | ---: |
| Cold rebuild, watcher snapshot to Ready | 12.4 s |
| Index bytes immediately at Ready | 82.9 MiB |
| Index bytes after asynchronous merge settled | 45.1–48.0 MiB across two runs |
| Daemon RSS before rebuild | 37.6 MiB |
| Peak daemon RSS during rebuild | 210.9 MiB |
| RSS at Ready with writer retained | 209.3 MiB |
| Settled RSS with writer retained | 175.0 MiB (137.5 MiB over the 37.5 MiB baseline in that run) |
| One-note update to searchable | 321 ms (commit/reload: 119 ms) |
| One-note archive to absent | 178 ms |
| 100-note update burst to searchable | 1.15 s (commit/reload: 893 ms) |
| 1,000-note update burst to searchable | 26.5 s (commit/reload: 17.0 s) |
| Corrupt prior `meta.json` then restart to Ready | 12.7 s |
| Kill during rebuild then restart to Ready | 15.7 s (concurrent build load) |
| Indexed document count after each rebuild | 2,463, matching the canonical active count |

The update measurements modified only a second disposable copy of the snapshot.
Each burst appended a distinct marker to 100 or 1,000 active titles with one
SQLite statement through PowerSync's writer; visibility was timed from the
write through the projection watcher, commit, reader reload, and marker query.
The numbers are single runs, not latency percentiles. The 1,000-note burst is
slower than a full rebuild; it is evidence to revisit batch strategy if this
write pattern becomes common. The settled disk and RSS values were measured
after Tantivy's background merge; the Ready log samples them earlier.

The private fixtures and matching 2,445-note benchmark snapshot were recovered
from the Mac checkout into ignored local scratch space. On that snapshot, the
production schema and query module matched the saved benchmark target ranks
**and complete top-10 hit lists** for all 48 coverage queries and 54 input-stage
queries. No private notes, queries, or results are committed. The tracked
coverage and prefix integration cases also passed. Pure query and
query-plus-snippet times from the earlier benchmark are separate from the
above daemon projection and update measurements. End-to-end `find` latency is
reserved for Slice B.
