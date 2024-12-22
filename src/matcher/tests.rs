use std::path::Path;

use crate::{
    matcher::path_matches_pattern,
    parser::{AstNode, Pattern},
};

macro_rules! assert_result {
    ($path:expr, $pattern:expr, complete) => {{
        let result = path_matches_pattern($path, &$pattern);
        assert!(!result.valid_as_prefix);
        assert!(result.valid_as_complete_match);
    }};
    ($path:expr, $pattern:expr, prefix) => {{
        let result = path_matches_pattern($path, &$pattern);
        assert!(result.valid_as_prefix);
        assert!(!result.valid_as_complete_match);
    }};
    ($path:expr, $pattern:expr, complete_and_prefix) => {{
        let result = path_matches_pattern($path, &$pattern);
        assert!(result.valid_as_prefix);
        assert!(result.valid_as_complete_match);
    }};
    ($path:expr, $pattern:expr, none) => {{
        let result = path_matches_pattern($path, &$pattern);
        assert!(!result.valid_as_prefix);
        assert!(!result.valid_as_complete_match);
    }};
}

#[test]
fn empty_pattern_matches_empty_path() {
    let path = Path::new("");
    let pattern = Pattern::from(vec![]);
    assert_result!(path, pattern, complete);
}

#[test]
fn single_literal_component() {
    let path = Path::new("foo");
    let pattern = Pattern::from(vec![AstNode::LiteralString(b"foo".into())]);
    assert_result!(path, pattern, complete);
}

#[test]
fn mismatching_literal_string() {
    let path = Path::new("foo");
    let pattern = Pattern::from(vec![AstNode::LiteralString(b"bar".into())]);
    assert_result!(path, pattern, none);
}

#[test]
fn literal_with_separator() {
    let path = Path::new("foo/bar");
    let pattern = Pattern::from(vec![
        AstNode::LiteralString(b"foo".into()),
        AstNode::Separator,
        AstNode::LiteralString(b"bar".into()),
    ]);
    assert_result!(path, pattern, complete);
}

#[test]
fn wildcard_matches_any_component() {
    let path = Path::new("foobarbaz");
    let pattern = Pattern::from(vec![AstNode::Wildcard]);
    assert_result!(path, pattern, complete);
}

#[test]
fn wildcard_matches_infix() {
    let path = Path::new("foobarbaz");
    let pattern = Pattern::from(vec![
        AstNode::LiteralString(b"foo".into()),
        AstNode::Wildcard,
        AstNode::LiteralString(b"baz".into()),
    ]);
    assert_result!(path, pattern, complete);
}

#[test]
fn wildcard_matches_prefix() {
    let path = Path::new("foobarbaz");
    let pattern = Pattern::from(vec![
        AstNode::Wildcard,
        AstNode::LiteralString(b"baz".into()),
    ]);
    assert_result!(path, pattern, complete);
}

#[test]
fn wildcard_matches_suffix() {
    let path = Path::new("foobarbaz");
    let pattern = Pattern::from(vec![
        AstNode::LiteralString(b"foo".into()),
        AstNode::Wildcard,
    ]);
    assert_result!(path, pattern, complete);
}

fn foo_recurse() -> Pattern {
    Pattern::from(vec![
        AstNode::LiteralString(b"foo".into()),
        AstNode::Separator,
        AstNode::Recurse,
    ])
}

#[test]
fn recurse_matches_prefix() {
    let path = Path::new("foo");
    let pattern = foo_recurse();
    assert_result!(path, pattern, complete_and_prefix);
}

#[test]
fn recurse_matches_nested_1() {
    let path = Path::new("foo/bar");
    let pattern = foo_recurse();
    assert_result!(path, pattern, complete_and_prefix);
}

#[test]
fn recurse_matches_nested_2() {
    let path = Path::new("foo/bar/baz");
    let pattern = foo_recurse();
    assert_result!(path, pattern, complete_and_prefix);
}

fn foo_recurse_bar() -> Pattern {
    Pattern::from(vec![
        AstNode::LiteralString(b"foo".into()),
        AstNode::Separator,
        AstNode::Recurse,
        AstNode::Separator,
        AstNode::LiteralString(b"bar".into()),
    ])
}

#[test]
fn recurse_matches_infix_empty() {
    let path = Path::new("foo/bar");
    let pattern = foo_recurse_bar();
    assert_result!(path, pattern, complete);
}

#[test]
fn recurse_matches_infix_nested_1() {
    let path = Path::new("foo/baz/bar");
    let pattern = foo_recurse_bar();
    assert_result!(path, pattern, complete);
}

#[test]
fn recurse_matches_infix_nested_2() {
    let path = Path::new("foo/baz/quux/bar");
    let pattern = foo_recurse_bar();
    assert_result!(path, pattern, complete);
}

#[test]
fn recurse_encourages_infix_further_match() {
    let path = Path::new("foo");
    let pattern = foo_recurse_bar();
    assert_result!(path, pattern, prefix);
}

#[test]
fn recurse_encourages_infix_further_match_1() {
    let path = Path::new("foo/baz");
    let pattern = foo_recurse_bar();
    assert_result!(path, pattern, prefix);
}
