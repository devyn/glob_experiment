use anyhow::Result;

use std::{ffi::OsStr, path::Path};

#[derive(Debug, Clone)]
pub(crate) struct Pattern {
    pub(crate) components: Vec<PatternComponent>,
}

#[derive(Debug, Clone)]
pub(crate) enum PatternComponent {
    Prefix(Vec<u8>),
    RootDir,
    CurDir,
    ParentDir,
    Recurse,
    Normal(Tokens),
}

#[derive(Debug, Clone)]
pub(crate) struct Tokens(Vec<Token>);

#[derive(Debug, Clone)]
pub(crate) enum Token {
    AnyCharacter,
    Wildcard,
    Characters(Vec<CharacterClass>),
    Alternatives(Vec<Tokens>),
    Repeat { tokens: Tokens, min: u32, max: u32 },
    LiteralString(Vec<u8>),
}

#[derive(Debug, Clone)]
pub(crate) enum CharacterClass {
    Single(char),
    Range(char, char),
}

pub(crate) fn parse(string: impl AsRef<OsStr>) -> Pattern {
    let components = Path::new(string.as_ref())
        .components()
        .map(parse_component)
        .collect();
    Pattern { components }
}

pub(crate) fn parse_component(component: std::path::Component<'_>) -> PatternComponent {
    match component {
        std::path::Component::Prefix(prefix) => {
            PatternComponent::Prefix(prefix.as_os_str().as_encoded_bytes().into())
        }
        std::path::Component::RootDir => PatternComponent::RootDir,
        std::path::Component::CurDir => PatternComponent::CurDir,
        std::path::Component::ParentDir => PatternComponent::ParentDir,
        std::path::Component::Normal(string) if string.as_encoded_bytes() == b"**" => {
            PatternComponent::Recurse
        }
        std::path::Component::Normal(string) => {
            PatternComponent::Normal(parse_tokens(string.as_encoded_bytes(), |_| true).0)
        }
    }
}

pub(crate) fn parse_tokens(
    mut string: &[u8],
    mut cond: impl FnMut(&[u8]) -> bool,
) -> (Tokens, &[u8]) {
    let mut tokens = vec![];
    while !string.is_empty() && cond(string) {
        let (token, next_string) = next_token(string);
        tokens.push(token);
        string = next_string;
    }
    (Tokens(tokens), string)
}

pub(crate) fn next_token(string: &[u8]) -> (Token, &[u8]) {
    token_any_character(string)
        .or_else(token_wildcard)
        .or_else(token_alternatives)
        .or_else(token_character_class)
        .or_else(token_repeat)
        .or_else(token_literal_string)
        .unwrap_or_else(|remaining| {
            panic!("failed to generate token. remaining: {:?}", remaining);
        })
}

pub(crate) type TokenResult<'a> = Result<(Token, &'a [u8]), &'a [u8]>;

pub(crate) fn token_any_character(string: &[u8]) -> TokenResult {
    if string.get(0) == Some(&b'?') {
        Ok((Token::AnyCharacter, &string[1..]))
    } else {
        Err(string)
    }
}

pub(crate) fn token_wildcard(string: &[u8]) -> TokenResult {
    if string.get(0) == Some(&b'*') {
        Ok((Token::Wildcard, &string[1..]))
    } else {
        Err(string)
    }
}

pub(crate) fn token_alternatives(mut string: &[u8]) -> TokenResult {
    let original_string = string;
    if string.get(0) == Some(&b'{') {
        string = &string[1..];
        let mut alts = vec![];
        loop {
            let (alt, next_string) =
                parse_tokens(string, |string| !matches!(string.get(0), Some(b',' | b'}')));
            alts.push(alt);
            string = next_string;
            match string.get(0) {
                Some(b',') => {
                    string = &string[1..];
                }
                Some(b'}') => {
                    string = &string[1..];
                    break;
                }
                Some(_) => continue,
                None => return Err(original_string),
            }
        }
        Ok((Token::Alternatives(alts), string))
    } else {
        Err(original_string)
    }
}

pub(crate) fn token_character_class(mut string: &[u8]) -> TokenResult {
    let original_string = string;
    if string.get(0) == Some(&b'[') {
        string = &string[1..];
        let mut classes = vec![];
        loop {
            let Some((start_char, next_string)) = get_utf8_char(string) else {
                return Err(original_string);
            };
            string = next_string;
            let ch_class = if string.get(0) == Some(&b'-') {
                // This is a range, due to the - char
                string = &string[1..];
                let Some((end_char, next_string)) = get_utf8_char(string) else {
                    return Err(original_string);
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
                None => return Err(original_string),
            }
        }
        Ok((Token::Characters(classes), string))
    } else {
        Err(original_string)
    }
}

pub(crate) fn token_repeat(mut string: &[u8]) -> TokenResult {
    let original_string = string;
    if string.get(0) == Some(&b'<') {
        string = &string[1..];
        let (tokens, next_string) =
            parse_tokens(string, |string| !matches!(string.get(0), Some(b':')));
        string = next_string;
        if string.get(0) != Some(&b':') {
            return Err(original_string);
        }
        string = &string[1..];
        let Some(end_index) = string.iter().position(|byte| *byte == b'>') else {
            return Err(original_string);
        };
        let Ok(repeat_params_string) = std::str::from_utf8(&string[..end_index]) else {
            // The parameters must be valid UTF-8
            return Err(original_string);
        };
        string = &string[(end_index + 1)..];
        let token =
            if let Some(comma_index) = repeat_params_string.bytes().position(|byte| byte == b',') {
                let (min_string, max_string) = repeat_params_string.split_at(comma_index);
                let Ok(min): Result<u32, _> = min_string.parse() else {
                    // unparseable number
                    return Err(original_string);
                };
                let Ok(max): Result<u32, _> = max_string[1..].parse() else {
                    // unparseable number
                    return Err(original_string);
                };
                Token::Repeat { tokens, min, max }
            } else {
                let Ok(times): Result<u32, _> = repeat_params_string.parse() else {
                    // unparseable number
                    return Err(original_string);
                };
                Token::Repeat {
                    tokens,
                    min: times,
                    max: times,
                }
            };
        Ok((token, string))
    } else {
        Err(original_string)
    }
}

pub(crate) fn get_utf8_char(string: &[u8]) -> Option<(char, &[u8])> {
    string
        .utf8_chunks()
        .next()
        .and_then(|chunk| chunk.valid().chars().next())
        .map(|ch: char| (ch, &string[ch.len_utf8()..]))
}

pub(crate) fn token_literal_string(string: &[u8]) -> TokenResult {
    // Bytes that can start other tokens
    const MEANINGFUL_BYTES: &[u8] = b"*?[]{}<>,:";
    // Take at least one byte, but if we find a meaningful byte, leave that alone for further parsing
    if let Some(index_of_meaningful_byte) = string[1..]
        .iter()
        .position(|byte| MEANINGFUL_BYTES.contains(byte))
        .map(|idx| idx + 1)
    {
        Ok((
            Token::LiteralString(string[0..index_of_meaningful_byte].into()),
            &string[index_of_meaningful_byte..],
        ))
    } else {
        Ok((Token::LiteralString(string.into()), b""))
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
