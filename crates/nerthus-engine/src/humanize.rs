//! Humanizer's string humanizing: property and enum member names become
//! the labels the interface shows (`ErosionScale` → `Erosion Scale`,
//! `LevelOfDetail` → `Level of Detail`).
//!
//! `StringHumanizeExtensions.Humanize` splits PascalCase with a regex that
//! uses lookahead, which the `regex` crate lacks; `pascal_case_part` walks
//! the same alternatives in the same order instead. Letter classes use
//! Rust's Unicode properties, which match .NET's exactly for ASCII (all
//! property and member names are ASCII).

/// Humanizer's `ToTitleCase` keeps these lower case after the first word.
const LOWER_CASE_WORDS: [&str; 21] = [
    "a", "an", "the", "and", "as", "but", "if", "nor", "or", "so", "yet", "at", "by", "for", "in", "of", "off", "on", "to", "up", "via",
];

fn is_upper(character: char) -> bool {
    character.is_uppercase()
}

fn is_lower(character: char) -> bool {
    character.is_lowercase()
}

/// `\p{Lo}`: letters without case (CJK, Hebrew, …).
fn is_other_letter(character: char) -> bool {
    character.is_alphabetic() && !character.is_uppercase() && !character.is_lowercase()
}

/// .NET's `\w`.
fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// `char.ToUpper`: one character in, one out.
fn upper_char(character: char) -> char {
    let mut mapped = character.to_uppercase();
    match (mapped.next(), mapped.next()) {
        (Some(single), None) => single,
        _ => character,
    }
}

fn lower_char(character: char) -> char {
    let mut mapped = character.to_lowercase();
    match (mapped.next(), mapped.next()) {
        (Some(single), None) => single,
        _ => character,
    }
}

fn lower_string(characters: &[char]) -> String {
    characters.iter().map(|character| lower_char(*character)).collect()
}

fn all_upper(characters: &[char]) -> bool {
    characters.iter().all(|character| is_upper(*character))
}

/// `\b` at `position`.
fn word_boundary(characters: &[char], position: usize) -> bool {
    let before = position > 0 && is_word(characters[position - 1]);
    let after = characters.get(position).is_some_and(|character| is_word(*character));
    before != after
}

/// The end of the `PascalCaseWordPartsRegex` match starting at `start`:
/// `[\p{Lu}]?[\p{Ll}]+|[0-9]+[\p{Ll}]*|[\p{Lu}]+(?=[\p{Lu}][\p{Ll}]|[0-9]|\b)|[\p{Lo}]+`.
fn pascal_case_part(characters: &[char], start: usize) -> Option<usize> {
    let upper_at = |index: usize| characters.get(index).is_some_and(|character| is_upper(*character));
    let lower_at = |index: usize| characters.get(index).is_some_and(|character| is_lower(*character));
    let digit_at = |index: usize| characters.get(index).is_some_and(|character| character.is_ascii_digit());

    // An optional capital, then lower case letters.
    let first_lower = if upper_at(start) { start + 1 } else { start };
    if lower_at(first_lower) {
        let mut end = first_lower;
        while lower_at(end) {
            end += 1;
        }
        return Some(end);
    }

    // Digits, then lower case letters.
    if digit_at(start) {
        let mut end = start;
        while digit_at(end) {
            end += 1;
        }
        while lower_at(end) {
            end += 1;
        }
        return Some(end);
    }

    // Capitals, as many as still leave a capitalized word, a digit or a
    // word boundary after them.
    if upper_at(start) {
        let mut run_end = start;
        while upper_at(run_end) {
            run_end += 1;
        }
        for end in (start + 1..=run_end).rev() {
            let starts_word = upper_at(end) && lower_at(end + 1);
            if starts_word || digit_at(end) || word_boundary(characters, end) {
                return Some(end);
            }
        }
    }

    // Letters without case.
    let other_at = |index: usize| characters.get(index).is_some_and(|character| is_other_letter(*character));
    if other_at(start) {
        let mut end = start;
        while other_at(end) {
            end += 1;
        }
        return Some(end);
    }
    None
}

