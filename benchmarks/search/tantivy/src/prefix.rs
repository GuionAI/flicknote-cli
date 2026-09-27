use std::collections::BTreeMap;

use tantivy::query::{
    BooleanQuery, BoostQuery, EmptyQuery, FuzzyTermQuery, Occur, Query, TermQuery,
};
use tantivy::schema::{Field, IndexRecordOption, Value};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{TantivyDocument, Term};

pub fn analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build()
}

pub struct PrefixPlan {
    pub query: Box<dyn Query>,
    pub word_prefix: Option<String>,
}

pub fn query_for(terms: &[String], fragments: [Field; 3], words: [Field; 3]) -> PrefixPlan {
    let Some((last, preceding)) = terms.split_last() else {
        return PrefixPlan {
            query: Box::new(EmptyQuery),
            word_prefix: None,
        };
    };
    // SimpleTokenizer does not segment CJK words. Keep the existing fragment route.
    let cjk = last.chars().any(|c| matches!(c as u32,
        0x3040..=0x30ff | 0x3400..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0x20000..=0x3134f
    ));
    let mut word_prefix = None;
    let required = if cjk {
        super::query_for(std::slice::from_ref(last), fragments)
    } else {
        let mut analyzer = analyzer();
        let mut stream = analyzer.token_stream(last);
        let mut tokens = Vec::new();
        while let Some(token) = stream.next() {
            tokens.push(token.text.clone());
        }
        word_prefix = tokens.last().cloned();
        let token_count = tokens.len();
        let clauses = tokens
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let fields = words
                    .into_iter()
                    .zip([3.0, 2.0, 1.0])
                    .map(|(field, weight)| {
                        let term = Term::from_field_text(field, text);
                        let query: Box<dyn Query> = if i + 1 == token_count {
                            // Native term-prefix automaton; distance 0 deliberately disables typos.
                            Box::new(FuzzyTermQuery::new_prefix(term, 0, true))
                        } else {
                            Box::new(TermQuery::new(term, IndexRecordOption::WithFreqs))
                        };
                        (
                            Occur::Should,
                            Box::new(BoostQuery::new(query, weight)) as Box<dyn Query>,
                        )
                    })
                    .collect();
                (
                    Occur::Must,
                    Box::new(BooleanQuery::new(fields)) as Box<dyn Query>,
                )
            })
            .collect();
        Box::new(BooleanQuery::new(clauses)) as Box<dyn Query>
    };
    let mut clauses = vec![(Occur::Must, required)];
    // Native prefix automata have constant scores. Retain ngram BM25 for ranking.
    let ranking_terms = if cjk { preceding } else { terms };
    if !ranking_terms.is_empty() {
        clauses.push((Occur::Should, super::query_for(ranking_terms, fragments)));
    }
    PrefixPlan {
        query: Box::new(BooleanQuery::new(clauses)),
        word_prefix,
    }
}

pub fn snippet(document: &TantivyDocument, fields: [Field; 3], prefix: &str) -> Option<String> {
    for field in fields {
        let text = document
            .get_first(field)
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let mut tokenizer = analyzer();
        let mut tokens = tokenizer.token_stream(text);
        let mut matched = BTreeMap::new();
        while let Some(token) = tokens.next() {
            if token.text.starts_with(prefix) {
                matched.insert(token.text.clone(), 1.0);
            }
        }
        if !matched.is_empty() {
            // Automaton queries do not enumerate query_terms for SnippetGenerator::create.
            // Supply the exact matched words from this hit to the public constructor.
            return Some(
                SnippetGenerator::new(matched, analyzer(), field, 150)
                    .snippet_from_doc(document)
                    .to_html(),
            );
        }
    }
    None
}
