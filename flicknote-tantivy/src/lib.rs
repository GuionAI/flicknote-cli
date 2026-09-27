//! Shared Tantivy schema and retrieval semantics for the daemon and search benchmark.

use std::collections::HashSet;
use std::path::Path;

use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, BoostQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    FAST, Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};
use tantivy::{Index, IndexReader, TantivyDocument, Term};

pub mod coverage;
pub mod prefix;

#[derive(Clone, Copy)]
pub struct Fields {
    pub uuid: Field,
    pub short_id: Field,
    pub project: Field,
    pub updated_at: Field,
    pub updated_sort: Field,
    pub text: [Field; 3],
    pub words: [Field; 3],
}

pub fn schema() -> (Schema, Fields) {
    let mut schema = Schema::builder();
    let uuid = schema.add_text_field("uuid", STRING | STORED);
    let short_id = schema.add_u64_field("short_id", STORED | FAST);
    let project = schema.add_text_field("project", STRING | STORED);
    let updated_at = schema.add_text_field("updated_at", STORED);
    let updated_sort = schema.add_i64_field("updated_sort", FAST);
    let ngram = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("ngram_2_3")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();
    let text =
        ["title", "summary", "content"].map(|name| schema.add_text_field(name, ngram.clone()));
    let word_options = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("words")
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    let words = ["title_words", "summary_words", "content_words"]
        .map(|name| schema.add_text_field(name, word_options.clone()));
    (
        schema.build(),
        Fields {
            uuid,
            short_id,
            project,
            updated_at,
            updated_sort,
            text,
            words,
        },
    )
}

pub fn open_new(path: &Path) -> tantivy::Result<(Index, Fields)> {
    let (schema, fields) = schema();
    let index = Index::create_in_dir(path, schema)?;
    register_tokenizers(&index)?;
    Ok((index, fields))
}

pub fn register_tokenizers(index: &Index) -> tantivy::Result<()> {
    index.tokenizers().register(
        "ngram_2_3",
        TextAnalyzer::builder(NgramTokenizer::new(2, 3, false)?)
            .filter(LowerCaser)
            .build(),
    );
    index.tokenizers().register("words", prefix::analyzer());
    Ok(())
}

pub fn document(fields: Fields, note: &Note<'_>) -> TantivyDocument {
    let mut doc = TantivyDocument::default();
    doc.add_text(fields.uuid, note.uuid);
    if let Some(id) = note.short_id {
        doc.add_u64(fields.short_id, id);
    }
    doc.add_text(fields.project, note.project);
    doc.add_text(fields.updated_at, note.updated_at);
    doc.add_i64(fields.updated_sort, note.updated_sort);
    for (i, value) in note.text.into_iter().enumerate() {
        doc.add_text(fields.text[i], value);
        doc.add_text(fields.words[i], value);
    }
    doc
}

pub struct Note<'a> {
    pub uuid: &'a str,
    pub short_id: Option<u64>,
    pub project: &'a str,
    pub updated_at: &'a str,
    pub updated_sort: i64,
    pub text: [&'a str; 3],
}

pub struct Hit {
    pub uuid: String,
    pub short_id: Option<u64>,
    pub snippet: Option<String>,
}

pub fn search(
    reader: &IndexReader,
    fields: Fields,
    terms: &[String],
    project: Option<&str>,
    limit: usize,
) -> tantivy::Result<Vec<Hit>> {
    if terms.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let searcher = reader.searcher();
    let mut query = coverage::query_for(terms, fields.text, fields.words);
    if let Some(project) = project {
        query = Box::new(BooleanQuery::new(vec![
            (Occur::Must, query),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(fields.project, project),
                    IndexRecordOption::Basic,
                )),
            ),
        ]));
    }
    let found = searcher.search(
        &query,
        &TopDocs::with_limit(100).tweak_score(|segment: &tantivy::SegmentReader| {
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
            move |doc, score: f32| (score as u64, dates.get_val(doc), ids.get_val(doc))
        }),
    )?;
    let mut ranked = Vec::new();
    for (_, address) in found {
        let doc: TantivyDocument = searcher.doc(address)?;
        let score = coverage::score(&doc, fields.text, terms);
        if score == 0 {
            continue;
        }
        let date = doc
            .get_first(fields.updated_at)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_owned();
        let id = doc.get_first(fields.short_id).and_then(|v| v.as_u64());
        ranked.push((score, date, id, doc));
    }
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
    });
    let generators = fields
        .text
        .map(|field| SnippetGenerator::create(&searcher, &*query, field))
        .into_iter()
        .collect::<tantivy::Result<Vec<_>>>()?;
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, _, id, doc)| {
            let uuid = doc
                .get_first(fields.uuid)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            let snippet = terms
                .iter()
                .filter(|term| term.chars().count() == 1)
                .find_map(|term| prefix::snippet(&doc, fields.text, &term.to_lowercase()))
                .or_else(|| {
                    let candidates = generators
                        .iter()
                        .map(|generator| generator.snippet_from_doc(&doc))
                        .filter(|snippet| !snippet.is_empty())
                        .collect::<Vec<_>>();
                    candidates
                        .iter()
                        .find(|snippet| {
                            let fragment = snippet.fragment().to_lowercase();
                            terms
                                .iter()
                                .any(|term| fragment.contains(&term.to_lowercase()))
                        })
                        .or_else(|| candidates.first())
                        .map(tantivy::snippet::Snippet::to_html)
                });
            Ok(Hit {
                uuid,
                short_id: id,
                snippet,
            })
        })
        .collect()
}

