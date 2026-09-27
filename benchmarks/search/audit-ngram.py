"""Diagnose saved candidates and prepare synthetic last-term input sequences.

Reads only the benchmark snapshot. Private fixtures and reports stay in .local/.
This measures ranking changes within saved candidates, not a new engine.
"""

import json
from pathlib import Path
import sqlite3
import statistics
import time


LOCAL = Path(__file__).resolve().parent / ".local"


def read(name):
    return json.loads((LOCAL / name).read_text())


def write(name, value):
    (LOCAL / name).write_text(json.dumps(value, ensure_ascii=False, indent=2))


def diagnose(queries, results, notes):
    by_id = {query["id"]: query for query in queries}
    rows = []
    for result in results:
        query = by_id[result["id"]]
        terms = [term.lower() for term in query["terms"]]
        started = time.perf_counter()
        scored = []
        for rank, note_id in enumerate(result["hits"]):
            # Cached lowercase snapshot text: no I/O or snippet work in this timer.
            fields = notes[note_id]
            coverage = sum(
                max((3 - i for i, text in enumerate(fields) if term in text), default=0)
                for term in terms
            )
            scored.append((coverage, rank, note_id))
        filtered = [note_id for coverage, _, note_id in scored if coverage > 0]
        reranked = [
            note_id
            for coverage, _, note_id in sorted(scored, key=lambda row: (-row[0], row[1]))
            if coverage > 0
        ]
        elapsed = (time.perf_counter() - started) * 1000
        target = set(query["relevantIds"])
        rankings = {"baseline": result["hits"], "filter": filtered, "coverage": reranked}
        row = {
            "id": query["id"],
            "audience": query["audience"],
            "group": query["group"],
            "hitCount": len(scored),
            "noLiteralTerm": sum(score == 0 for score, _, _ in scored),
            "top1NoLiteralTerm": bool(scored and scored[0][0] == 0),
            "ranks": {
                mode: next((i + 1 for i, note_id in enumerate(ids) if note_id in target), None)
                for mode, ids in rankings.items()
            },
            "hasLabel": bool(target),
            "rerankCachedTextMs": elapsed,
            "engineWithSnippetMs": result["withSnippetMs"],
        }
        if "prefixLength" in query:
            row["prefixLength"] = query["prefixLength"]
            row["sourceId"] = query["sourceId"]
            row["isComplete"] = query["isComplete"]
            row["precedingTerms"] = len(terms) - 1
            row["hitsWithLastFragment"] = sum(
                any(terms[-1] in field for field in notes[note_id])
                for note_id in result["hits"]
            )
        rows.append(row)
    return rows


def summarize(rows):
    out = {
        "queries": len(rows),
        "hits": sum(row["hitCount"] for row in rows),
        "noLiteralTerm": sum(row["noLiteralTerm"] for row in rows),
        "top1NoLiteralTerm": sum(row["top1NoLiteralTerm"] for row in rows),
    }
    if rows:
        out["medianRerankCachedTextMs"] = statistics.median(row["rerankCachedTextMs"] for row in rows)
        out["medianEngineWithSnippetMs"] = statistics.median(row["engineWithSnippetMs"] for row in rows)
    for audience, cutoff in [("human", 5), ("agent", 10)]:
        labeled = [row for row in rows if row["audience"] == audience and row["hasLabel"]]
        out[audience] = {
            "queries": len(labeled),
            "misses": {
                mode: sum(row["ranks"][mode] is None or row["ranks"][mode] > cutoff for row in labeled)
                for mode in ["baseline", "filter", "coverage"]
            },
        }
    return out


