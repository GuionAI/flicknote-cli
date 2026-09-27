use super::*;

pub fn run(index_path: &Path, fixture_path: &Path) -> Result<ProbeReport, Box<dyn Error>> {
    let queries: Fixture = serde_json::from_slice(&fs::read(fixture_path)?)?;
    let candidate_limit =
        queries
            .candidate_limit
            .unwrap_or(if queries.mode == SearchMode::Coverage {
                100
            } else {
                10
            });
    if !(10..=1000).contains(&candidate_limit) {
        return Err("candidateLimit must be between 10 and 1000".into());
    }
    let index = Index::open_in_dir(index_path)?;
    index.tokenizers().register(
        "ngram_2_3",
        TextAnalyzer::builder(NgramTokenizer::new(2, 3, false)?)
            .filter(LowerCaser)
            .build(),
    );
    index.tokenizers().register("words", prefix::analyzer());
    let schema = index.schema();
    let short_id = schema.get_field("short_id")?;
    let project = schema.get_field("project")?;
    let fields = [
        schema.get_field("title")?,
        schema.get_field("summary")?,
        schema.get_field("content")?,
    ];
    let word_fields = if queries.mode != SearchMode::Ngram {
        Some([
            schema.get_field("title_words")?,
            schema.get_field("summary_words")?,
            schema.get_field("content_words")?,
        ])
    } else {
        None
    };
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let resident_after_open_bytes = resident_bytes();
    let mut results = Vec::with_capacity(queries.queries.len());
    for item in queries.queries {
        let started = Instant::now();
        let (mut query, word_prefix) = if let Some(words) = word_fields {
            let plan = if queries.mode == SearchMode::Coverage {
                prefix::PrefixPlan {
                    query: coverage::query_for(&item.terms, fields, words),
                    word_prefix: None,
                }
            } else {
                prefix::query_for(&item.terms, fields, words)
            };
            (plan.query, plan.word_prefix)
        } else {
            (query_for(&item.terms, fields), None)
        };
        if let Some(project_name) = &item.project {
            query = Box::new(BooleanQuery::new(vec![
                (Occur::Must, query),
                (
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(project, project_name),
                        IndexRecordOption::Basic,
                    )),
                ),
            ]));
        }
        let found = if queries.mode == SearchMode::Coverage {
            // Break coverage ties before truncating candidates, without loading their bodies.
            searcher
                .search(
                    &query,
                    &TopDocs::with_limit(candidate_limit).tweak_score(
                        |segment: &tantivy::SegmentReader| {
                            let dates = segment
                                .fast_fields()
                                .i64("updated_sort")
                                .expect("schema has updated_sort")
                                .first_or_default_col(0);
                            let ids = segment
                                .fast_fields()
                                .u64("short_id")
                                .expect("schema has short_id")
                                .first_or_default_col(0);
                            move |doc, score: f32| {
                                (score as u64, dates.get_val(doc), ids.get_val(doc))
                            }
                        },
                    ),
                )?
                .into_iter()
                .map(|(_, address)| address)
                .collect::<Vec<_>>()
        } else {
            searcher
                .search(
                    &query,
                    &TopDocs::with_limit(candidate_limit).order_by_score(),
                )?
                .into_iter()
                .map(|(_, address)| address)
                .collect()
        };
        let mut documents = Vec::with_capacity(found.len());
        let mut hits = Vec::with_capacity(found.len());
        for address in found {
            let document: TantivyDocument = searcher.doc(address)?;
            if let Some(id) = document
                .get_first(short_id)
                .and_then(|value| value.as_u64())
            {
                hits.push(id);
                documents.push(document);
            }
        }
        if queries.mode == SearchMode::Coverage {
            let updated = schema.get_field("updated_at")?;
            let mut ranked: Vec<_> = hits
                .into_iter()
                .zip(documents)
                .map(|(id, doc)| {
                    let score = coverage::score(&doc, fields, &item.terms);
                    let date = doc
                        .get_first(updated)
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    (score, date, id, doc)
                })
                .filter(|row| row.0 > 0)
                .collect();
            ranked.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| b.1.cmp(&a.1))
                    .then_with(|| b.2.cmp(&a.2))
            });
            (hits, documents) = ranked
                .into_iter()
                .take(10)
                .map(|(_, _, id, doc)| (id, doc))
                .unzip();
        }
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let generators = fields
            .map(|field| SnippetGenerator::create(&searcher, &*query, field))
            .into_iter()
            .collect::<tantivy::Result<Vec<_>>>()?;
        let snippets = documents
            .iter()
            .take(10)
            .map(|document| {
                if let Some(word_prefix) = &word_prefix {
                    return prefix::snippet(document, fields, word_prefix);
                }
                if queries.mode == SearchMode::Coverage {
                    for term in item.terms.iter().filter(|term| term.chars().count() == 1) {
                        if let Some(snippet) =
                            prefix::snippet(document, fields, &term.to_lowercase())
                        {
                            return Some(snippet);
                        }
                    }
                }
                let candidates = generators
                    .iter()
                    .map(|generator| generator.snippet_from_doc(document))
                    .filter(|snippet| !snippet.is_empty())
                    .collect::<Vec<_>>();
                candidates
                    .iter()
                    .find(|snippet| {
                        let fragment = snippet.fragment().to_lowercase();
                        item.terms
                            .iter()
                            .any(|term| fragment.contains(&term.to_lowercase()))
                    })
                    .or_else(|| candidates.first())
                    .map(|snippet| snippet.to_html())
            })
            .collect();
        let with_snippet_ms = started.elapsed().as_secs_f64() * 1000.0;
        let target_rank = hits
            .iter()
            .position(|id| item.relevant_ids.contains(id))
            .map(|position| position + 1);
        results.push(QueryResult {
            id: item.id,
            audience: item.audience,
            group: item.group,
            elapsed_ms,
            with_snippet_ms,
            hits,
            snippets,
            target_rank,
        });
    }
    let resident_after_queries_bytes = resident_bytes();
    Ok(ProbeReport {
        resident_after_open_bytes,
        resident_after_queries_bytes,
        median_query_ms: median(
            &results
                .iter()
                .map(|result| result.elapsed_ms)
                .collect::<Vec<_>>(),
        ),
        median_with_snippet_ms: median(
            &results
                .iter()
                .map(|result| result.with_snippet_ms)
                .collect::<Vec<_>>(),
        ),
        results,
    })
}
