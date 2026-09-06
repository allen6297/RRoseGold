//! Hand-written lexer: source text → [`Token`] / [`TokenKind`].

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    Fn,
    Import,
    As,
    From,
    Var,
    Const,
    If,
    Else,
    Elif,
    While,
    For,
    In,
    Return,
    Pass,
    Break,
    Continue,
    Match,
    Struct,
    Class,
    Trait,
    Extends,
    Enum,
    Impl,
    Mod,
    Signal,
    Pub,
    True,
    False,
    None,
    Self_,
    Super,
    // Types
    Void,
    Bool,
    Int,
    Float,
    StringType,
    Array,
    Map,
    // Literals
    IntLit(i64),
    FloatLit(f64),
    StringLit(String),
    FStringLit(String),
    Ident(String),
    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    SlashSlash,
    Percent,
    Eq,
    EqEq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    AndAnd,
    OrOr,
    Bang,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    AmpAssign,
    PipeAssign,
    CaretAssign,
    Assign,
    PlusAssign,
    MinusAssign,
    StarAssign,
    SlashAssign,
    PercentAssign,
    Range,
    RangeInclusive,
    // Punctuation
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Semicolon,
    Dot,
    Comma,
    Question,
    Arrow, // `->` return type, same as `:` after `)`
    At,
    /// `##` rest-of-line. Attaches to the next parsed item. `#` is skipped, not this.
    DocComment(String),
    /// `#` line comment. Kept so `fmt` can reprint it.
    Comment(String),
    // Special
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub span: crate::Span,
}