def main():
    queries = read("queries.json")["queries"]
    with sqlite3.connect((LOCAL / "flicknote.db").as_uri() + "?mode=ro", uri=True) as database:
        notes = {
            row[0]: [(text or "").lower() for text in row[1:]]
            for row in database.execute(
                "SELECT short_id, title, summary, content FROM notes WHERE deleted_at IS NULL"
            )
        }
    rows = diagnose(queries, read("tantivy-results.json")["results"], notes)
    primary = [row for row in rows if row["group"] not in ("challenge", "cross-language")]
    report = {"all": summarize(rows), "primary": summarize(primary), "rows": rows}
    sequences = []
    for query in queries:
        if query["audience"] != "human" or query["group"] in ("challenge", "cross-language"):
            continue
        if not query["relevantIds"] or not query["terms"]:
            continue
        last = query["terms"][-1]
        for length in sorted({n for n in [1, 2, 3, len(last) - 1, len(last)] if 1 <= n <= len(last)}):
            terms = query["terms"][:-1] + [last[:length]]
            sequences.append({
                **query,
                "id": f"{query['id']}-input-{length}",
                "sourceId": query["id"],
                "group": "synthetic-input",
                "terms": terms,
                "query": " ".join(terms),
                "prefixLength": length,
                "isComplete": length == len(last),
            })
    fixture = {"queries": sequences}
    wide_fixture = {**fixture, "candidateLimit": 100}
    native_fixture = {**fixture, "mode": "hybridPrefix"}
    if (LOCAL / "tantivy-native-prefix-results.json").exists():
        assert read("native-prefix-queries.json") == native_fixture, "Native prefix fixture changed; rerun the engine"
        native = read("tantivy-native-prefix-results.json")
        native_rows = diagnose(sequences, native["results"], notes)
        report["nativePrefixRows"] = native_rows
        report["nativePrefix"] = {
            "indexBytes": native["indexBytes"],
            "residentAfterQueriesBytes": native["residentAfterQueriesBytes"],
            "partialTwoPlus": summarize([
                row for row in native_rows if row["prefixLength"] >= 2 and not row["isComplete"]
            ]),
            "complete": summarize([row for row in native_rows if row["isComplete"]]),
            "oneCharacter": summarize([row for row in native_rows if row["prefixLength"] == 1]),
        }
    if (LOCAL / "tantivy-input-100-results.json").exists():
        assert read("input-queries-100.json") == wide_fixture, "Wide input fixture changed; rerun the engine"
    prefix_results = LOCAL / "tantivy-input-results.json"
    if prefix_results.exists():
        # Prevent silently analyzing results against a changed fixture.
        assert read("input-queries.json") == fixture, "Input fixture changed; rerun the engine"
        input_rows = diagnose(sequences, read(prefix_results.name)["results"], notes)
        report["inputRows"] = input_rows
        report["input"] = {
            "sequences": len({row["sourceId"] for row in input_rows}),
            "oneCharacter": summarize([row for row in input_rows if row["prefixLength"] == 1]),
            "partialTwoPlus": summarize([
                row for row in input_rows if row["prefixLength"] >= 2 and not row["isComplete"]
            ]),
            "complete": summarize([row for row in input_rows if row["isComplete"]]),
        }
    write("input-queries.json", fixture)
    write("input-queries-100.json", wide_fixture)
    write("native-prefix-queries.json", native_fixture)
    if (LOCAL / "tantivy-input-100-results.json").exists():
        wide_rows = diagnose(sequences, read("tantivy-input-100-results.json")["results"], notes)
        report["input100Rows"] = wide_rows
        report["input100"] = {
            "partialTwoPlus": summarize([
                row for row in wide_rows if row["prefixLength"] >= 2 and not row["isComplete"]
            ]),
            "complete": summarize([row for row in wide_rows if row["isComplete"]]),
        }
    write("ngram-audit.json", report)
    print(json.dumps({key: value for key, value in report.items() if not key.endswith("Rows") and key != "rows"}, indent=2))
    print(f"Prepared {len(sequences)} frames across {len({q['sourceId'] for q in sequences})} input sequences")


if __name__ == "__main__":
    main()
