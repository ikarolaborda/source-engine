//! Bounded parser and deterministic writer for Source 1 text KeyValues.
//!
//! The representation deliberately preserves ordering, duplicate keys,
//! conditionals, and top-level `#include`/`#base` directives. Resolving those
//! directives is a filesystem policy and is intentionally left to callers.

use std::fmt;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_input_bytes: usize,
    pub max_token_bytes: usize,
    pub max_items: usize,
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_token_bytes: 64 * 1024,
            max_items: 1_000_000,
            max_depth: 128,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ParseOptions {
    /// Match `KeyValues::UsesEscapeSequences`. The legacy default is false.
    pub escape_sequences: bool,
    /// Accept a block the input ends inside, keeping the keys read so far,
    /// which is what `RecursiveLoadFromBuffer` does on reaching EOF early.
    /// Off by default so a truncated file is refused rather than silently
    /// half-read; callers that must load content as shipped turn it on.
    pub keep_unterminated_blocks: bool,
    pub limits: Limits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    String(String),
    Object(Vec<Node>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub value: Value,
    /// The original bracketed condition, for example `[$POSIX]`.
    pub condition: Option<String>,
}

impl Node {
    pub fn string(&self) -> Option<&str> {
        match &self.value {
            Value::String(value) => Some(value),
            Value::Object(_) => None,
        }
    }

    pub fn children(&self) -> Option<&[Node]> {
        match &self.value {
            Value::String(_) => None,
            Value::Object(children) => Some(children),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Node(Node),
    Include(String),
    Base(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub items: Vec<Item>,
}

impl Document {
    pub fn roots(&self) -> impl Iterator<Item = &Node> {
        self.items.iter().filter_map(|item| match item {
            Item::Node(node) => Some(node),
            Item::Include(_) | Item::Base(_) => None,
        })
    }

    pub fn to_canonical_string(&self) -> String {
        let mut output = String::new();
        for item in &self.items {
            match item {
                Item::Node(node) => write_node(&mut output, node, 0),
                Item::Include(path) => {
                    output.push_str("#include\t");
                    write_quoted(&mut output, path);
                    output.push('\n');
                }
                Item::Base(path) => {
                    output.push_str("#base\t");
                    write_quoted(&mut output, path);
                    output.push('\n');
                }
            }
        }
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    InputTooLarge { size: usize, limit: usize },
    InvalidUtf8,
    InvalidUtf16,
    EmbeddedNul,
    TokenTooLarge { size: usize, limit: usize },
    ItemLimitExceeded { limit: usize },
    DepthLimitExceeded { limit: usize },
    UnterminatedString,
    UnterminatedComment,
    InvalidEscape(char),
    UnexpectedEnd(&'static str),
    UnexpectedToken(&'static str),
    EmptyName,
    TrailingCloseBrace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub kind: ErrorKind,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyValues error at byte {}: ", self.offset)?;
        match &self.kind {
            ErrorKind::InputTooLarge { size, limit } => {
                write!(f, "input size {size} exceeds limit {limit}")
            }
            ErrorKind::InvalidUtf8 => write!(f, "input is not valid UTF-8"),
            ErrorKind::InvalidUtf16 => write!(f, "input is not valid UTF-16LE"),
            ErrorKind::EmbeddedNul => write!(f, "input contains an embedded NUL"),
            ErrorKind::TokenTooLarge { size, limit } => {
                write!(f, "token size {size} exceeds limit {limit}")
            }
            ErrorKind::ItemLimitExceeded { limit } => {
                write!(f, "item count exceeds limit {limit}")
            }
            ErrorKind::DepthLimitExceeded { limit } => {
                write!(f, "nesting depth exceeds limit {limit}")
            }
            ErrorKind::UnterminatedString => write!(f, "unterminated quoted string"),
            ErrorKind::UnterminatedComment => write!(f, "unterminated block comment"),
            ErrorKind::InvalidEscape(value) => write!(f, "unsupported escape sequence \\{value}"),
            ErrorKind::UnexpectedEnd(expected) => {
                write!(f, "unexpected end while expecting {expected}")
            }
            ErrorKind::UnexpectedToken(expected) => write!(f, "expected {expected}"),
            ErrorKind::EmptyName => write!(f, "empty key name"),
            ErrorKind::TrailingCloseBrace => write!(f, "unmatched closing brace"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn parse(text: &str) -> Result<Document> {
    parse_with_options(text, ParseOptions::default())
}

pub fn parse_with_options(text: &str, options: ParseOptions) -> Result<Document> {
    if text.len() > options.limits.max_input_bytes {
        return Err(Error {
            offset: 0,
            kind: ErrorKind::InputTooLarge {
                size: text.len(),
                limit: options.limits.max_input_bytes,
            },
        });
    }
    if let Some(offset) = text.as_bytes().iter().position(|byte| *byte == 0) {
        return Err(Error {
            offset,
            kind: ErrorKind::EmbeddedNul,
        });
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    Parser::new(text, options).document()
}

/// Parses UTF-8 (with an optional BOM) or the UTF-16LE BOM accepted by the
/// legacy loader.
pub fn parse_bytes(bytes: &[u8], options: ParseOptions) -> Result<Document> {
    if bytes.len() > options.limits.max_input_bytes {
        return Err(Error {
            offset: 0,
            kind: ErrorKind::InputTooLarge {
                size: bytes.len(),
                limit: options.limits.max_input_bytes,
            },
        });
    }
    if bytes.starts_with(&[0xff, 0xfe]) {
        let body = &bytes[2..];
        if body.len() % 2 != 0 {
            return Err(Error {
                offset: bytes.len() - 1,
                kind: ErrorKind::InvalidUtf16,
            });
        }
        let units = body
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
        let mut text = String::new();
        for character in char::decode_utf16(units) {
            text.push(character.map_err(|_| Error {
                offset: 2,
                kind: ErrorKind::InvalidUtf16,
            })?);
        }
        parse_with_options(&text, options)
    } else {
        let text = std::str::from_utf8(bytes).map_err(|error| Error {
            offset: error.valid_up_to(),
            kind: ErrorKind::InvalidUtf8,
        })?;
        parse_with_options(text, options)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TokenKind {
    Word(String),
    Condition(String),
    Open,
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    offset: usize,
    kind: TokenKind,
}

struct Lexer<'a> {
    text: &'a str,
    position: usize,
    options: ParseOptions,
}

impl Lexer<'_> {
    fn next(&mut self) -> Result<Option<Token>> {
        self.skip_trivia()?;
        if self.position == self.text.len() {
            return Ok(None);
        }
        let offset = self.position;
        let first = self.text.as_bytes()[self.position];
        if first == b'{' || first == b'}' {
            self.position += 1;
            return Ok(Some(Token {
                offset,
                kind: if first == b'{' {
                    TokenKind::Open
                } else {
                    TokenKind::Close
                },
            }));
        }
        if first == b'"' {
            return self.quoted(offset).map(Some);
        }
        self.unquoted(offset).map(Some)
    }

    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            while self
                .text
                .as_bytes()
                .get(self.position)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.position += 1;
            }
            let rest = &self.text.as_bytes()[self.position..];
            if rest.starts_with(b"//") {
                self.position += 2;
                while let Some(byte) = self.text.as_bytes().get(self.position) {
                    self.position += 1;
                    if *byte == b'\n' {
                        break;
                    }
                }
                continue;
            }
            if rest.starts_with(b"/*") {
                let start = self.position;
                self.position += 2;
                let Some(end) = self.text.as_bytes()[self.position..]
                    .windows(2)
                    .position(|pair| pair == b"*/")
                else {
                    return Err(Error {
                        offset: start,
                        kind: ErrorKind::UnterminatedComment,
                    });
                };
                self.position += end + 2;
                continue;
            }
            return Ok(());
        }
    }

    fn quoted(&mut self, offset: usize) -> Result<Token> {
        self.position += 1;
        let mut value = String::new();
        loop {
            let Some(character) = self.text[self.position..].chars().next() else {
                return Err(Error {
                    offset,
                    kind: ErrorKind::UnterminatedString,
                });
            };
            self.position += character.len_utf8();
            if character == '"' {
                break;
            }
            if character == '\\' && self.options.escape_sequences {
                let Some(escaped) = self.text[self.position..].chars().next() else {
                    return Err(Error {
                        offset,
                        kind: ErrorKind::UnterminatedString,
                    });
                };
                self.position += escaped.len_utf8();
                value.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '\\' => '\\',
                    '"' => '"',
                    other => {
                        return Err(Error {
                            offset: self.position - other.len_utf8() - 1,
                            kind: ErrorKind::InvalidEscape(other),
                        });
                    }
                });
            } else {
                value.push(character);
            }
            self.check_token_size(offset, value.len())?;
        }
        Ok(Token {
            offset,
            kind: TokenKind::Word(value),
        })
    }

    fn unquoted(&mut self, offset: usize) -> Result<Token> {
        let start = self.position;
        while let Some(byte) = self.text.as_bytes().get(self.position) {
            if byte.is_ascii_whitespace() || matches!(*byte, b'"' | b'{' | b'}') {
                break;
            }
            self.position += 1;
            self.check_token_size(offset, self.position - start)?;
        }
        let value = &self.text[start..self.position];
        let kind = if value.starts_with('[') && value.ends_with(']') {
            TokenKind::Condition(value.to_owned())
        } else {
            TokenKind::Word(value.to_owned())
        };
        Ok(Token { offset, kind })
    }

    fn check_token_size(&self, offset: usize, size: usize) -> Result<()> {
        if size > self.options.limits.max_token_bytes {
            Err(Error {
                offset,
                kind: ErrorKind::TokenTooLarge {
                    size,
                    limit: self.options.limits.max_token_bytes,
                },
            })
        } else {
            Ok(())
        }
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    lookahead: Option<Token>,
    item_count: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str, options: ParseOptions) -> Self {
        Self {
            lexer: Lexer {
                text,
                position: 0,
                options,
            },
            lookahead: None,
            item_count: 0,
        }
    }

    fn document(mut self) -> Result<Document> {
        let mut items = Vec::new();
        while let Some(token) = self.next()? {
            let (offset, name) = word(token, "a root name or directive")?;
            if name.eq_ignore_ascii_case("#include") || name.eq_ignore_ascii_case("#base") {
                let path = self.expect_word("an include path")?.1;
                self.count_item(offset)?;
                if name.eq_ignore_ascii_case("#include") {
                    items.push(Item::Include(path));
                } else {
                    items.push(Item::Base(path));
                }
                continue;
            }
            let condition = self.optional_condition()?;
            self.expect_open()?;
            let children = self.object(1)?;
            self.count_item(offset)?;
            items.push(Item::Node(Node {
                name,
                value: Value::Object(children),
                condition,
            }));
        }
        Ok(Document { items })
    }

    fn object(&mut self, depth: usize) -> Result<Vec<Node>> {
        if depth > self.lexer.options.limits.max_depth {
            return Err(Error {
                offset: self.lexer.position,
                kind: ErrorKind::DepthLimitExceeded {
                    limit: self.lexer.options.limits.max_depth,
                },
            });
        }
        let mut children = Vec::new();
        loop {
            let Some(token) = self.next()? else {
                // The legacy loader reports an unclosed block to its error
                // stack and then keeps the keys it already read, and shipped
                // content relies on that: one of Half-Life 2's materials is
                // a closing brace short and still loads in the game.
                if self.lexer.options.keep_unterminated_blocks {
                    return Ok(children);
                }
                return Err(Error {
                    offset: self.lexer.position,
                    kind: ErrorKind::UnexpectedEnd("a closing brace"),
                });
            };
            if token.kind == TokenKind::Close {
                return Ok(children);
            }
            let (offset, name) = word(token, "a key name")?;
            if name.is_empty() {
                return Err(Error {
                    offset,
                    kind: ErrorKind::EmptyName,
                });
            }
            let before_value = self.optional_condition()?;
            let value_token = self.next()?.ok_or(Error {
                offset: self.lexer.position,
                kind: ErrorKind::UnexpectedEnd("a value or opening brace"),
            })?;
            let (value, after_value) = match value_token.kind {
                TokenKind::Open => (Value::Object(self.object(depth + 1)?), None),
                TokenKind::Word(value) => {
                    let condition = self.optional_condition()?;
                    (Value::String(value), condition)
                }
                TokenKind::Condition(_) | TokenKind::Close => {
                    return Err(Error {
                        offset: value_token.offset,
                        kind: ErrorKind::UnexpectedToken("a value or opening brace"),
                    });
                }
            };
            if before_value.is_some() && after_value.is_some() {
                return Err(Error {
                    offset,
                    kind: ErrorKind::UnexpectedToken("at most one conditional per key"),
                });
            }
            self.count_item(offset)?;
            children.push(Node {
                name,
                value,
                condition: before_value.or(after_value),
            });
        }
    }

    fn optional_condition(&mut self) -> Result<Option<String>> {
        match self.peek()? {
            Some(Token {
                kind: TokenKind::Condition(_),
                ..
            }) => match self.next()?.expect("peeked token must exist").kind {
                TokenKind::Condition(value) => Ok(Some(value)),
                _ => unreachable!(),
            },
            _ => Ok(None),
        }
    }

    fn expect_open(&mut self) -> Result<()> {
        let token = self.next()?.ok_or(Error {
            offset: self.lexer.position,
            kind: ErrorKind::UnexpectedEnd("an opening brace"),
        })?;
        if token.kind != TokenKind::Open {
            return Err(Error {
                offset: token.offset,
                kind: if token.kind == TokenKind::Close {
                    ErrorKind::TrailingCloseBrace
                } else {
                    ErrorKind::UnexpectedToken("an opening brace")
                },
            });
        }
        Ok(())
    }

    fn expect_word(&mut self, expected: &'static str) -> Result<(usize, String)> {
        let token = self.next()?.ok_or(Error {
            offset: self.lexer.position,
            kind: ErrorKind::UnexpectedEnd(expected),
        })?;
        word(token, expected)
    }

    fn peek(&mut self) -> Result<Option<&Token>> {
        if self.lookahead.is_none() {
            self.lookahead = self.lexer.next()?;
        }
        Ok(self.lookahead.as_ref())
    }

    fn next(&mut self) -> Result<Option<Token>> {
        if self.lookahead.is_some() {
            Ok(self.lookahead.take())
        } else {
            self.lexer.next()
        }
    }

    fn count_item(&mut self, offset: usize) -> Result<()> {
        self.item_count += 1;
        if self.item_count > self.lexer.options.limits.max_items {
            Err(Error {
                offset,
                kind: ErrorKind::ItemLimitExceeded {
                    limit: self.lexer.options.limits.max_items,
                },
            })
        } else {
            Ok(())
        }
    }
}

fn word(token: Token, expected: &'static str) -> Result<(usize, String)> {
    match token.kind {
        TokenKind::Word(value) => Ok((token.offset, value)),
        TokenKind::Close => Err(Error {
            offset: token.offset,
            kind: ErrorKind::TrailingCloseBrace,
        }),
        TokenKind::Condition(_) | TokenKind::Open => Err(Error {
            offset: token.offset,
            kind: ErrorKind::UnexpectedToken(expected),
        }),
    }
}

fn write_node(output: &mut String, node: &Node, depth: usize) {
    write_indent(output, depth);
    write_quoted(output, &node.name);
    if let Some(condition) = &node.condition {
        output.push('\t');
        output.push_str(condition);
    }
    match &node.value {
        Value::String(value) => {
            output.push('\t');
            write_quoted(output, value);
            output.push('\n');
        }
        Value::Object(children) => {
            output.push('\n');
            write_indent(output, depth);
            output.push_str("{\n");
            for child in children {
                write_node(output, child, depth + 1);
            }
            write_indent(output, depth);
            output.push_str("}\n");
        }
    }
}

fn write_indent(output: &mut String, depth: usize) {
    for _ in 0..depth {
        output.push('\t');
    }
}

fn write_quoted(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            other => output.push(other),
        }
    }
    output.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escaped_options() -> ParseOptions {
        ParseOptions {
            escape_sequences: true,
            ..ParseOptions::default()
        }
    }

    #[test]
    fn preserves_duplicates_order_directives_and_conditions() {
        let input = r#"
            // material comment
            #base "base.vmt"
            "LightmappedGeneric" [$POSIX]
            {
                "$basetexture" "brick/wall"
                "$surfaceprop" concrete
                "$surfaceprop" "brick" [!$X360]
                "Proxies" { "AnimatedTexture" { "rate" "2" } }
            }
            #include extras.vmt
        "#;
        let document = parse(input).unwrap();
        assert_eq!(document.items.len(), 3);
        assert_eq!(document.roots().count(), 1);
        let root = document.roots().next().unwrap();
        assert_eq!(root.name, "LightmappedGeneric");
        assert_eq!(root.condition.as_deref(), Some("[$POSIX]"));
        let children = root.children().unwrap();
        assert_eq!(children.len(), 4);
        assert_eq!(children[1].name, "$surfaceprop");
        assert_eq!(children[2].name, "$surfaceprop");
        assert_eq!(children[2].condition.as_deref(), Some("[!$X360]"));
    }

    #[test]
    fn canonical_output_round_trips_with_escapes() {
        let input = "\"root\" { \"message\" \"line\\n\\\"quoted\\\"\" }";
        let first = parse_with_options(input, escaped_options()).unwrap();
        let encoded = first.to_canonical_string();
        let second = parse_with_options(&encoded, escaped_options()).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first.roots().next().unwrap().children().unwrap()[0].string(),
            Some("line\n\"quoted\"")
        );
    }

    #[test]
    fn accepts_utf8_and_utf16_boms() {
        let utf8 = b"\xef\xbb\xbf\"root\" { \"key\" \"value\" }";
        assert_eq!(
            parse_bytes(utf8, ParseOptions::default())
                .unwrap()
                .roots()
                .count(),
            1
        );

        let text = "\"root\" { \"key\" \"value\" }";
        let mut utf16 = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(
            parse_bytes(&utf16, ParseOptions::default())
                .unwrap()
                .roots()
                .count(),
            1
        );
    }

    #[test]
    fn keeps_an_unterminated_block_only_when_asked_to() {
        let truncated = "\"root\" { \"key\" \"value\" \"nested\" { \"inner\" \"1\" }";

        // Strict by default: a file that ends mid-block is truncated, and
        // silently returning half of it would hide that.
        let error = parse(truncated).expect_err("a truncated document is refused");
        assert!(matches!(
            error.kind,
            ErrorKind::UnexpectedEnd("a closing brace")
        ));

        // The legacy loader reports the same thing and then keeps what it
        // read, which is what loading content as shipped requires.
        let document = parse_with_options(
            truncated,
            ParseOptions {
                keep_unterminated_blocks: true,
                ..Default::default()
            },
        )
        .expect("the keys read so far are kept");

        let root = document.roots().next().expect("the root was kept");
        let children = root.children().expect("with its keys");
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].string(), Some("value"));
        assert_eq!(
            children[1]
                .children()
                .and_then(|nested| nested.first())
                .and_then(Node::string),
            Some("1"),
            "the block that did close is complete"
        );
    }

    #[test]
    fn rejects_malformed_and_limited_inputs() {
        assert!(matches!(
            parse("\"root\" { \"key\" \"unterminated }"),
            Err(Error {
                kind: ErrorKind::UnterminatedString,
                ..
            })
        ));
        let options = ParseOptions {
            limits: Limits {
                max_depth: 1,
                ..Limits::default()
            },
            ..ParseOptions::default()
        };
        assert!(matches!(
            parse_with_options("root { child { key value } }", options),
            Err(Error {
                kind: ErrorKind::DepthLimitExceeded { .. },
                ..
            })
        ));
        let options = ParseOptions {
            limits: Limits {
                max_items: 1,
                ..Limits::default()
            },
            ..ParseOptions::default()
        };
        assert!(matches!(
            parse_with_options("root { a b c d }", options),
            Err(Error {
                kind: ErrorKind::ItemLimitExceeded { .. },
                ..
            })
        ));
    }
}
