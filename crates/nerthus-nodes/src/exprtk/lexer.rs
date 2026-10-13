//! ExprTk's lexer (`exprtk::lexer::generator`) and the token passes its
//! parser runs with default settings: `true`/`false` replaced by numbers,
//! split operators joined (`- -` becomes `+`), implicit multiplication
//! inserted (`2x`, `a b`), then numbers and brackets checked.

/// `lexer::token::token_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Error,
    Eof,
    Number,
    Symbol,
    Str,
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    Shr,
    Shl,
    Lte,
    Ne,
    Gte,
    Swap,
    Lt,
    Gt,
    Eq,
    RightBracket,
    LeftBracket,
    RightSquare,
    LeftSquare,
    RightCurly,
    LeftCurly,
    Comma,
    Add,
    Sub,
    Div,
    Mul,
    Mod,
    Pow,
    Colon,
    Ternary,
    /// `!` on its own (only valid in `!=`).
    Bang,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub value: String,
    pub position: usize,
}

impl Token {
    fn new(kind: TokenKind, value: impl Into<String>, position: usize) -> Token {
        Token { kind, value: value.into(), position }
    }
}

fn is_operator_char(character: char) -> bool {
    "+-*/^<>=,!()[]{}%:?&|;".contains(character)
}

fn is_letter(character: char) -> bool {
    character.is_ascii_alphabetic()
}

fn is_digit(character: char) -> bool {
    character.is_ascii_digit()
}

fn is_letter_or_digit(character: char) -> bool {
    is_letter(character) || is_digit(character)
}

/// `generator::process`: the expression's raw tokens.
fn scan(expression: &[char]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut index = 0;
    let at = |index: usize| expression.get(index).copied().unwrap_or('\0');
    while index < expression.len() {
        let character = expression[index];
        if character.is_whitespace() {
            index += 1;
            continue;
        }
        let comment_mode = match (character, at(index + 1)) {
            ('#', _) => Some((1, 1)),
            ('/', '/') => Some((1, 2)),
            ('/', '*') => Some((2, 2)),
            _ => None,
        };
        if let Some((mode, skip)) = comment_mode {
            index += skip;
            while index < expression.len() {
                if mode == 1 && expression[index] == '\n' {
                    index += 1;
                    break;
                }
                if mode == 2 && expression[index] == '*' && at(index + 1) == '/' {
                    index += 2;
                    break;
                }
                index += 1;
            }
            continue;
        }
        if is_operator_char(character) {
            index = scan_operator(expression, index, &mut tokens);
            continue;
        }
        if is_letter(character) {
            index = scan_symbol(expression, index, &mut tokens);
            continue;
        }
        if character == '.' && !is_digit(at(index + 1)) {
            index = scan_operator(expression, index, &mut tokens);
            continue;
        }
        if is_digit(character) || character == '.' {
            index = scan_number(expression, index, &mut tokens);
            continue;
        }
        if character == '\'' {
            index = scan_string(expression, index, &mut tokens);
            continue;
        }
        if character == '$' {
            index = scan_special_function(expression, index, &mut tokens);
            continue;
        }
        if character == '~' {
            tokens.push(Token::new(TokenKind::Symbol, "~", index));
            index += 1;
            continue;
        }
        let text: String = expression[index..(index + 2).min(expression.len())].iter().collect();
        tokens.push(Token::new(TokenKind::Error, text, index));
        index += 1;
    }
    tokens
}

