use anyhow::Result;

use std::{
    ffi::OsStr,
    path::{is_separator, Component, Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Pattern {
    pub tokens: Tokens,
}

#[derive(Debug, Clone, Default)]
pub struct Tokens(Vec<Token>);

impl std::ops::Deref for Tokens {
    type Target = Vec<Token>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Tokens {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone)]
pub enum Token {
    Separator,
    Prefix(Vec<u8>),
    RootDir,
    Recurse,
    LiteralString(Vec<u8>),
    AnyCharacter,
    Wildcard,
    Characters(Vec<CharacterClass>),
    BeginScope,
    EndScope,
    Alternative,
    Repeat { min: u32, max: u32 },
}

#[derive(Debug, Clone)]
pub enum CharacterClass {
    Single(char),
    Range(char, char),
}

pub fn parse(string: impl AsRef<OsStr>) -> Pattern {
    let path = Path::new(string.as_ref());
    let mut components_iter = path.components().peekable();

    // Split the path into prefix components (where no glob pattern is allowed) and others
    let mut tokens = Tokens::default();
    let mut path_relative = PathBuf::new();
    while let Some(Component::Prefix(..) | Component::RootDir) = components_iter.peek() {
        tokens.push(match components_iter.next() {
            Some(Component::Prefix(prefix_component)) => {
                Token::Prefix(prefix_component.as_os_str().as_encoded_bytes().into())
            }
            Some(Component::RootDir) => Token::RootDir,
            _ => unreachable!(),
        });
    }
    path_relative.extend(components_iter);

    // Parse the remainder of the path into tokens
    parse_tokens(
        path_relative.as_os_str().as_encoded_bytes(),
        |_| true,
        &mut tokens,
    );

    Pattern { tokens }
}

pub fn parse_tokens<'a>(
    mut string: &'a [u8],
    mut cond: impl FnMut(&[u8]) -> bool,
    out: &mut Tokens,
) -> &'a [u8] {
    while !string.is_empty() && cond(string) {
        string = next_token(string, out);
    }
    string
}

pub fn next_token<'a>(string: &'a [u8], out: &mut Tokens) -> &'a [u8] {
    token_separator((string, out))
        .or_else(token_any_character)
        .or_else(token_recurse)
        .or_else(token_wildcard)
        .or_else(token_alternatives)
        .or_else(token_character_class)
        .or_else(token_repeat)
        .or_else(token_literal_string)
        .unwrap_or_else(|remaining| {
            panic!("failed to generate token. remaining: {:?}", remaining);
        })
        .0
}

type TokenInput<'a, 'b> = (&'a [u8], &'b mut Tokens);
type TokenResult<'a, 'b> = Result<TokenInput<'a, 'b>, TokenInput<'a, 'b>>;

fn token_separator<'a, 'b>((string, out): TokenInput<'a, 'b>) -> TokenResult<'a, 'b> {
    match get_utf8_char(string) {
        Some((ch, next_string)) if is_separator(ch) => {
            out.push(Token::Separator);
            Ok((next_string, out))
        }
        _ => Err((string, out)),
    }
}

fn token_any_character<'a, 'b>((string, out): TokenInput<'a, 'b>) -> TokenResult<'a, 'b> {
    if string.get(0) == Some(&b'?') {
        out.push(Token::AnyCharacter);
        Ok((&string[1..], out))
    } else {
        Err((string, out))
    }
}

fn token_recurse<'a, 'b>((string, out): TokenInput<'a, 'b>) -> TokenResult<'a, 'b> {
    if string.get(0..2) == Some(b"**") {
        out.push(Token::Recurse);
        Ok((&string[2..], out))
    } else {
        Err((string, out))
    }
}

fn token_wildcard<'a, 'b>((string, out): TokenInput<'a, 'b>) -> TokenResult<'a, 'b> {
    if string.get(0) == Some(&b'*') {
        out.push(Token::Wildcard);
        Ok((&string[1..], out))
    } else {
        Err((string, out))
    }
}

fn token_alternatives<'a, 'b>((mut string, out): TokenInput<'a, 'b>) -> TokenResult<'a, 'b> {
    let original_string = string;
    let original_len = out.len();
    if string.get(0) == Some(&b'{') {
        string = &string[1..];
        out.push(Token::BeginScope);
        loop {
            string = parse_tokens(
                string,
                |string| !matches!(string.get(0), Some(b',' | b'}')),
                out,
            );
            match string.get(0) {
                Some(b',') => {
                    string = &string[1..];
                    out.push(Token::Alternative);
                }
                Some(b'}') => {
                    string = &string[1..];
                    out.push(Token::EndScope);
                    break;
                }
                Some(_) => continue,
                None => {
                    out.truncate(original_len);
                    return Err((original_string, out));
                }
            }
        }
        Ok((string, out))
    } else {
        Err((original_string, out))
    }
}

fn token_character_class<'a, 'b>((mut string, out): TokenInput<'a, 'b>) -> TokenResult<'a, 'b> {
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
        out.push(Token::Characters(classes));
        Ok((string, out))
    } else {
        Err((original_string, out))
    }
}

fn token_repeat<'a, 'b>((mut string, out): TokenInput<'a, 'b>) -> TokenResult<'a, 'b> {
    let original_string = string;
    let original_len = out.len();
    macro_rules! fail {
        () => {
            out.truncate(original_len);
            return Err((original_string, out));
        };
    }
    if string.get(0) == Some(&b'<') {
        out.push(Token::BeginScope);
        string = &string[1..];
        string = parse_tokens(string, |string| !matches!(string.get(0), Some(b':')), out);
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
        let token =
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
                Token::Repeat { min, max }
            } else {
                let Ok(times): Result<u32, _> = repeat_params_string.parse() else {
                    // unparseable number
                    fail!();
                };
                Token::Repeat {
                    min: times,
                    max: times,
                }
            };
        out.push(token);
        out.push(Token::EndScope);
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

fn token_literal_string<'a, 'b>((string, out): TokenInput<'a, 'b>) -> TokenResult<'a, 'b> {
    // Bytes that can start other tokens
    const MEANINGFUL_BYTES: &[u8] = b"*?[]{}<>,:/\\";
    // Take at least one byte, but if we find a meaningful byte, leave that alone for further parsing
    if let Some(index_of_meaningful_byte) = string[1..]
        .iter()
        .position(|byte| MEANINGFUL_BYTES.contains(byte))
        .map(|idx| idx + 1)
    {
        out.push(Token::LiteralString(
            string[0..index_of_meaningful_byte].into(),
        ));
        Ok((&string[index_of_meaningful_byte..], out))
    } else {
        out.push(Token::LiteralString(string.into()));
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