/// `FromPascalCase`.
fn from_pascal_case(input: &str) -> String {
    let characters: Vec<char> = input.chars().collect();
    let mut words = Vec::new();
    let mut position = 0;
    while position < characters.len() {
        let Some(end) = pascal_case_part(&characters, position) else {
            position += 1;
            continue;
        };
        let part = &characters[position..end];
        let follows_space = position > 0 && characters[position - 1] == ' ';
        let is_i = part.len() == 1 && part[0] == 'I';
        let keeps_case = all_upper(part) && (part.len() > 1 || follows_space || is_i);
        let word = if keeps_case { part.iter().collect() } else { lower_string(part) };
        words.push(word);
        position = end;
    }

    let mut text = words.join(" ");
    let letters: Vec<char> = text.chars().filter(|character| *character != ' ').collect();
    if all_upper(&letters) && text.contains(' ') {
        let characters: Vec<char> = text.chars().collect();
        text = lower_string(&characters);
    }
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => std::iter::once(upper_char(first)).chain(characters).collect(),
        None => text,
    }
}

fn from_underscore_dash_separated_words(input: &str) -> String {
    let words: Vec<&str> = input.split(['_', '-']).collect();
    words.join(" ")
}

/// `FreestandingSpacingCharRegex`: `\s[-_]|[-_]\s`.
fn has_freestanding_spacing_character(input: &str) -> bool {
    let characters: Vec<char> = input.chars().collect();
    characters.windows(2).any(|pair| {
        let separator = |character: char| character == '-' || character == '_';
        (pair[0].is_whitespace() && separator(pair[1])) || (separator(pair[0]) && pair[1].is_whitespace())
    })
}

/// `string.Humanize()`.
pub fn humanize(input: &str) -> String {
    let characters: Vec<char> = input.chars().collect();
    if all_upper(&characters) {
        return input.to_string();
    }
    if has_freestanding_spacing_character(input) {
        let separated = from_underscore_dash_separated_words(input);
        return from_pascal_case(&separated);
    }
    if input.contains('_') || input.contains('-') {
        return from_underscore_dash_separated_words(input);
    }
    from_pascal_case(input)
}

/// `ApplyCase(LetterCasing.Title)` (Humanizer's `ToTitleCase`).
pub fn title_case(input: &str) -> String {
    let mut characters: Vec<char> = input.chars().collect();
    let length = characters.len();
    let word_or_wide = |character: char| is_word(character) || !character.is_ascii();
    let mut first_word = true;
    let mut position = 0;
    while position < length {
        if !word_or_wide(characters[position]) {
            position += 1;
            continue;
        }
        // `(\w|[^\u0000-\u007F])+'?\w*`
        let start = position;
        let mut end = start;
        while end < length && word_or_wide(characters[end]) {
            end += 1;
        }
        if end < length && characters[end] == '\'' {
            end += 1;
        }
        while end < length && is_word(characters[end]) {
            end += 1;
        }

        let word: Vec<char> = characters[start..end].to_vec();
        if !all_upper(&word) {
            let text: String = word.iter().collect();
            let stays_lower = !first_word && LOWER_CASE_WORDS.contains(&text.as_str());
            for (offset, character) in word.iter().enumerate() {
                let capital = offset == 0 && !stays_lower;
                characters[start + offset] = if capital { upper_char(*character) } else { lower_char(*character) };
            }
        }
        first_word = false;
        position = end;
    }
    characters.into_iter().collect()
}

/// `Humanize(LetterCasing.Title)`: how properties without a `[Name]` and
/// enum members without a `[Description]` are labelled.
pub fn humanize_title(input: &str) -> String {
    let humanized = humanize(input);
    title_case(&humanized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_names() {
        assert_eq!(humanize_title("ErosionScale"), "Erosion Scale");
        assert_eq!(humanize_title("NoErosion"), "No Erosion");
        assert_eq!(humanize_title("LevelOfDetail"), "Level of Detail");
        assert_eq!(humanize_title("UseHQMode"), "Use HQ Mode");
        assert_eq!(humanize_title("RGBMerge"), "RGB Merge");
        assert_eq!(humanize_title("HQ"), "HQ");
        assert_eq!(humanize_title("X2"), "X 2");
        assert_eq!(humanize_title("Seed"), "Seed");
    }

    #[test]
    fn humanize_keeps_sentence_case() {
        assert_eq!(humanize("ErosionScale"), "Erosion scale");
        assert_eq!(humanize("snake_case-name"), "snake case name");
        assert_eq!(humanize("UseHQMode"), "Use HQ mode");
    }

    #[test]
    fn descriptions_take_title_case() {
        assert_eq!(title_case("Half Raw 16-bit (.raw)"), "Half Raw 16-Bit (.Raw)");
        assert_eq!(title_case("Adaptive (Sophia)"), "Adaptive (Sophia)");
        assert_eq!(title_case("the end of it"), "The End of It");
    }
}