fn scan_operator(expression: &[char], index: usize, tokens: &mut Vec<Token>) -> usize {
    let at = |offset: usize| expression.get(index + offset).copied();
    let first = expression[index];
    if at(0) == Some('<') && at(1) == Some('=') && at(2) == Some('>') {
        tokens.push(Token::new(TokenKind::Swap, "<=>", index));
        return index + 3;
    }
    if let Some(second) = at(1) {
        let kind = match (first, second) {
            ('<', '=') => Some(TokenKind::Lte),
            ('>', '=') => Some(TokenKind::Gte),
            ('<', '>') => Some(TokenKind::Ne),
            ('!', '=') => Some(TokenKind::Ne),
            ('=', '=') => Some(TokenKind::Eq),
            (':', '=') => Some(TokenKind::Assign),
            ('<', '<') => Some(TokenKind::Shl),
            ('>', '>') => Some(TokenKind::Shr),
            ('+', '=') => Some(TokenKind::AddAssign),
            ('-', '=') => Some(TokenKind::SubAssign),
            ('*', '=') => Some(TokenKind::MulAssign),
            ('/', '=') => Some(TokenKind::DivAssign),
            ('%', '=') => Some(TokenKind::ModAssign),
            _ => None,
        };
        if let Some(kind) = kind {
            let text: String = expression[index..index + 2].iter().collect();
            tokens.push(Token::new(kind, text, index));
            return index + 2;
        }
    }
    let kind = match first {
        '<' => TokenKind::Lt,
        '>' => TokenKind::Gt,
        ';' => TokenKind::Eof,
        '&' | '|' => TokenKind::Symbol,
        '=' => TokenKind::Eq,
        ')' => TokenKind::RightBracket,
        '(' => TokenKind::LeftBracket,
        ']' => TokenKind::RightSquare,
        '[' => TokenKind::LeftSquare,
        '}' => TokenKind::RightCurly,
        '{' => TokenKind::LeftCurly,
        ',' => TokenKind::Comma,
        '+' => TokenKind::Add,
        '-' => TokenKind::Sub,
        '/' => TokenKind::Div,
        '*' => TokenKind::Mul,
        '%' => TokenKind::Mod,
        '^' => TokenKind::Pow,
        ':' => TokenKind::Colon,
        '?' => TokenKind::Ternary,
        '!' => TokenKind::Bang,
        _ => TokenKind::Error,
    };
    tokens.push(Token::new(kind, first.to_string(), index));
    index + 1
}

/// Letters, digits, `_`, and dots inside a name (`abc.xyz`).
fn scan_symbol(expression: &[char], start: usize, tokens: &mut Vec<Token>) -> usize {
    let mut index = start;
    while index < expression.len() {
        let character = expression[index];
        if !is_letter_or_digit(character) && character != '_' {
            if character != '.' {
                break;
            }
            let next = expression.get(index + 1).copied();
            let dot_ends_name = index != start && next.is_some_and(|next| !is_letter_or_digit(next) && next != '_');
            if dot_ends_name {
                break;
            }
        }
        index += 1;
    }
    let text: String = expression[start..index].iter().collect();
    tokens.push(Token::new(TokenKind::Symbol, text, start));
    index
}

/// `123`, `1.5`, `.5`, `1e3`, `1.5E-3`; malformed numbers become errors.
fn scan_number(expression: &[char], start: usize, tokens: &mut Vec<Token>) -> usize {
    let mut index = start;
    let mut dot_found = false;
    let mut e_found = false;
    let mut post_e_sign_found = false;
    let mut post_e_digit_found = false;
    let error = |index: usize, tokens: &mut Vec<Token>| {
        let text: String = expression[start..index].iter().collect();
        tokens.push(Token::new(TokenKind::Error, text, start));
        index
    };
    while index < expression.len() {
        let character = expression[index];
        if character == '.' {
            if dot_found {
                return error(index, tokens);
            }
            dot_found = true;
            index += 1;
            continue;
        }
        if character.eq_ignore_ascii_case(&'e') {
            let Some(&next) = expression.get(index + 1) else {
                return error(index, tokens);
            };
            if next != '+' && next != '-' && !is_digit(next) {
                return error(index, tokens);
            }
            e_found = true;
            index += 1;
            continue;
        }
        if e_found && (character == '+' || character == '-') && !post_e_digit_found {
            if post_e_sign_found {
                return error(index, tokens);
            }
            post_e_sign_found = true;
            index += 1;
            continue;
        }
        if e_found && is_digit(character) {
            post_e_digit_found = true;
            index += 1;
            continue;
        }
        if character != '.' && !is_digit(character) {
            break;
        }
        index += 1;
    }
    let text: String = expression[start..index].iter().collect();
    tokens.push(Token::new(TokenKind::Number, text, start));
    index
}

