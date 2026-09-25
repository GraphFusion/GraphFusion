use crate::error::{Error, Result};
use crate::token::{keyword_or_identifier, Token, TokenKind};

#[derive(Clone, Debug)]
pub struct Lexer<'a> {
    input: &'a str,
    offset: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self { input, offset: 0 }
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }

    fn peek_next_char(&self) -> Option<char> {
        let mut chars = self.input[self.offset..].chars();
        chars.next()?;
        chars.next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.offset += ch.len_utf8();
        Some(ch)
    }

    fn token_from(&self, kind: TokenKind, start: usize) -> Token {
        Token::new(kind, &self.input[start..self.offset], start)
    }

    fn eat_while(&mut self, pred: impl Fn(char) -> bool) {
        while matches!(self.peek_char(), Some(ch) if pred(ch)) {
            self.bump();
        }
    }

    fn skip_ws_and_comments(&mut self) -> Result<()> {
        loop {
            self.eat_while(char::is_whitespace);

            if self.input[self.offset..].starts_with("//")
                || self.input[self.offset..].starts_with("--")
            {
                self.eat_while(|ch| ch != '\n');
                continue;
            }

            if self.input[self.offset..].starts_with("/*") {
                let start = self.offset;
                self.offset += 2;
                while self.offset < self.input.len() && !self.input[self.offset..].starts_with("*/")
                {
                    self.bump();
                }
                if self.input[self.offset..].starts_with("*/") {
                    self.offset += 2;
                } else {
                    return Err(Error::UnterminatedBlockComment {
                        offset: start,
                        token_text: self.input[start..].to_owned(),
                    });
                }
                continue;
            }

            break;
        }
        Ok(())
    }

    fn identifier(&mut self, start: usize) -> Token {
        self.eat_while(is_identifier_continue);
        let text = &self.input[start..self.offset];
        Token::new(keyword_or_identifier(text), text, start)
    }

    fn quoted_identifier(&mut self, start: usize, quote: char) -> Result<Token> {
        self.quoted_identifier_with_escape(start, quote, true)
    }

    fn quoted_identifier_no_escape(&mut self, start: usize) -> Result<Token> {
        self.bump();
        let quote = self.peek_char().ok_or_else(|| Error::UnrecognizedToken {
            offset: start,
            token_text: self.input[start..self.offset].to_owned(),
        })?;
        self.quoted_identifier_with_escape(start, quote, false)
    }

    fn quoted_identifier_with_escape(
        &mut self,
        start: usize,
        quote: char,
        escaping_enabled: bool,
    ) -> Result<Token> {
        let value = self.quoted_sequence(start, quote, escaping_enabled)?;
        let kind = if quote == '"' {
            TokenKind::DoubleQuotedString
        } else {
            TokenKind::QuotedString
        };
        Ok(Token::new(kind, value, start))
    }

    fn quoted_sequence(
        &mut self,
        start: usize,
        quote: char,
        escaping_enabled: bool,
    ) -> Result<String> {
        if self.bump() != Some(quote) {
            return Err(Error::UnrecognizedToken {
                offset: start,
                token_text: self.input[start..self.offset].to_owned(),
            });
        }
        let mut value = String::new();
        loop {
            match self.bump() {
                Some(ch) if ch == quote => {
                    if self.peek_char() == Some(quote) {
                        self.bump();
                        value.push(quote);
                    } else {
                        return Ok(value);
                    }
                }
                Some('\\') if escaping_enabled => value.push(self.escaped_character(start)?),
                Some(ch) => value.push(ch),
                None => {
                    return Err(Error::UnterminatedString {
                        offset: start,
                        token_text: self.input[start..].to_owned(),
                    })
                }
            }
        }
    }

    fn string(&mut self, start: usize) -> Result<Token> {
        self.character_string(start, true)
    }

    fn string_no_escape(&mut self, start: usize) -> Result<Token> {
        self.bump();
        self.character_string(start, false)
    }

    fn character_string(&mut self, start: usize, mut escaping_enabled: bool) -> Result<Token> {
        let mut value = String::new();
        loop {
            self.expect_string_chunk(start, &mut value, escaping_enabled)?;
            if self.eat_literal_separator() {
                if self.peek_char() == Some('@') && self.peek_next_char() == Some('\'') {
                    self.bump();
                    escaping_enabled = false;
                    continue;
                }
                if self.peek_char() == Some('\'') {
                    escaping_enabled = true;
                    continue;
                }
            }
            break;
        }
        Ok(Token::new(TokenKind::String, value, start))
    }

    fn expect_string_chunk(
        &mut self,
        start: usize,
        value: &mut String,
        escaping_enabled: bool,
    ) -> Result<()> {
        if self.bump() != Some('\'') {
            return Err(Error::UnrecognizedToken {
                offset: start,
                token_text: self.input[start..self.offset].to_owned(),
            });
        }

        loop {
            match self.bump() {
                Some('\'') => {
                    if self.peek_char() == Some('\'') {
                        self.bump();
                        value.push('\'');
                    } else {
                        return Ok(());
                    }
                }
                Some('\\') if escaping_enabled => value.push(self.escaped_character(start)?),
                Some(ch) => value.push(ch),
                None => {
                    return Err(Error::UnterminatedString {
                        offset: start,
                        token_text: self.input[start..].to_owned(),
                    })
                }
            }
        }
    }

    fn escaped_character(&mut self, string_start: usize) -> Result<char> {
        let escape_start = self.offset.saturating_sub('\\'.len_utf8());
        match self.bump() {
            Some('\\') => Ok('\\'),
            Some('\'') => Ok('\''),
            Some('"') => Ok('"'),
            Some('`') => Ok('`'),
            Some('t') => Ok('\t'),
            Some('b') => Ok('\u{0008}'),
            Some('n') => Ok('\n'),
            Some('r') => Ok('\r'),
            Some('f') => Ok('\u{000c}'),
            Some('u') => self.unicode_escape(escape_start, 4),
            Some('U') => self.unicode_escape(escape_start, 6),
            Some(ch) => Err(Error::Message {
                offset: escape_start,
                message: format!("invalid string escape '\\{ch}'"),
            }),
            None => Err(Error::UnterminatedString {
                offset: string_start,
                token_text: self.input[string_start..].to_owned(),
            }),
        }
    }

    fn unicode_escape(&mut self, escape_start: usize, digits: usize) -> Result<char> {
        let mut value = 0_u32;
        for _ in 0..digits {
            match self.bump().and_then(|ch| ch.to_digit(16)) {
                Some(digit) => value = value * 16 + digit,
                None => {
                    return Err(Error::Message {
                        offset: escape_start,
                        message: "invalid string escape".to_owned(),
                    })
                }
            }
        }
        char::from_u32(value).ok_or_else(|| Error::Message {
            offset: escape_start,
            message: "invalid string escape".to_owned(),
        })
    }

    fn byte_string(&mut self, start: usize) -> Result<Token> {
        self.bump();
        let mut value = String::new();
        loop {
            self.expect_byte_string_chunk(start, &mut value)?;

            if self.eat_literal_separator() && self.peek_char() == Some('\'') {
                continue;
            }
            break;
        }

        if !value.len().is_multiple_of(2) {
            return Err(Error::Message {
                offset: start,
                message: format!(
                    "invalid byte string literal '{}'",
                    &self.input[start..self.offset]
                ),
            });
        }
        Ok(Token::new(TokenKind::ByteString, value, start))
    }

    fn expect_byte_string_chunk(&mut self, start: usize, value: &mut String) -> Result<()> {
        if self.bump() != Some('\'') {
            return Err(Error::UnrecognizedToken {
                offset: start,
                token_text: self.input[start..self.offset].to_owned(),
            });
        }

        loop {
            let offset = self.offset;
            match self.bump() {
                Some('\'') => return Ok(()),
                Some(ch) if ch.is_ascii_hexdigit() => value.push(ch),
                Some(ch) if ch.is_whitespace() => {}
                Some(ch) => {
                    return Err(Error::Message {
                        offset,
                        message: format!("invalid byte string literal character '{ch}'"),
                    })
                }
                None => {
                    return Err(Error::UnterminatedString {
                        offset: start,
                        token_text: self.input[start..].to_owned(),
                    })
                }
            }
        }
    }

    fn eat_literal_separator(&mut self) -> bool {
        let mut saw_newline = false;
        while matches!(self.peek_char(), Some(ch) if ch.is_whitespace()) {
            saw_newline |= self.peek_char().is_some_and(|ch| ch == '\n' || ch == '\r');
            self.bump();
        }
        saw_newline
    }

    fn number(&mut self, start: usize) -> Result<Token> {
        if self.peek_char() == Some('.') {
            let mut text = String::from(".");
            self.bump();
            self.digits_with_underscores(start, 10, false, &mut text)?;
            let kind = self.consume_numeric_suffix(TokenKind::Decimal, start)?;
            return Ok(Token::new(kind, text, start));
        }

        if self.peek_char() == Some('0') {
            if matches!(self.peek_next_char(), Some('x' | 'X')) {
                return self.radix_integer(start, 16);
            }
            if matches!(self.peek_next_char(), Some('o' | 'O')) {
                return self.radix_integer(start, 8);
            }
            if matches!(self.peek_next_char(), Some('b' | 'B')) {
                return self.radix_integer(start, 2);
            }
        }

        let mut text = String::new();
        self.digits_with_underscores(start, 10, false, &mut text)?;
        let mut kind = TokenKind::Integer;
        if self.peek_char() == Some('.')
            && !matches!(self.peek_next_char(), Some(ch) if is_identifier_start(ch) || ch == '.')
        {
            kind = TokenKind::Decimal;
            self.bump();
            text.push('.');
            if matches!(self.peek_char(), Some(ch) if ch.is_ascii_digit()) {
                self.digits_with_underscores(start, 10, false, &mut text)?;
            }
        }
        if matches!(self.peek_char(), Some('e' | 'E')) {
            kind = TokenKind::Decimal;
            self.bump();
            text.push('e');
            if matches!(self.peek_char(), Some('+' | '-')) {
                text.push(self.bump().expect("peeked sign"));
            }
            let exp_start = self.offset;
            self.digits_with_underscores(start, 10, false, &mut text)?;
            if exp_start == self.offset {
                return Err(Error::BadNumber {
                    offset: start,
                    token_text: self.input[start..self.offset].to_owned(),
                });
            }
        }
        kind = self.consume_numeric_suffix(kind, start)?;
        if matches!(self.peek_char(), Some('_')) {
            self.bump();
            return Err(Error::BadNumber {
                offset: start,
                token_text: self.input[start..self.offset].to_owned(),
            });
        }
        Ok(Token::new(kind, text, start))
    }

    fn consume_numeric_suffix(&mut self, kind: TokenKind, start: usize) -> Result<TokenKind> {
        let kind = if matches!(self.peek_char(), Some('m' | 'M' | 'f' | 'F' | 'd' | 'D')) {
            self.bump();
            TokenKind::Decimal
        } else {
            kind
        };
        if matches!(self.peek_char(), Some(ch) if is_identifier_continue(ch)) {
            self.bump();
            return Err(Error::BadNumber {
                offset: start,
                token_text: self.input[start..self.offset].to_owned(),
            });
        }
        Ok(kind)
    }

    fn radix_integer(&mut self, start: usize, radix: u32) -> Result<Token> {
        self.bump();
        self.bump();
        let mut digits = String::new();
        self.digits_with_underscores(start, radix, true, &mut digits)?;
        if digits.is_empty() || matches!(self.peek_char(), Some(ch) if is_identifier_continue(ch)) {
            return Err(Error::BadNumber {
                offset: start,
                token_text: self.input[start..self.offset].to_owned(),
            });
        }
        let value = i64::from_str_radix(&digits, radix).map_err(|_| Error::BadNumber {
            offset: start,
            token_text: self.input[start..self.offset].to_owned(),
        })?;
        Ok(Token::new(TokenKind::Integer, value.to_string(), start))
    }

    fn digits_with_underscores(
        &mut self,
        start: usize,
        radix: u32,
        allow_leading_underscore: bool,
        out: &mut String,
    ) -> Result<usize> {
        let mut digits = 0;
        let mut pending_underscore = false;
        loop {
            match self.peek_char() {
                Some('_') if allow_leading_underscore || digits > 0 => {
                    if pending_underscore {
                        self.bump();
                        return Err(Error::BadNumber {
                            offset: start,
                            token_text: self.input[start..self.offset].to_owned(),
                        });
                    }
                    pending_underscore = true;
                    self.bump();
                }
                Some(ch) if ch.is_digit(radix) => {
                    out.push(ch);
                    digits += 1;
                    pending_underscore = false;
                    self.bump();
                }
                _ => break,
            }
        }
        if pending_underscore {
            return Err(Error::BadNumber {
                offset: start,
                token_text: self.input[start..self.offset].to_owned(),
            });
        }
        Ok(digits)
    }

    fn parameter(&mut self, start: usize) -> Result<Token> {
        self.bump();

        if self.peek_char() == Some('@') && matches!(self.peek_next_char(), Some('"' | '`')) {
            self.bump();
            let quote = self.peek_char().expect("peeked parameter quote");
            let value = self.quoted_sequence(start, quote, false)?;
            return Ok(Token::new(TokenKind::Parameter, value, start));
        }

        if matches!(self.peek_char(), Some('"' | '`')) {
            let quote = self.peek_char().expect("peeked parameter quote");
            let value = self.quoted_sequence(start, quote, true)?;
            return Ok(Token::new(TokenKind::Parameter, value, start));
        }

        if matches!(self.peek_char(), Some(ch) if is_identifier_continue(ch)) {
            self.eat_while(is_identifier_continue);
            return Ok(self.token_from(TokenKind::Parameter, start));
        }

        Err(Error::UnrecognizedToken {
            offset: start,
            token_text: "$".to_owned(),
        })
    }
}

