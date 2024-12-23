use anyhow::Result;

use std::{
    ffi::OsStr,
    path::{is_separator, Component, Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Pattern {
    pub nodes: Vec<AstNode>,
}

#[derive(Debug, Clone)]
pub enum AstNode {
    Separator,
    Prefix(String),
    RootDir,
    CurDir,
    ParentDir,
    Recurse,
    LiteralString(Vec<u8>),
    AnyCharacter,
    Wildcard,
    Characters(Vec<CharacterClass>),
    Alternatives {
        choices: Vec<Pattern>,
    },
    Repeat {
        min: u32,
        max: u32,
        pattern: Pattern,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CharacterClass {
    Single(char),
    Range(char, char),
}

pub fn parse(string: impl AsRef<OsStr>) -> Pattern {
    let path = Path::new(string.as_ref());
    let mut components_iter = path.components().peekable();

    // Split the path into prefix components (where no glob pattern is allowed) and others
    let mut nodes = vec![];
    let mut path_relative = PathBuf::new();
    while let Some(Component::Prefix(..) | Component::RootDir) = components_iter.peek() {
        nodes.push(match components_iter.next() {
            Some(Component::Prefix(prefix_component)) => {
                AstNode::Prefix(prefix_component.as_os_str().to_string_lossy().into_owned())
            }
            Some(Component::RootDir) => AstNode::RootDir,
            _ => unreachable!(),
        });
    }
    path_relative.extend(components_iter);

    // Parse the remainder of the path into nodes
    parse_nodes(
        path_relative.as_os_str().as_encoded_bytes(),
        |_| true,
        &mut nodes,
    );

    Pattern { nodes }
}

pub fn parse_nodes<'a>(
    mut string: &'a [u8],
    mut cond: impl FnMut(&[u8]) -> bool,
    out: &mut Vec<AstNode>,
) -> &'a [u8] {
    while !string.is_empty() && cond(string) {
        string = next_node(string, out);
    }
    string
}

pub fn next_node<'a>(string: &'a [u8], out: &mut Vec<AstNode>) -> &'a [u8] {
    node_separator((string, out))
        .or_else(node_any_character)
        .or_else(node_recurse)
        .or_else(node_wildcard)
        .or_else(node_alternatives)
        .or_else(node_character_class)
        .or_else(node_repeat)
        .or_else(node_cur_or_parent_dir)
        .or_else(node_literal_string)
        .unwrap_or_else(|remaining| {
            panic!("failed to generate node. remaining: {:?}", remaining);
        })
        .0
}

type NodeInput<'a, 'b> = (&'a [u8], &'b mut Vec<AstNode>);
type NodeResult<'a, 'b> = Result<NodeInput<'a, 'b>, NodeInput<'a, 'b>>;

fn node_separator<'a, 'b>((string, out): NodeInput<'a, 'b>) -> NodeResult<'a, 'b> {
    match get_utf8_char(string) {
        Some((ch, next_string)) if is_separator(ch) => {
            out.push(AstNode::Separator);
            Ok((next_string, out))
        }
        _ => Err((string, out)),
    }
}

fn node_any_character<'a, 'b>((string, out): NodeInput<'a, 'b>) -> NodeResult<'a, 'b> {
    if string.get(0) == Some(&b'?') {
        out.push(AstNode::AnyCharacter);
        Ok((&string[1..], out))
    } else {
        Err((string, out))
    }
}

fn node_recurse<'a, 'b>((string, out): NodeInput<'a, 'b>) -> NodeResult<'a, 'b> {
    if string.get(0..2) == Some(b"**") {
        out.push(AstNode::Recurse);
        Ok((&string[2..], out))
    } else {
        Err((string, out))
    }
}

fn node_wildcard<'a, 'b>((string, out): NodeInput<'a, 'b>) -> NodeResult<'a, 'b> {
    if string.get(0) == Some(&b'*') {
        out.push(AstNode::Wildcard);
        Ok((&string[1..], out))
    } else {
        Err((string, out))
    }
}