/// `'text'` with `\` escapes kept for the parser.
fn scan_string(expression: &[char], start: usize, tokens: &mut Vec<Token>) -> usize {
    let mut index = start + 1;
    let mut escaped = false;
    while index < expression.len() {
        let character = expression[index];
        if !escaped && character == '\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if !escaped && character == '\'' {
            break;
        }
        escaped = false;
        index += 1;
    }
    if index >= expression.len() {
        let text: String = expression[start..].iter().collect();
        tokens.push(Token::new(TokenKind::Error, text, start));
        return expression.len();
    }
    let text: String = expression[start + 1..index].iter().collect();
    tokens.push(Token::new(TokenKind::Str, text, start));
    index + 1
}

/// `$fNN` special functions.
fn scan_special_function(expression: &[char], start: usize, tokens: &mut Vec<Token>) -> usize {
    let end = start + 4;
    let valid = end <= expression.len()
        && expression[start + 1].eq_ignore_ascii_case(&'f')
        && is_digit(expression[start + 2])
        && is_digit(expression[start + 3]);
    if !valid {
        let text: String = expression[start..end.min(expression.len())].iter().collect();
        tokens.push(Token::new(TokenKind::Error, text, start));
        return end.min(expression.len());
    }
    let text: String = expression[start..end].iter().collect();
    tokens.push(Token::new(TokenKind::Symbol, text, start));
    end
}

/// `operator_joiner(2)`: one pass over adjacent pairs.
fn join_pairs(tokens: Vec<Token>) -> Vec<Token> {
    if tokens.len() < 2 {
        return tokens;
    }
    let join = |first: &Token, second: &Token| -> Option<Token> {
        use TokenKind::*;
        let kind = match (first.kind, second.kind) {
            (Colon, Eq) => Assign,
            (Add, Eq) => AddAssign,
            (Sub, Eq) => SubAssign,
            (Mul, Eq) => MulAssign,
            (Div, Eq) => DivAssign,
            (Mod, Eq) => ModAssign,
            (Gt, Eq) => Gte,
            (Lt, Eq) => Lte,
            (Eq, Eq) => Eq,
            (Bang, Eq) => Ne,
            (Lt, Gt) => Ne,
            (Lte, Gt) => Swap,
            (Add, Sub) | (Sub, Add) => Sub,
            (Sub, Sub) => Add,
            _ => return None,
        };
        let value = match kind {
            Assign => ":=",
            AddAssign => "+=",
            SubAssign => "-=",
            MulAssign => "*=",
            DivAssign => "/=",
            ModAssign => "%=",
            Gte => ">=",
            Lte => "<=",
            Eq => "==",
            Ne => "!=",
            Swap => "<=>",
            Sub => "-",
            _ => "+",
        };
        Some(Token::new(kind, value, first.position))
    };
    let mut joined = Vec::with_capacity(tokens.len());
    let last = tokens.len() - 1;
    let mut index = 0;
    while index < last {
        loop {
            match join(&tokens[index], &tokens[index + 1]) {
                None => {
                    joined.push(tokens[index].clone());
                    break;
                }
                Some(token) => {
                    joined.push(token);
                    index += 2;
                    if index >= last {
                        break;
                    }
                }
            }
        }
        index += 1;
    }
    // ExprTk always appends the last token, even when the last join used
    // it.
    joined.push(tokens[last].clone());
    joined
}

/// `operator_joiner(3)`: `[*]` becomes a symbol.
fn join_triples(tokens: Vec<Token>) -> Vec<Token> {
    if tokens.len() < 3 {
        return tokens;
    }
    let mut joined = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        let is_star = index + 2 < tokens.len()
            && tokens[index].kind == TokenKind::LeftSquare
            && tokens[index + 1].kind == TokenKind::Mul
            && tokens[index + 2].kind == TokenKind::RightSquare;
        if is_star {
            joined.push(Token::new(TokenKind::Symbol, "[*]", tokens[index].position));
            index += 3;
            continue;
        }
        joined.push(tokens[index].clone());
        index += 1;
    }
    joined
}

/// ExprTk's reserved words, which the implicit multiplication skips.
const RESERVED_WORDS: &[&str] = &[
    "assert", "break", "case", "continue", "const", "default", "false", "for", "if", "else", "ilike", "in", "like", "and", "nand", "nor", "not", "null", "or",
    "repeat", "return", "shl", "shr", "swap", "switch", "true", "until", "var", "while", "xnor", "xor", "&", "|",
];

