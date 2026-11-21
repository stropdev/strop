//! The plans by concern: file selection, the content plan, the row
//! decision. Shared helpers live here.

use super::*;
use crate::query::facts::RowFacts;
use crate::query::parser::SearchQuery;
use crate::query::QueryState;

mod admission;
mod content;
mod file;

fn plan(input: &str) -> (SearchQuery, FileSelectionPlan) {
    let query = crate::query::parse_search(input);
    let plan = FileSelectionPlan::compile(&query).unwrap();
    (query, plan)
}

fn content_plan(input: &str) -> ContentPlan {
    let query = crate::query::parse_search(input);
    assert_eq!(
        query.state,
        QueryState::Ready,
        "{input}: {:?}",
        query.diagnostics
    );
    ContentPlan::compile(&query)
        .unwrap()
        .expect("content required")
}

/// A line of a file, with no evidence beyond itself.
fn line<'a>(path: &'a str, text: &'a str) -> RowFacts<'a> {
    RowFacts {
        path: Some(path),
        native_path: Some(std::path::Path::new(path)),
        text,
        ..RowFacts::default()
    }
}

fn admits_line(plan: &ContentPlan, path: &str, text: &str) -> bool {
    plan.admits(&line(path, text), &Evidence::default())
}
