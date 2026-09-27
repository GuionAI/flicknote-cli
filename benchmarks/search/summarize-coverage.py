"""Aggregate private Tantivy coverage runs without publishing note/query text."""
import json
from pathlib import Path
import statistics

ROOT = Path(__file__).resolve().parent / '.local'

def summarize(fixture_name, report_name):
    fixture = json.loads((ROOT / fixture_name).read_text())
    report = json.loads((ROOT / report_name).read_text())
    queries = {q['id']: q for q in fixture['queries']}
    groups = {
        'human': lambda q: q['audience'] == 'human' and q['group'] not in ['challenge', 'cross-language', 'synthetic-input'],
        'agent': lambda q: q['audience'] == 'agent' and q['group'] not in ['challenge', 'cross-language'],
        'incomplete': lambda q: q.get('isComplete') is False and q.get('prefixLength', 0) >= 2,
        'complete': lambda q: q.get('isComplete') is True,
        'oneCharacter': lambda q: q.get('isComplete') is False and q.get('prefixLength') == 1,
    }
    out = {key: report[key] for key in ['noteCount', 'indexBytes', 'residentAfterQueriesBytes']}
    for name, predicate in groups.items():
        rows = [r for r in report['results'] if predicate(queries[r['id']]) and queries[r['id']]['relevantIds']]
        if not rows:
            continue
        cutoff = 10 if name == 'agent' else 5
        out[name] = {
            'queries': len(rows),
            'misses': sum(r['targetRank'] is None or r['targetRank'] > cutoff for r in rows),
            'medianSearchAndRerankMs': statistics.median(r['elapsedMs'] for r in rows),
            'medianWithFinalSnippetsMs': statistics.median(r['withSnippetMs'] for r in rows),
            'missingSnippets': sum(s is None for r in rows for s in r['snippets']),
        }
    return out

result = {
    name: summarize(fixture, report)
    for name, fixture, report in [
        ('original', 'coverage-queries.json', 'tantivy-coverage-results.json'),
        ('input', 'coverage-input-queries.json', 'tantivy-coverage-input-results.json'),
        ('growth', 'coverage-growth-queries.json', 'tantivy-coverage-growth-results.json'),
    ]
}
(ROOT / 'coverage-summary.json').write_text(json.dumps(result, indent=2))
print(json.dumps(result, indent=2))
