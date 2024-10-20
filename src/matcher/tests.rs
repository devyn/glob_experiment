use std::path::Path;

use crate::{
    matcher::path_matches_pattern,
    parser::{Pattern, Token},
};

macro_rules! assert_result {
    ($result:expr, complete) => {{
        let result = $result;
        assert!(!result.valid_as_prefix);
        assert!(result.valid_as_complete_match);
    }};
    ($result:expr, none) => {{
        let result = $result;
        assert!(!result.valid_as_prefix);
        assert!(!result.valid_as_complete_match);
    }};
}

#[test]
fn empty_pattern_matches_empty_path() {
    let path = Path::new("");
    let pattern = Pattern::from(vec![]);
    let match_result = path_matches_pattern(path, &pattern);
    assert_result!(match_result, complete);
}

#[test]
fn single_literal_component() {
    let path = Path::new("foo");
    let pattern = Pattern::from(vec![Token::LiteralString(b"foo".into())]);
    let match_result = path_matches_pattern(path, &pattern);
    assert_result!(match_result, complete);
}

#[test]
fn mismatching_literal_string() {
    let path = Path::new("foo");
    let pattern = Pattern::from(vec![Token::LiteralString(b"bar".into())]);
    let match_result = path_matches_pattern(path, &pattern);
    assert_result!(match_result, none);
}

#[test]
fn literal_with_separator() {
    let path = Path::new("foo/bar");
    let pattern = Pattern::from(vec![
        Token::LiteralString(b"foo".into()),
        Token::Separator,
        Token::LiteralString(b"bar".into()),
    ]);
    let match_result = path_matches_pattern(path, &pattern);
    assert_result!(match_result, none);
}

#[test]
fn wildcard_matches_any_component() {
    let path = Path::new("foobarbaz");
    let pattern = Pattern::from(vec![Token::Wildcard]);
    let match_result = path_matches_pattern(path, &pattern);
    assert_result!(match_result, complete);
}

#[test]
fn wildcard_matches_infix() {
    let path = Path::new("foobarbaz");
    let pattern = Pattern::from(vec![
        Token::LiteralString(b"foo".into()),
        Token::Wildcard,
        Token::LiteralString(b"baz".into()),
    ]);
    let match_result = path_matches_pattern(path, &pattern);
    assert_result!(match_result, complete);
}

#[test]
fn wildcard_matches_prefix() {
    let path = Path::new("foobarbaz");
    let pattern = Pattern::from(vec![Token::Wildcard, Token::LiteralString(b"baz".into())]);
    let match_result = path_matches_pattern(path, &pattern);
    assert_result!(match_result, complete);
}

#[test]
fn wildcard_matches_suffix() {
    let path = Path::new("foobarbaz");
    let pattern = Pattern::from(vec![Token::LiteralString(b"foo".into()), Token::Wildcard]);
    let match_result = path_matches_pattern(path, &pattern);
    assert_result!(match_result, complete);
}
