use anyhow::Result;

use std::{ffi::OsStr, path::Path};

#[derive(Debug, Clone)]
pub struct Pattern {
    pub components: Vec<PatternComponent>,
}

#[derive(Debug, Clone)]
pub enum PatternComponent {
    Prefix(Vec<u8>),
    RootDir,
    CurDir,
    ParentDir,
    Recurse,
    Normal(Tokens),
}

#[derive(Debug, Clone)]
pub struct Tokens(Vec<Token>);

#[derive(Debug, Clone)]
pub enum Token {
    AnyCharacter,
    Wildcard,
    Characters(Vec<CharacterClass>),
    Alternatives(Vec<Tokens>),
    Repeat { tokens: Tokens, min: u32, max: u32 },
    LiteralString(Vec<u8>),
}

#[derive(Debug, Clone)]
pub enum CharacterClass {
    Single(char),
    Range(char, char),
}

pub fn parse(string: impl AsRef<OsStr>) -> Pattern {
    let components = Path::new(string.as_ref())
        .components()
        .map(parse_component)
        .collect();
    Pattern { components }
}

pub fn parse_component(component: std::path::Component<'_>) -> PatternComponent {
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

pub fn parse_tokens(mut string: &[u8], mut cond: impl FnMut(&[u8]) -> bool) -> (Tokens, &[u8]) {
    let mut tokens = vec![];
    while !string.is_empty() && cond(string) {
        let (token, next_string) = next_token(string);
        tokens.push(token);
        string = next_string;
    }
    (Tokens(tokens), string)
}

pub fn next_token(string: &[u8]) -> (Token, &[u8]) {
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

pub type TokenResult<'a> = Result<(Token, &'a [u8]), &'a [u8]>;

pub fn token_any_character(string: &[u8]) -> TokenResult {
    if string.get(0) == Some(&b'?') {
        Ok((Token::AnyCharacter, &string[1..]))
    } else {
        Err(string)
    }
}

pub fn token_wildcard(string: &[u8]) -> TokenResult {
    if string.get(0) == Some(&b'*') {
        Ok((Token::Wildcard, &string[1..]))
    } else {
        Err(string)
    }
}

pub fn token_alternatives(mut string: &[u8]) -> TokenResult {
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

pub fn token_character_class(mut string: &[u8]) -> TokenResult {
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

pub fn token_repeat(mut string: &[u8]) -> TokenResult {
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

pub fn get_utf8_char(string: &[u8]) -> Option<(char, &[u8])> {
    string
        .utf8_chunks()
        .next()
        .and_then(|chunk| chunk.valid().chars().next())
        .map(|ch: char| (ch, &string[ch.len_utf8()..]))
}

pub fn token_literal_string(string: &[u8]) -> TokenResult {
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
