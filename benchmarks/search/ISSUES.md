# Search benchmark findings and open questions

This is a local decision experiment, not a production search implementation.
Keep note text, query wording, IDs, relevance labels, and raw results in `.local/`.
The HTML report contains only aggregate observations and implementation estimates.

## Evidence boundaries

- The current fixture has 48 queries. Primary labeled groups contain 13 Human
  queries evaluated at top five and 21 Agent queries evaluated at top ten.
  Other rows include constructed challenges, cross-language probes, and an
  unjudged query. Labels identify intended targets, not every relevant note.
- Some Agent queries came from observed sessions; Human queries and input
  sequences are constructed. The fixture has influenced implementation.
  Independent tasks and human-reviewed labels are needed before treating miss
  counts as production failure rates.
- The 54 input frames derive from 13 Human queries and inherit their targets:
  28 incomplete two-plus-character frames, 13 complete controls, and 13
  one-character frames. Only four incomplete frames contain CJK.
- Timings are individual samples, not repeated latency distributions or p95.
  Runner languages, candidate policies, snippet implementations, and timing
  boundaries differ. None measures the full production CLI request.
- FTS5 is held in memory in the Bun runner. Its reported disk footprint was a
  separate local measurement. Do not interpret this as a production persistence
  or update benchmark.

## SQL, FTS5, and Meilisearch

- Weighted literal coverage with recency ties performs well on this sample.
  Raw FTS5 trigram BM25 and field-coverage reranking are separate variants;
  poor raw BM25 ranks do not establish an engine-level retrieval limitation.
- FTS5's built-in trigram cannot retrieve one- or two-character substrings
  through MATCH. The coverage variant scans for short terms. That path becomes
  more expensive with corpus size and is the main reason to investigate Tantivy.
- SQL/FTS5 literal matching misses typo probes that Meili handles. Cross-language
  semantic probes are outside the main scope and missed across the initial
  candidates; no embeddings are enabled.
- Meili direct-query timing excludes the daemon's serial canonical hydration.
  A projection-only result could remove that work, but complete list DTOs also
  need project/topic metadata and updates when those entities change. The
  report's optimized 40 ms target remains an assumption.
- Meili's temporary benchmark index differs from the live daemon projection.
  RSS snapshots are not exclusive memory, writer peaks, or embedded increments.

## Tantivy evolution

- The initial ngram experiment used all grams of each term in one field, then
  OR across terms. Official NgramTokenizer gives grams position zero; conjunction
  does not establish adjacency. A snippet can highlight grams without displaying
  a continuous full query term.
- The earlier 55.3 MiB directory sample was taken before waiting for background
  merges and may include intermediates. The runner now waits for merges; final
  segment layout can still vary between builds.
- `hybridPrefix` adds word fields and native zero-edit-distance prefix queries.
  It requires the last term, unlike the fragment OR baseline. SimpleTokenizer
  does not provide CJK dictionary segmentation or split every script transition.
- Native fuzzy-prefix queries do not enumerate expanded terms for snippet
  construction; the experiment supplies matched words to SnippetGenerator.
- `coverage` moves 3/2/1 field weights into ConstScoreQuery and uses
  DisjunctionMaxQuery for the best field per term. Multi-term OR sums coverage.
  One Latin character uses word prefix; two-plus characters use fragments.
- Update-time and ID fast fields break ties BEFORE candidate truncation. Doing
  this only after retrieval lost two short-query targets among equal scores.
- The coverage path reads at most 100 candidate bodies by default, verifies
  literal coverage, reranks, and generates snippets for the final ten. It has
  no full-corpus text scan or SQL query fallback. Matching postings are still
  visited; this is not constant-time retrieval.

## Current result and remaining work

- Coverage-mode primary misses: Human 0/13, Agent 1/21. Incomplete input misses:
  0/28; complete controls: 0/13. These match the existing SQL/FTS5 primary counts.
- Incomplete-input median including verification, rerank, and final snippets:
  11.4 ms at 2,445 notes and 13.3 ms at 9,780 duplicated notes in the latest run.
- The growth experiment duplicates each note four times with distinct IDs and
  counts all target copies as relevant. It measures extra postings and storage,
  not real new-topic diversity. Duplicate crowding affects top-k behavior.
- Standalone CJK characters remain unsupported. A bounded pool can still lose
  targets when scattered grams inflate competing scores. Input completion,
  trailing spaces, and mixed scripts need explicit product semantics.
- Still unmeasured: production daemon memory increment, writer peak, update
  visibility, recovery behavior, and full RPC/result projection latency.
- Production LOC estimates exclude tests and shared CLI code. Experimental
  scaffolding is not production LOC; the 2,000-line budget is a design constraint,
  not a measured implementation size.