fn is_reserved(symbol: &str) -> bool {
    RESERVED_WORDS.iter().any(|word| word.eq_ignore_ascii_case(symbol))
}

/// `commutative_inserter`: `*` between a number or closing bracket and a
/// symbol, number or opening bracket (not before `(` after a symbol).
fn insert_multiplications(tokens: Vec<Token>) -> Vec<Token> {
    use TokenKind::*;
    let mut result = Vec::with_capacity(tokens.len() * 2);
    for index in 0..tokens.len() {
        result.push(tokens[index].clone());
        let Some(next) = tokens.get(index + 1) else {
            continue;
        };
        let current = &tokens[index];
        let skipped = (current.kind == Symbol && (is_reserved(&current.value) || current.value.starts_with('$')))
            || (next.kind == Symbol && is_reserved(&next.value));
        if skipped {
            continue;
        }
        let matches = matches!(
            (current.kind, next.kind),
            (Number, Symbol)
                | (Number, LeftBracket)
                | (Number, LeftCurly)
                | (Number, LeftSquare)
                | (Symbol, Number)
                | (RightBracket, Number)
                | (RightCurly, Number)
                | (RightSquare, Number)
                | (RightBracket, Symbol)
                | (RightCurly, Symbol)
                | (RightSquare, Symbol)
                | (Symbol, Symbol)
        );
        if matches {
            result.push(Token::new(Mul, "*", next.position));
        }
    }
    result
}

/// Why lexing failed (ExprTk's diagnostic).
#[derive(Clone, Debug, PartialEq)]
pub struct LexError(pub String);

/// The expression's tokens after ExprTk's default passes (in its order:
/// implicit multiplication, operator joins, `true`/`false`), checked by its
/// scanners. ExprTk keeps no trailing end token: the parser reads past the
/// list as end of input.
pub fn tokenize(expression: &str) -> Result<Vec<Token>, LexError> {
    if expression.is_empty() {
        return Err(LexError("ERR001 - Empty expression!".to_string()));
    }
    let characters: Vec<char> = expression.chars().collect();
    let mut tokens = scan(&characters);
    if let Some(bad) = tokens.iter().find(|token| token.kind == TokenKind::Error) {
        return Err(LexError(format!("ERR004 - Invalid token in expression: '{}'", bad.value)));
    }
    if tokens.is_empty() {
        return Err(LexError("ERR002 - Empty expression!".to_string()));
    }

    tokens = insert_multiplications(tokens);
    tokens = join_pairs(tokens);
    tokens = join_triples(tokens);

    // `symbol_replacer`: true and false become numbers.
    for token in tokens.iter_mut() {
        if token.kind != TokenKind::Symbol {
            continue;
        }
        let lowered = token.value.to_ascii_lowercase();
        if lowered == "true" || lowered == "false" {
            token.kind = TokenKind::Number;
            token.value = if lowered == "true" { "1" } else { "0" }.to_string();
        }
    }

    check_numbers(&tokens)?;
    check_brackets(&tokens)?;
    check_sequences(&tokens)?;
    check_triples(&tokens)?;
    Ok(tokens)
}

/// `numeric_checker`.
fn check_numbers(tokens: &[Token]) -> Result<(), LexError> {
    let bad = tokens.iter().find(|token| token.kind == TokenKind::Number && crate::exprtk::number::string_to_real(&token.value).is_none());
    match bad {
        Some(token) => Err(LexError(format!("ERR006 - Invalid numeric token: '{}'", token.value))),
        None => Ok(()),
    }
}

/// `bracket_checker`.
fn check_brackets(tokens: &[Token]) -> Result<(), LexError> {
    let mut stack = Vec::new();
    for token in tokens {
        let closing = match token.kind {
            TokenKind::LeftBracket => Some(')'),
            TokenKind::LeftCurly => Some('}'),
            TokenKind::LeftSquare => Some(']'),
            _ => None,
        };
        if let Some(closing) = closing {
            stack.push(closing);
            continue;
        }
        let close = match token.kind {
            TokenKind::RightBracket => ')',
            TokenKind::RightCurly => '}',
            TokenKind::RightSquare => ']',
            _ => continue,
        };
        if stack.pop() != Some(close) {
            return Err(LexError(format!("ERR005 - Mismatched brackets: '{}'", token.value)));
        }
    }
    if let Some(expected) = stack.last() {
        return Err(LexError(format!("ERR005 - Mismatched brackets: '{expected}'")));
    }
    Ok(())
}

