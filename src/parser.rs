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