pub struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: usize,
    col: usize,
    current: String,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            chars: source.chars().peekable(),
            line: 1,
            col: 1,
            current: String::new(),
        }
    }

    fn keyword_or_ident(name: String) -> TokenKind {
        match name.as_str() {
            "fn" => TokenKind::Fn,
            "import" => TokenKind::Import,
            "as" => TokenKind::As,
            "from" => TokenKind::From,
            "var" => TokenKind::Var,
            "const" => TokenKind::Const,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "elif" => TokenKind::Elif,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "in" => TokenKind::In,
            "return" => TokenKind::Return,
            "pass" => TokenKind::Pass,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "match" => TokenKind::Match,
            "struct" => TokenKind::Struct,
            "class" => TokenKind::Class,
            "trait" => TokenKind::Trait,
            "extends" => TokenKind::Extends,
            "enum" => TokenKind::Enum,
            "impl" => TokenKind::Impl,
            "mod" => TokenKind::Mod,
            "signal" => TokenKind::Signal,
            "pub" => TokenKind::Pub,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "none" => TokenKind::None,
            "self" => TokenKind::Self_,
            "super" => TokenKind::Super,
            "Void" => TokenKind::Void,
            "Bool" => TokenKind::Bool,
            "Int" => TokenKind::Int,
            "Float" => TokenKind::Float,
            "String" => TokenKind::StringType,
            "Array" => TokenKind::Array,
            "Map" => TokenKind::Map,
            _ => TokenKind::Ident(name),
        }
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        self.current.push(c);
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn peek(&mut self) -> Option<&char> {
        self.chars.peek()
    }

    fn skip_whitespace(&mut self) {
        while let Some(&c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    /// Consume `#` and the rest of the line. One leading space after `#` is dropped.
    fn read_line_comment(&mut self) -> String {
        self.advance(); // #
        if matches!(self.peek(), Some(' ') | Some('\t')) {
            self.advance();
        }
        let mut text = String::new();
        while let Some(&c) = self.peek() {
            if c == '\n' {
                break;
            }
            text.push(c);
            self.advance();
        }
        text.trim_end().to_string()
    }

    /// Consume `##` and the rest of the line. One leading space after `##` is dropped.
    fn read_doc_comment(&mut self) -> String {
        self.advance(); // #
        self.advance(); // #
        if matches!(self.peek(), Some(' ') | Some('\t')) {
            self.advance();
        }
        let mut text = String::new();
        while let Some(&c) = self.peek() {
            if c == '\n' {
                break;
            }
            text.push(c);
            self.advance();
        }
        text.trim_end().to_string()
    }

    fn read_string(&mut self, prefix: char) -> Result<String, String> {
        let is_fstring = prefix == 'f';
        let quote = self
            .advance()
            .ok_or_else(|| "unterminated string".to_string())?;
        if quote != '"' {
            return Err(format!("expected string quote, got {:?}", quote));
        }
        let mut value = String::new();
        loop {
            let c = self
                .advance()
                .ok_or_else(|| "unterminated string".to_string())?;
            if c == '"' {
                break;
            }
            if c == '\\' {
                let esc = self
                    .advance()
                    .ok_or_else(|| "unterminated escape".to_string())?;
                match esc {
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    '\\' => value.push('\\'),
                    '"' => value.push('"'),
                    _ => value.push(esc),
                }
            } else {
                value.push(c);
            }
        }
        if is_fstring { Ok(value) } else { Ok(value) }
    }

    fn read_number(&mut self, first: char) -> TokenKind {
        let mut value = first.to_string();
        let mut is_float = false;
        while let Some(&c) = self.peek() {
            if c.is_ascii_digit() {
                value.push(c);
                self.advance();
            } else if c == '.' && !is_float {
                if let Some(next) = self.chars.clone().nth(1) {
                    if next.is_ascii_digit() {
                        is_float = true;
                        value.push(c);
                        self.advance();
                        continue;
                    }
                }
                break;
            } else {
                break;
            }
        }
        if is_float {
            TokenKind::FloatLit(value.parse().unwrap_or(0.0))
        } else {
            TokenKind::IntLit(value.parse().unwrap_or(0))
        }
    }

    fn read_ident(&mut self, first: char) -> TokenKind {
        let mut value = first.to_string();
        while let Some(&c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                value.push(c);
                self.advance();
            } else {
                break;
            }
        }
        TokenKind::Ident(value)
    }

    pub fn next_token(&mut self) -> Result<Token, String> {
        loop {
            self.current.clear();
            self.skip_whitespace();
            self.current.clear();
            let c = match self.peek() {
                Some(&c) => c,
                None => {
                    return Ok(Token {
                        kind: TokenKind::Eof,
                        text: "".to_string(),
                        span: crate::Span {
                            line: self.line as u32,
                            col: self.col as u32,
                        },
                    });
                }
            };
            if c == '#' {
                if self.chars.clone().nth(1) == Some('#') {
                    let start_line = self.line;
                    let start_col = self.col;
                    let text = self.read_doc_comment();
                    return Ok(Token {
                        kind: TokenKind::DocComment(text),
                        text: self.current.clone(),
                        span: crate::Span {
                            line: start_line as u32,
                            col: start_col as u32,
                        },
                    });
                }
                let start_line = self.line;
                let start_col = self.col;
                let text = self.read_line_comment();
                return Ok(Token {
                    kind: TokenKind::Comment(text),
                    text: self.current.clone(),
                    span: crate::Span {
                        line: start_line as u32,
                        col: start_col as u32,
                    },
                });
            }
            let start_line = self.line;
            let start_col = self.col;
            let kind = match c {
                'f' => {
                    self.advance();
                    if self.peek() == Some(&'"') {
                        let value = self.read_string('f')?;
                        TokenKind::FStringLit(value)
                    } else {
                        let mut name = "f".to_string();
                        while let Some(&c2) = self.peek() {
                            if c2.is_alphanumeric() || c2 == '_' {
                                name.push(c2);
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        Self::keyword_or_ident(name)
                    }
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    self.advance();
                    let kind = self.read_ident(c);
                    if let TokenKind::Ident(name) = kind {
                        Self::keyword_or_ident(name)
                    } else {
                        kind
                    }
                }
                '0'..='9' => {
                    self.advance();
                    self.read_number(c)
                }
                '"' => {
                    let value = self.read_string(' ')?;
                    TokenKind::StringLit(value)
                }
                '+' => {
                    self.advance();
                    if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::PlusAssign
                    } else {
                        TokenKind::Plus
                    }
                }
                '-' => {
                    self.advance();
                    if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::MinusAssign
                    } else if self.peek() == Some(&'>') {
                        self.advance();
                        TokenKind::Arrow
                    } else {
                        TokenKind::Minus
                    }
                }
                '*' => {
                    self.advance();
                    if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::StarAssign
                    } else {
                        TokenKind::Star
                    }
                }
                '/' => {
                    self.advance();
                    if self.peek() == Some(&'/') {
                        self.advance();
                        TokenKind::SlashSlash
                    } else if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::SlashAssign
                    } else {
                        TokenKind::Slash
                    }
                }
                '%' => {
                    self.advance();
                    if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::PercentAssign
                    } else {
                        TokenKind::Percent
                    }
                }
                '=' => {
                    self.advance();
                    if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::EqEq
                    } else {
                        TokenKind::Assign
                    }
                }
                '!' => {
                    self.advance();
                    if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::NotEq
                    } else {
                        TokenKind::Bang
                    }
                }
                '<' => {
                    self.advance();
                    if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::LtEq
                    } else if self.peek() == Some(&'<') {
                        self.advance();
                        TokenKind::Shl
                    } else {
                        TokenKind::Lt
                    }
                }
                '>' => {
                    self.advance();
                    if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::GtEq
                    } else if self.peek() == Some(&'>') {
                        self.advance();
                        TokenKind::Shr
                    } else {
                        TokenKind::Gt
                    }
                }
                '&' => {
                    self.advance();
                    if self.peek() == Some(&'&') {
                        self.advance();
                        TokenKind::AndAnd
                    } else if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::AmpAssign
                    } else {
                        TokenKind::Amp
                    }
                }
                '|' => {
                    self.advance();
                    if self.peek() == Some(&'|') {
                        self.advance();
                        TokenKind::OrOr
                    } else if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::PipeAssign
                    } else {
                        TokenKind::Pipe
                    }
                }
                '^' => {
                    self.advance();
                    if self.peek() == Some(&'=') {
                        self.advance();
                        TokenKind::CaretAssign
                    } else {
                        TokenKind::Caret
                    }
                }
                '~' => {
                    self.advance();
                    TokenKind::Tilde
                }
                '(' => {
                    self.advance();
                    TokenKind::LParen
                }
                ')' => {
                    self.advance();
                    TokenKind::RParen
                }
                '{' => {
                    self.advance();
                    TokenKind::LBrace
                }
                '}' => {
                    self.advance();
                    TokenKind::RBrace
                }
                '[' => {
                    self.advance();
                    TokenKind::LBracket
                }
                ']' => {
                    self.advance();
                    TokenKind::RBracket
                }
                ':' => {
                    self.advance();
                    TokenKind::Colon
                }
                ';' => {
                    self.advance();
                    TokenKind::Semicolon
                }
                '.' => {
                    self.advance();
                    if self.peek() == Some(&'.') {
                        self.advance();
                        if self.peek() == Some(&'=') {
                            self.advance();
                            TokenKind::RangeInclusive
                        } else {
                            TokenKind::Range
                        }
                    } else {
                        TokenKind::Dot
                    }
                }
                ',' => {
                    self.advance();
                    TokenKind::Comma
                }
                '?' => {
                    self.advance();
                    TokenKind::Question
                }
                '@' => {
                    self.advance();
                    TokenKind::At
                }
                _ => {
                    return Err(format!(
                        "unexpected character {:?} at {}:{}",
                        c, self.line, self.col
                    ));
                }
            };
            let text = self.current.clone();
            return Ok(Token {
                kind,
                text,
                span: crate::Span {
                    line: start_line as u32,
                    col: start_col as u32,
                },
            });
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let is_eof = token.kind == TokenKind::Eof;
            tokens.push(token);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_comment_vs_line_comment() {
        let tokens = Lexer::new("## hello\n# skipped\nfn\n").tokenize().unwrap();
        assert!(
            matches!(&tokens[0].kind, TokenKind::DocComment(s) if s == "hello"),
            "{:?}",
            tokens[0].kind
        );
        assert!(
            matches!(&tokens[1].kind, TokenKind::Comment(s) if s == "skipped"),
            "{:?}",
            tokens[1].kind
        );
        assert_eq!(tokens[2].kind, TokenKind::Fn);
    }

    #[test]
    fn doc_comment_keeps_inner_hash() {
        let tokens = Lexer::new("## # still docs\n").tokenize().unwrap();
        assert!(
            matches!(&tokens[0].kind, TokenKind::DocComment(s) if s == "# still docs"),
            "{:?}",
            tokens[0].kind
        );
    }

    #[test]
    fn bitwise_tokens() {
        let kinds: Vec<_> = Lexer::new("1 << 3 >> 1 | 2 & 4 ^ ~0 && true || false\n")
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .collect();
        assert!(matches!(kinds[1], TokenKind::Shl));
        assert!(matches!(kinds[3], TokenKind::Shr));
        assert!(matches!(kinds[5], TokenKind::Pipe));
        assert!(matches!(kinds[7], TokenKind::Amp));
        assert!(matches!(kinds[9], TokenKind::Caret));
        assert!(matches!(kinds[10], TokenKind::Tilde));
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::AndAnd)));
        assert!(kinds.iter().any(|k| matches!(k, TokenKind::OrOr)));
    }
}