impl Iterator for Lexer<'_> {
    type Item = Result<Token>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Err(err) = self.skip_ws_and_comments() {
            return Some(Err(err));
        }
        let start = self.offset;
        let ch = self.peek_char()?;
        let token = match ch {
            '(' => {
                self.bump();
                Ok(self.token_from(TokenKind::LParen, start))
            }
            ')' => {
                self.bump();
                Ok(self.token_from(TokenKind::RParen, start))
            }
            '[' => {
                self.bump();
                Ok(self.token_from(TokenKind::LBracket, start))
            }
            ']' => {
                self.bump();
                Ok(self.token_from(TokenKind::RBracket, start))
            }
            '{' => {
                self.bump();
                Ok(self.token_from(TokenKind::LBrace, start))
            }
            '}' => {
                self.bump();
                Ok(self.token_from(TokenKind::RBrace, start))
            }
            ',' => {
                self.bump();
                Ok(self.token_from(TokenKind::Comma, start))
            }
            ':' => {
                self.bump();
                if self.peek_char() == Some(':') {
                    self.bump();
                    Ok(self.token_from(TokenKind::DoubleColon, start))
                } else {
                    Ok(self.token_from(TokenKind::Colon, start))
                }
            }
            ';' => {
                self.bump();
                Ok(self.token_from(TokenKind::Semicolon, start))
            }
            '.' if matches!(self.peek_next_char(), Some(ch) if ch.is_ascii_digit()) => {
                self.number(start)
            }
            '.' => {
                self.bump();
                Ok(self.token_from(TokenKind::Dot, start))
            }
            '+' => {
                self.bump();
                Ok(self.token_from(TokenKind::Plus, start))
            }
            '?' => {
                self.bump();
                Ok(self.token_from(TokenKind::Question, start))
            }
            '*' => {
                self.bump();
                Ok(self.token_from(TokenKind::Star, start))
            }
            '/' => {
                self.bump();
                Ok(self.token_from(TokenKind::Slash, start))
            }
            '%' => {
                self.bump();
                Ok(self.token_from(TokenKind::Percent, start))
            }
            '|' => {
                self.bump();
                if self.peek_char() == Some('|') {
                    self.bump();
                    Ok(self.token_from(TokenKind::DoublePipe, start))
                } else {
                    Ok(self.token_from(TokenKind::Pipe, start))
                }
            }
            '&' => {
                self.bump();
                Ok(self.token_from(TokenKind::Ampersand, start))
            }
            '~' => {
                self.bump();
                Ok(self.token_from(TokenKind::Tilde, start))
            }
            '=' => {
                self.bump();
                Ok(self.token_from(TokenKind::Eq, start))
            }
            '!' if self.peek_next_char() == Some('=') => {
                self.bump();
                self.bump();
                Ok(self.token_from(TokenKind::Neq, start))
            }
            '!' => {
                self.bump();
                Ok(self.token_from(TokenKind::Bang, start))
            }
            '<' if self.peek_next_char() == Some('=') => {
                self.bump();
                self.bump();
                Ok(self.token_from(TokenKind::Le, start))
            }
            '<' if self.peek_next_char() == Some('>') => {
                self.bump();
                self.bump();
                Ok(self.token_from(TokenKind::Neq, start))
            }
            '<' if self.peek_next_char() == Some('-') => {
                self.bump();
                self.bump();
                Ok(self.token_from(TokenKind::ArrowLeft, start))
            }
            '<' => {
                self.bump();
                Ok(self.token_from(TokenKind::Lt, start))
            }
            '>' if self.peek_next_char() == Some('=') => {
                self.bump();
                self.bump();
                Ok(self.token_from(TokenKind::Ge, start))
            }
            '>' => {
                self.bump();
                Ok(self.token_from(TokenKind::Gt, start))
            }
            '-' if self.peek_next_char() == Some('>') => {
                self.bump();
                self.bump();
                Ok(self.token_from(TokenKind::ArrowRight, start))
            }
            '-' => {
                self.bump();
                Ok(self.token_from(TokenKind::Minus, start))
            }
            '\'' => self.string(start),
            '"' | '`' => self.quoted_identifier(start, ch),
            '@' if self.peek_next_char() == Some('\'') => self.string_no_escape(start),
            '@' if matches!(self.peek_next_char(), Some('"' | '`')) => {
                self.quoted_identifier_no_escape(start)
            }
            '$' => self.parameter(start),
            ch if ch.is_ascii_digit() => self.number(start),
            'X' | 'x' if self.peek_next_char() == Some('\'') => self.byte_string(start),
            ch if is_identifier_start(ch) => Ok(self.identifier(start)),
            _ => {
                self.bump();
                Err(Error::UnrecognizedToken {
                    offset: start,
                    token_text: ch.to_string(),
                })
            }
        };
        Some(token)
    }
}

pub fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_alphabetic()
}

pub fn is_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}