fn is_left_bracket(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::LeftBracket | TokenKind::LeftSquare | TokenKind::LeftCurly)
}

fn is_right_bracket(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::RightBracket | TokenKind::RightSquare | TokenKind::RightCurly)
}

/// `sequence_validator::invalid_bracket_check`.
fn invalid_bracket_pair(base: TokenKind, next: TokenKind) -> bool {
    use TokenKind::*;
    if is_right_bracket(base) {
        return match next {
            Assign => base != RightSquare,
            Str => base != RightBracket,
            _ => false,
        };
    }
    if is_left_bracket(base) {
        if is_right_bracket(next) || is_left_bracket(next) {
            return false;
        }
        return !matches!(next, Number | Symbol | Str | Add | Sub | Colon | Ternary);
    }
    if is_right_bracket(next) {
        return !matches!(base, Number | Symbol | Str | Eof | Colon | Ternary);
    }
    if is_left_bracket(next) {
        return is_right_bracket(base);
    }
    false
}

/// `sequence_validator`'s invalid pairs.
fn invalid_pair(base: TokenKind, next: TokenKind) -> bool {
    use TokenKind::*;
    let literal_pair = matches!((base, next), (Number, Number) | (Str, Str) | (Number, Str) | (Str, Number));
    if literal_pair {
        return true;
    }
    let first_set = matches!(base, Assign | Shr | Shl | Lte | Ne | Gte | Lt | Gt | Eq | Comma | Add | Sub | Div | Mul | Mod | Pow | Colon | Ternary);
    let second_set = matches!(next, Assign | Shr | Shl | Lte | Ne | Gte | Lt | Gt | Eq | Comma | Div | Mul | Mod | Pow | Colon);
    first_set && second_set
}

/// `sequence_validator`.
fn check_sequences(tokens: &[Token]) -> Result<(), LexError> {
    for pair in tokens.windows(2) {
        if invalid_bracket_pair(pair[0].kind, pair[1].kind) || invalid_pair(pair[0].kind, pair[1].kind) {
            return Err(LexError(format!("ERR007 - Invalid token sequence: '{}' and '{}'", pair[0].value, pair[1].value)));
        }
    }
    Ok(())
}

/// `sequence_validator_3tokens`.
fn check_triples(tokens: &[Token]) -> Result<(), LexError> {
    use TokenKind::*;
    for triple in tokens.windows(3) {
        let kinds = (triple[0].kind, triple[1].kind, triple[2].kind);
        let repeated = kinds.0 == kinds.1 && kinds.1 == kinds.2 && matches!(kinds.0, Number | Str | Comma | Add | Sub | Div | Mul | Mod | Pow);
        let alternating = matches!(kinds, (Add, Sub, Add) | (Sub, Add, Sub) | (Div, Mul, Div) | (Mul, Div, Mul) | (Mod, Pow, Mod) | (Pow, Mod, Pow));
        if repeated || alternating {
            return Err(LexError(format!("ERR008 - Invalid token sequence: '{}' and '{}'", triple[0].value, triple[1].value)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(expression: &str) -> Vec<TokenKind> {
        tokenize(expression).unwrap().iter().map(|token| token.kind).collect()
    }

    #[test]
    fn implicit_multiplication_and_joins() {
        use TokenKind::*;
        assert_eq!(kinds("2x"), vec![Number, Mul, Symbol]);
        assert_eq!(kinds("a b"), vec![Symbol, Mul, Symbol]);
        assert_eq!(kinds("sin(x)"), vec![Symbol, LeftBracket, Symbol, RightBracket]);
        assert_eq!(kinds("x - -y"), vec![Symbol, Add, Symbol]);
        assert_eq!(kinds("x < = 2"), vec![Symbol, Lte, Number]);
        assert_eq!(kinds("x = 2"), vec![Symbol, Eq, Number]);
        assert_eq!(kinds("true + 1"), vec![Number, Add, Number]);
    }
}
