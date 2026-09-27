//! Indexed field coverage, followed by bounded literal verification.
use tantivy::query::{BooleanQuery, ConstScoreQuery, DisjunctionMaxQuery, Occur, Query, TermQuery};
use tantivy::schema::{Field, IndexRecordOption, Value};
use tantivy::{TantivyDocument, Term};

pub fn query_for(terms: &[String], fields: [Field; 3], words: [Field; 3]) -> Box<dyn Query> {
    let clauses = terms
        .iter()
        .map(|text| {
            let grams = super::grams(text);
            let alternatives = fields
                .into_iter()
                .enumerate()
                .map(|(i, field)| {
                    let query: Box<dyn Query> = if grams.is_empty() {
                        // One Latin letter uses the native prefix index; CJK remains unsupported.
                        super::prefix::query_for(
                            std::slice::from_ref(text),
                            [field; 3],
                            [words[i]; 3],
                        )
                        .query
                    } else {
                        Box::new(BooleanQuery::new(
                            grams
                                .iter()
                                .map(|gram| {
                                    (
                                        Occur::Must,
                                        Box::new(TermQuery::new(
                                            Term::from_field_text(field, gram),
                                            IndexRecordOption::Basic,
                                        ))
                                            as Box<dyn Query>,
                                    )
                                })
                                .collect(),
                        ))
                    };
                    Box::new(ConstScoreQuery::new(query, (3 - i) as f32)) as Box<dyn Query>
                })
                .collect();
            (
                Occur::Should,
                Box::new(DisjunctionMaxQuery::new(alternatives)) as Box<dyn Query>,
            )
        })
        .collect();
    Box::new(BooleanQuery::new(clauses))
}

pub fn score(document: &TantivyDocument, fields: [Field; 3], terms: &[String]) -> usize {
    let texts = fields.map(|field| {
        document
            .get_first(field)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_lowercase()
    });
    terms
        .iter()
        .map(|term| {
            let needle = term.to_lowercase();
            texts
                .iter()
                .position(|text| text.contains(&needle))
                .map_or(0, |i| 3 - i)
        })
        .sum()
}
