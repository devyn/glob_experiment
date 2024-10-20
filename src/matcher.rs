use std::path::{Component, Components, Path};

use crate::parser::{Pattern, Token};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy)]
pub struct MatchResult {
    /// True if the match could be made valid with more path components.
    ///
    /// A complete match may not be valid as a prefix if there's no way the pattern could accept
    /// any more path components.
    pub valid_as_prefix: bool,
    /// True if the match is completely valid as a match for the glob.
    pub valid_as_complete_match: bool,
}

impl MatchResult {
    pub fn none() -> Self {
        MatchResult {
            valid_as_prefix: false,
            valid_as_complete_match: false,
        }
    }
}

#[derive(Debug, Clone)]
enum Fallback {
    Wildcard(usize),
}

fn next_string<'a, 'b>(
    path_components: &mut Components<'b>,
    current_string: &'a mut Option<&'b [u8]>,
) -> Option<&'a mut &'b [u8]> {
    if let Some(ref mut s) = current_string {
        Some(s)
    } else if let Some(component) = path_components.next() {
        match component {
            Component::Normal(normal_str) => {
                *current_string = Some(normal_str.as_encoded_bytes());
                current_string.as_mut()
            }
            _ => None,
        }
    } else {
        None
    }
}

fn length_of_first_char(string: &[u8]) -> Option<usize> {
    string.utf8_chunks().next().map(|chunk| {
        chunk
            .valid()
            .chars()
            .next()
            .map(|ch| ch.len_utf8())
            .unwrap_or(1)
    })
}

#[derive(Debug)]
struct Matcher<'a> {
    pc: usize,
    path_components: Components<'a>,
    current_string: Option<&'a [u8]>,
    fallback_stack: Vec<Fallback>,
    result: MatchResult,
}

impl<'a> Matcher<'a> {
    fn advance(&mut self, tokens: &[Token]) -> Option<MatchResult> {
        eprintln!("{:?}, {:?}", self, tokens);
        if self.pc >= tokens.len() {
            Some(MatchResult {
                valid_as_prefix: false,
                valid_as_complete_match: self.path_components.next().is_none(),
            })
        } else {
            let token = &tokens[self.pc];
            let success = match token {
                Token::Separator => self.separator(),
                Token::Prefix(_) => todo!(),
                Token::RootDir => todo!(),
                Token::Recurse => todo!(),
                Token::LiteralString(string) => self.literal_string(string),
                Token::AnyCharacter => todo!(),
                Token::Wildcard => self.wildcard(),
                Token::Characters(_) => todo!(),
                Token::BeginScope => todo!(),
                Token::EndScope => todo!(),
                Token::Alternative => todo!(),
                Token::Repeat { min, max } => todo!(),
            };
            if success {
                None
            } else {
                if self.fallback() {
                    None
                } else {
                    Some(self.result)
                }
            }
        }
    }

    fn fallback(&mut self) -> bool {
        while let Some(fallback) = self.fallback_stack.pop() {
            match fallback {
                Fallback::Wildcard(wildcard_pc) => {
                    // Try to advance by one char
                    if next_string(&mut self.path_components, &mut self.current_string)
                        .and_then(|current_string| {
                            length_of_first_char(&current_string).map(|len| {
                                *current_string = &current_string[len..];
                            })
                        })
                        .is_some()
                    {
                        // If we could, try again from the wildcard pc
                        self.pc = wildcard_pc;
                        return true;
                    } else {
                        // Go back to next failure case
                        continue;
                    }
                }
            }
        }
        false
    }

    fn separator(&mut self) -> bool {
        if self.current_string.is_some_and(|s| !s.is_empty()) {
            false
        } else {
            self.current_string = None;
            self.result.valid_as_prefix = true;
            self.pc += 1;
            true
        }
    }

    fn wildcard(&mut self) -> bool {
        // Wildcard can only match a string component
        if next_string(&mut self.path_components, &mut self.current_string).is_some() {
            self.fallback_stack.push(Fallback::Wildcard(self.pc));
            self.pc += 1;
            true
        } else {
            false
        }
    }

    fn literal_string(&mut self, string: &[u8]) -> bool {
        if let Some(current_string) =
            next_string(&mut self.path_components, &mut self.current_string)
        {
            self.result.valid_as_prefix = false;
            if current_string.starts_with(&string) {
                *current_string = &current_string[string.len()..];

                self.pc += 1;
                true
            } else {
                false
            }
        } else {
            false
        }
    }
}

pub fn path_matches_pattern(path: &Path, pattern: &Pattern) -> MatchResult {
    let mut matcher = Matcher {
        pc: 0,
        path_components: path.components(),
        current_string: None,
        fallback_stack: vec![],
        result: MatchResult {
            valid_as_prefix: true,
            valid_as_complete_match: false,
        },
    };

    loop {
        match matcher.advance(&pattern.tokens) {
            Some(result) => return result,
            None => continue,
        }
    }
}