pub fn grams(value: &str) -> Vec<String> {
    let chars: Vec<char> = value.to_lowercase().chars().collect();
    let mut unique = HashSet::new();
    for size in 2..=3 {
        for window in chars.windows(size) {
            unique.insert(window.iter().collect::<String>());
        }
    }
    unique.into_iter().collect()
}

// The prefix module also uses this for CJK's fragment route.
pub fn query_for(terms: &[String], fields: [Field; 3]) -> Box<dyn Query> {
    let clauses = terms
        .iter()
        .filter_map(|text| {
            let grams = grams(text);
            if grams.is_empty() {
                return None;
            }
            let alternatives = fields
                .into_iter()
                .zip([3.0, 2.0, 1.0])
                .map(|(field, weight)| {
                    let required = grams
                        .iter()
                        .map(|gram| {
                            (
                                Occur::Must,
                                Box::new(TermQuery::new(
                                    Term::from_field_text(field, gram),
                                    IndexRecordOption::WithFreqs,
                                )) as Box<dyn Query>,
                            )
                        })
                        .collect();
                    (
                        Occur::Should,
                        Box::new(BoostQuery::new(
                            Box::new(BooleanQuery::new(required)),
                            weight,
                        )) as Box<dyn Query>,
                    )
                })
                .collect();
            Some((
                Occur::Should,
                Box::new(BooleanQuery::new(alternatives)) as Box<dyn Query>,
            ))
        })
        .collect();
    Box::new(BooleanQuery::new(clauses))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_search_ranks_verified_coverage_and_filters_project() {
        let dir = tempfile::tempdir().unwrap();
        let (index, fields) = open_new(dir.path()).unwrap();
        let mut writer = index.writer(50_000_000).unwrap();
        for (id, project, title, content, updated) in [
            (1, "alpha", "red chair", "", "2026-01-01T00:00:00Z"),
            (2, "alpha", "red", "chair chair", "2026-03-01T00:00:00Z"),
            (3, "alpha", "abc bcd", "", "2026-05-01T00:00:00Z"),
            (4, "alpha", "abcd", "", "2026-01-01T00:00:00Z"),
            (5, "beta", "red chair", "", "2026-06-01T00:00:00Z"),
        ] {
            writer
                .add_document(document(
                    fields,
                    &Note {
                        uuid: &format!("uuid-{id}"),
                        short_id: Some(id),
                        project,
                        updated_at: updated,
                        updated_sort: 0,
                        text: [title, "", content],
                    },
                ))
                .unwrap();
        }
        writer.commit().unwrap();
        let reader = index.reader().unwrap();
        reader.reload().unwrap();
        let found = search(
            &reader,
            fields,
            &["red".into(), "chair".into()],
            Some("alpha"),
            10,
        )
        .unwrap();
        assert_eq!(
            found
                .iter()
                .map(|hit| hit.short_id.unwrap())
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(found.iter().all(|hit| {
            hit.snippet
                .as_ref()
                .is_some_and(|snippet| snippet.contains("<b>"))
        }));
        let continuity = search(&reader, fields, &["abcd".into()], None, 10).unwrap();
        assert_eq!(
            continuity
                .iter()
                .map(|hit| hit.short_id.unwrap())
                .collect::<Vec<_>>(),
            [4]
        );
    }
}