fn node_alternatives<'a, 'b>((mut string, out): NodeInput<'a, 'b>) -> NodeResult<'a, 'b> {
    let original_string = string;
    let mut choices = vec![];
    let mut current_out = vec![];
    if string.get(0) == Some(&b'{') {
        string = &string[1..];
        loop {
            string = parse_nodes(
                string,
                |string| !matches!(string.get(0), Some(b',' | b'}')),
                &mut current_out,
            );
            match string.get(0) {
                Some(b',') => {
                    string = &string[1..];
                    let nodes = std::mem::replace(&mut current_out, vec![]);
                    choices.push(Pattern { nodes });
                }
                Some(b'}') => {
                    string = &string[1..];
                    choices.push(Pattern { nodes: current_out });
                    break;
                }
                Some(_) => continue,
                None => {
                    return Err((original_string, out));
                }
            }
        }
        out.push(AstNode::Alternatives { choices });
        Ok((string, out))
    } else {
        Err((original_string, out))
    }
}

fn node_character_class<'a, 'b>((mut string, out): NodeInput<'a, 'b>) -> NodeResult<'a, 'b> {
    let original_string = string;
    if string.get(0) == Some(&b'[') {
        string = &string[1..];
        let mut classes = vec![];
        loop {
            let Some((start_char, next_string)) = get_utf8_char(string) else {
                return Err((original_string, out));
            };
            string = next_string;
            let ch_class = if string.get(0) == Some(&b'-') {
                // This is a range, due to the - char
                string = &string[1..];
                let Some((end_char, next_string)) = get_utf8_char(string) else {
                    return Err((original_string, out));
                };
                string = next_string;
                CharacterClass::Range(start_char, end_char)
            } else {
                // It's a single char
                CharacterClass::Single(start_char)
            };
            classes.push(ch_class);
            match string.get(0) {
                Some(b']') => {
                    string = &string[1..];
                    break;
                }
                Some(_) => continue,
                None => return Err((original_string, out)),
            }
        }
        out.push(AstNode::Characters(classes));
        Ok((string, out))
    } else {
        Err((original_string, out))
    }
}

fn node_repeat<'a, 'b>((mut string, out): NodeInput<'a, 'b>) -> NodeResult<'a, 'b> {
    let original_string = string;
    let mut current_out = vec![];
    macro_rules! fail {
        () => {
            return Err((original_string, out));
        };
    }
    if string.get(0) == Some(&b'<') {
        string = &string[1..];
        string = parse_nodes(
            string,
            |string| !matches!(string.get(0), Some(b':')),
            &mut current_out,
        );
        if string.get(0) != Some(&b':') {
            fail!();
        }
        string = &string[1..];
        let Some(end_index) = string.iter().position(|byte| *byte == b'>') else {
            fail!();
        };
        let Ok(repeat_params_string) = std::str::from_utf8(&string[..end_index]) else {
            // The parameters must be valid UTF-8
            fail!();
        };
        string = &string[(end_index + 1)..];
        let node =
            if let Some(comma_index) = repeat_params_string.bytes().position(|byte| byte == b',') {
                let (min_string, max_string) = repeat_params_string.split_at(comma_index);
                let Ok(min): Result<u32, _> = min_string.parse() else {
                    // unparseable number
                    fail!();
                };
                let Ok(max): Result<u32, _> = max_string[1..].parse() else {
                    // unparseable number
                    fail!();
                };
                AstNode::Repeat {
                    min,
                    max,
                    pattern: Pattern { nodes: current_out },
                }
            } else {
                let Ok(times): Result<u32, _> = repeat_params_string.parse() else {
                    // unparseable number
                    fail!();
                };
                AstNode::Repeat {
                    min: times,
                    max: times,
                    pattern: Pattern { nodes: current_out },
                }
            };
        out.push(node);
        Ok((string, out))
    } else {
        Err((original_string, out))
    }
}

fn get_utf8_char(string: &[u8]) -> Option<(char, &[u8])> {
    string
        .utf8_chunks()
        .next()
        .and_then(|chunk| chunk.valid().chars().next())
        .map(|ch: char| (ch, &string[ch.len_utf8()..]))
}

fn starts_at_path_component_boundary(string: &[u8]) -> bool {
    string.is_empty() || get_utf8_char(string).is_some_and(|(ch, _)| is_separator(ch))
}

fn node_cur_or_parent_dir<'a, 'b>((string, out): NodeInput<'a, 'b>) -> NodeResult<'a, 'b> {
    // We have to look behind and ahead to make sure this is an isolated node
    match out.last() {
        None | Some(AstNode::RootDir) | Some(AstNode::Separator) => match string {
            [b'.', b'.', next_string @ ..] if starts_at_path_component_boundary(next_string) => {
                out.push(AstNode::ParentDir);
                Ok((next_string, out))
            }
            [b'.', next_string @ ..] if starts_at_path_component_boundary(next_string) => {
                out.push(AstNode::CurDir);
                Ok((next_string, out))
            }
            _ => Err((string, out)),
        },
        _ => Err((string, out)),
    }
}

fn node_literal_string<'a, 'b>((string, out): NodeInput<'a, 'b>) -> NodeResult<'a, 'b> {
    // Bytes that can start other nodes
    const MEANINGFUL_BYTES: &[u8] = b"*?[]{}<>,:/\\";
    // Take at least one byte, but if we find a meaningful byte, leave that alone for further parsing
    if let Some(index_of_meaningful_byte) = string[1..]
        .iter()
        .position(|byte| MEANINGFUL_BYTES.contains(byte))
        .map(|idx| idx + 1)
    {
        out.push(AstNode::LiteralString(
            string[0..index_of_meaningful_byte].into(),
        ));
        Ok((&string[index_of_meaningful_byte..], out))
    } else {
        out.push(AstNode::LiteralString(string.into()));
        Ok((b"", out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_prefix() {
        let pattern = parse("/Users/fdncred/src");
        println!("{:?}\n", pattern.components); // Debug print to inspect the components

        assert_eq!(pattern.components.len(), 4);

        if let PatternComponent::RootDir = &pattern.components[0] {
            // Expected
        } else {
            panic!(
                "Expected RootDir component, found {:?}",
                pattern.components[0]
            );
        }

        if let PatternComponent::Normal(tokens) = &pattern.components[1] {
            assert_eq!(tokens.0.len(), 1);
            if let Token::LiteralString(literal) = &tokens.0[0] {
                assert_eq!(literal, b"Users");
            } else {
                panic!("Expected LiteralString token, found {:?}", tokens.0[0]);
            }
        } else {
            panic!(
                "Expected Normal component, found {:?}",
                pattern.components[1]
            );
        }

        if let PatternComponent::Normal(tokens) = &pattern.components[2] {
            assert_eq!(tokens.0.len(), 1);
            if let Token::LiteralString(literal) = &tokens.0[0] {
                assert_eq!(literal, b"fdncred");
            } else {
                panic!("Expected LiteralString token, found {:?}", tokens.0[0]);
            }
        } else {
            panic!(
                "Expected Normal component, found {:?}",
                pattern.components[1]
            );
        }

        if let PatternComponent::Normal(tokens) = &pattern.components[3] {
            assert_eq!(tokens.0.len(), 1);
            if let Token::LiteralString(literal) = &tokens.0[0] {
                assert_eq!(literal, b"src");
            } else {
                panic!("Expected LiteralString token, found {:?}", tokens.0[0]);
            }
        } else {
            panic!(
                "Expected Normal component, found {:?}",
                pattern.components[2]
            );
        }
    }

    #[test]
    fn test_parse_root_dir() {
        let pattern = parse("/");
        assert_eq!(pattern.components.len(), 1);
        assert!(matches!(pattern.components[0], PatternComponent::RootDir));
    }

    #[test]
    fn test_parse_cur_dir() {
        let pattern = parse("./");
        assert_eq!(pattern.components.len(), 1);
        assert!(matches!(pattern.components[0], PatternComponent::CurDir));
    }

    #[test]
    fn test_parse_parent_dir() {
        let pattern = parse("../");
        assert_eq!(pattern.components.len(), 1);
        assert!(matches!(pattern.components[0], PatternComponent::ParentDir));
    }

    #[test]
    fn test_parse_recurse() {
        let pattern = parse("**");
        assert_eq!(pattern.components.len(), 1);
        assert!(matches!(pattern.components[0], PatternComponent::Recurse));
    }

    #[test]
    fn test_parse_normal() {
        let pattern = parse("src");
        assert_eq!(pattern.components.len(), 1);
        if let PatternComponent::Normal(tokens) = &pattern.components[0] {
            assert_eq!(tokens.0.len(), 1);
            if let Token::LiteralString(literal) = &tokens.0[0] {
                assert_eq!(literal, b"src");
            } else {
                panic!("Expected LiteralString token");
            }
        } else {
            panic!("Expected Normal component");
        }
    }

    #[test]
    fn test_token_any_character() {
        let (token, remaining) = token_any_character(b"?rest").unwrap();
        assert!(matches!(token, Token::AnyCharacter));
        assert_eq!(remaining, b"rest");
    }

    #[test]
    fn test_token_wildcard() {
        let (token, remaining) = token_wildcard(b"*rest").unwrap();
        assert!(matches!(token, Token::Wildcard));
        assert_eq!(remaining, b"rest");
    }

    #[test]
    fn test_token_alternatives() {
        let (token, remaining) = token_alternatives(b"{a,b}rest").unwrap();
        if let Token::Alternatives(alts) = token {
            assert_eq!(alts.len(), 2);
            assert_eq!(alts[0].0.len(), 1);
            assert_eq!(alts[1].0.len(), 1);
            if let Token::LiteralString(literal) = &alts[0].0[0] {
                assert_eq!(literal, b"a");
            } else {
                panic!("Expected LiteralString token");
            }
            if let Token::LiteralString(literal) = &alts[1].0[0] {
                assert_eq!(literal, b"b");
            } else {
                panic!("Expected LiteralString token");
            }
        } else {
            panic!("Expected Alternatives token");
        }
        assert_eq!(remaining, b"rest");
    }

    #[test]
    fn test_token_character_class() {
        let (token, remaining) = token_character_class(b"[a-z]rest").unwrap();
        if let Token::Characters(classes) = token {
            assert_eq!(classes.len(), 1);
            if let CharacterClass::Range(start, end) = &classes[0] {
                assert_eq!(*start, 'a');
                assert_eq!(*end, 'z');
            } else {
                panic!("Expected Range character class");
            }
        } else {
            panic!("Expected Characters token");
        }
        assert_eq!(remaining, b"rest");
    }

    #[test]
    fn test_token_repeat() {
        let (token, remaining) = token_repeat(b"<a:2>rest").unwrap();
        if let Token::Repeat { tokens, min, max } = token {
            assert_eq!(min, 2);
            assert_eq!(max, 2);
            assert_eq!(tokens.0.len(), 1);
            if let Token::LiteralString(literal) = &tokens.0[0] {
                assert_eq!(literal, b"a");
            } else {
                panic!("Expected LiteralString token");
            }
        } else {
            panic!("Expected Repeat token");
        }
        assert_eq!(remaining, b"rest");
    }

    #[test]
    fn test_token_literal_string() {
        let (token, remaining) = token_literal_string(b"literal*rest").unwrap();
        if let Token::LiteralString(literal) = token {
            assert_eq!(literal, b"literal");
        } else {
            panic!("Expected LiteralString token");
        }
        assert_eq!(remaining, b"*rest");
    }

    #[test]
    fn test_parse_glob_wildcard() {
        let pattern = parse("src/*");
        assert_eq!(pattern.components.len(), 2);
        if let PatternComponent::Normal(tokens) = &pattern.components[1] {
            assert_eq!(tokens.0.len(), 1);
            assert!(matches!(tokens.0[0], Token::Wildcard));
        } else {
            panic!("Expected Normal component with Wildcard token");
        }
    }

    #[test]
    fn test_parse_glob_any_character() {
        let pattern = parse("src/fi?e");
        assert_eq!(pattern.components.len(), 2);
        if let PatternComponent::Normal(tokens) = &pattern.components[1] {
            assert_eq!(tokens.0.len(), 3);
            assert!(matches!(tokens.0[1], Token::AnyCharacter));
        } else {
            panic!("Expected Normal component with AnyCharacter token");
        }
    }

    #[test]
    fn test_parse_glob_alternatives() {
        let pattern = parse("src/{file,dir}");
        assert_eq!(pattern.components.len(), 2);
        if let PatternComponent::Normal(tokens) = &pattern.components[1] {
            assert_eq!(tokens.0.len(), 1);
            if let Token::Alternatives(alts) = &tokens.0[0] {
                assert_eq!(alts.len(), 2);
                assert_eq!(alts[0].0.len(), 1);
                assert_eq!(alts[1].0.len(), 1);
                if let Token::LiteralString(literal) = &alts[0].0[0] {
                    assert_eq!(literal, b"file");
                } else {
                    panic!("Expected LiteralString token");
                }
                if let Token::LiteralString(literal) = &alts[1].0[0] {
                    assert_eq!(literal, b"dir");
                } else {
                    panic!("Expected LiteralString token");
                }
            } else {
                panic!("Expected Alternatives token");
            }
        } else {
            panic!("Expected Normal component with Alternatives token");
        }
    }

    #[test]
    fn test_parse_glob_character_class() {
        let pattern = parse("src/[a-z]ile");
        assert_eq!(pattern.components.len(), 2);
        if let PatternComponent::Normal(tokens) = &pattern.components[1] {
            assert_eq!(tokens.0.len(), 2);
            if let Token::Characters(classes) = &tokens.0[0] {
                assert_eq!(classes.len(), 1);
                if let CharacterClass::Range(start, end) = &classes[0] {
                    assert_eq!(*start, 'a');
                    assert_eq!(*end, 'z');
                } else {
                    panic!("Expected Range character class");
                }
            } else {
                panic!("Expected Characters token");
            }
        } else {
            panic!("Expected Normal component with Characters token");
        }
    }

    #[test]
    fn test_parse_glob_repeat() {
        let pattern = parse("src/<a:2>");
        assert_eq!(pattern.components.len(), 2);
        if let PatternComponent::Normal(tokens) = &pattern.components[1] {
            assert_eq!(tokens.0.len(), 1);
            if let Token::Repeat { tokens, min, max } = &tokens.0[0] {
                assert_eq!(*min, 2);
                assert_eq!(*max, 2);
                assert_eq!(tokens.0.len(), 1);
                if let Token::LiteralString(literal) = &tokens.0[0] {
                    assert_eq!(literal, b"a");
                } else {
                    panic!("Expected LiteralString token");
                }
            } else {
                panic!("Expected Repeat token");
            }
        } else {
            panic!("Expected Normal component with Repeat token");
        }
    }

    #[test]
    fn test_parse_glob_single_character() {
        let pattern = parse("src/f?le");
        assert_eq!(pattern.components.len(), 2);
        if let PatternComponent::Normal(tokens) = &pattern.components[1] {
            assert_eq!(tokens.0.len(), 3);
            assert!(matches!(tokens.0[1], Token::AnyCharacter));
        } else {
            panic!("Expected Normal component with AnyCharacter token");
        }
    }

    #[test]
    fn test_parse_glob_multiple_wildcards() {
        let pattern = parse("src/*/file/*");
        assert_eq!(pattern.components.len(), 4);
        if let PatternComponent::Normal(tokens) = &pattern.components[1] {
            assert_eq!(tokens.0.len(), 1);
            assert!(matches!(tokens.0[0], Token::Wildcard));
        } else {
            panic!("Expected Normal component with Wildcard token");
        }
        if let PatternComponent::Normal(tokens) = &pattern.components[3] {
            assert_eq!(tokens.0.len(), 1);
            assert!(matches!(tokens.0[0], Token::Wildcard));
        } else {
            panic!("Expected Normal component with Wildcard token");
        }
    }

    // This one may be too crazy, nested alternatives?
    // #[test]
    // fn test_parse_glob_nested_alternatives() {
    //     let pattern = parse("src/{file,dir/{sub1,sub2}}");
    //     assert_eq!(pattern.components.len(), 2);
    //     if let PatternComponent::Normal(tokens) = &pattern.components[1] {
    //         assert_eq!(tokens.0.len(), 1);
    //         if let Token::Alternatives(alts) = &tokens.0[0] {
    //             assert_eq!(alts.len(), 2);
    //             assert_eq!(alts[0].0.len(), 1);
    //             assert_eq!(alts[1].0.len(), 1);
    //             if let Token::LiteralString(literal) = &alts[0].0[0] {
    //                 assert_eq!(literal, b"file");
    //             } else {
    //                 panic!("Expected LiteralString token");
    //             }
    //             if let Token::Alternatives(sub_alts) = &alts[1].0[0] {
    //                 assert_eq!(sub_alts.len(), 2);
    //                 if let Token::LiteralString(literal) = &sub_alts[0].0[0] {
    //                     assert_eq!(literal, b"sub1");
    //                 } else {
    //                     panic!("Expected LiteralString token");
    //                 }
    //                 if let Token::LiteralString(literal) = &sub_alts[1].0[0] {
    //                     assert_eq!(literal, b"sub2");
    //                 } else {
    //                     panic!("Expected LiteralString token");
    //                 }
    //             } else {
    //                 panic!("Expected Alternatives token");
    //             }
    //         } else {
    //             panic!("Expected Alternatives token");
    //         }
    //     } else {
    //         panic!("Expected Normal component with Alternatives token");
    //     }
    // }

    #[test]
    fn test_parse_glob_complex_pattern() {
        let pattern = parse("src/{file,dir}/[a-z]*.{rs,txt}");
        assert_eq!(pattern.components.len(), 3);
        if let PatternComponent::Normal(tokens) = &pattern.components[1] {
            assert_eq!(tokens.0.len(), 1);
            if let Token::Alternatives(alts) = &tokens.0[0] {
                assert_eq!(alts.len(), 2);
                assert_eq!(alts[0].0.len(), 1);
                assert_eq!(alts[1].0.len(), 1);
                if let Token::LiteralString(literal) = &alts[0].0[0] {
                    assert_eq!(literal, b"file");
                } else {
                    panic!("Expected LiteralString token");
                }
                if let Token::LiteralString(literal) = &alts[1].0[0] {
                    assert_eq!(literal, b"dir");
                } else {
                    panic!("Expected LiteralString token");
                }
            } else {
                panic!("Expected Alternatives token");
            }
        } else {
            panic!("Expected Normal component with Alternatives token");
        }
        if let PatternComponent::Normal(tokens) = &pattern.components[2] {
            assert_eq!(tokens.0.len(), 4);
            if let Token::Characters(classes) = &tokens.0[0] {
                assert_eq!(classes.len(), 1);
                if let CharacterClass::Range(start, end) = &classes[0] {
                    assert_eq!(*start, 'a');
                    assert_eq!(*end, 'z');
                } else {
                    panic!("Expected Range character class");
                }
            } else {
                panic!("Expected Characters token");
            }
            assert!(matches!(tokens.0[1], Token::Wildcard));
            if let Token::Alternatives(alts) = &tokens.0[3] {
                assert_eq!(alts.len(), 2);
                if let Token::LiteralString(literal) = &alts[0].0[0] {
                    assert_eq!(literal, b"rs");
                } else {
                    panic!("Expected LiteralString token");
                }
                if let Token::LiteralString(literal) = &alts[1].0[0] {
                    assert_eq!(literal, b"txt");
                } else {
                    panic!("Expected LiteralString token");
                }
            } else {
                panic!("Expected Alternatives token");
            }
        } else {
            panic!("Expected Normal component with Characters and Alternatives tokens");
        }
    }
}
