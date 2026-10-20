const NUMBER_WORDS: &[(&str, u32)] = &[
    ("zero", 0),
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("seventy", 70),
    ("eighty", 80),
    ("ninety", 90),
    ("hundred", 100),
    ("thousand", 1_000),
    ("million", 1_000_000),
    ("billion", 1_000_000_000),
];

const BRAND_PREFIXES: &[&str] = &["rtx", "gtx", "rx", "mx"];

const DUPLICATE_WORDS: &[&str] = &["a", "an", "the", "and", "to", "of", "is", "it"];

const BEFORE_BLOCKS: &[&str] = &[
    "at", "by", "around", "about", "roughly", "between", "past", "before", "after", "until", "till",
    "til", "it's", "its",
];

const AFTER_BLOCKS: &[&str] = &["o'clock", "oclock", "am", "pm", "a.m.", "p.m.", "ish"];

const LEAD_PUNCTUATION: &[char] = &['"', '\'', '“', '”', '‘', '’', '(', '[', '{', '<'];

const TRAIL_PUNCTUATION: &[char] = &[
    '.', ',', '!', '?', ';', ':', '"', '\'', '”', '’', ')', ']', '}', '>', '…',
];

#[derive(Clone, Copy)]
struct Token<'a> {
    ws: &'a str,
    word: &'a str,
    core: &'a str,
    lead: &'a str,
    trail: &'a str,
}

struct Run<'a> {
    values: Vec<u32>,
    has_zero: bool,
    end: usize,
    lead: &'a str,
    trail: &'a str,
}

pub fn normalize(text: &str) -> String {
    let (tokens, tail) = tokenize(text);
    if tokens.is_empty() {
        return text.to_string();
    }
    let tokens = collapse_duplicates(tokens);
    let mut out = String::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        let lower = token.core.to_lowercase();
        if BRAND_PREFIXES.contains(&lower.as_str()) && token.trail.is_empty() {
            if let Some(run) = read_run(&tokens, index + 1) {
                if let Some((digits, _)) = render(&run) {
                    if !blocked(&tokens, index, run.end) {
                        out.push_str(token.ws);
                        out.push_str(token.lead);
                        out.push_str(token.core);
                        out.push(' ');
                        out.push_str(&digits);
                        out.push_str(run.trail);
                        index = run.end;
                        continue;
                    }
                }
            }
        }
        if is_code_letter(token) {
            if let Some(run) = read_run(&tokens, index + 1) {
                if let Some((digits, _)) = render(&run) {
                    if !blocked(&tokens, index, run.end) {
                        out.push_str(token.ws);
                        out.push_str(token.core);
                        out.push_str(&digits);
                        out.push_str(run.trail);
                        index = run.end;
                        continue;
                    }
                }
            }
        }
        if let Some(run) = read_run(&tokens, index) {
            match render(&run) {
                Some((digits, strong)) => {
                    let suffix = if run.trail.is_empty() {
                        tokens.get(run.end).filter(|next| is_code_letter(next))
                    } else {
                        None
                    };
                    let end = run.end + usize::from(suffix.is_some());
                    if (strong || suffix.is_some()) && !blocked(&tokens, index, end) {
                        out.push_str(token.ws);
                        out.push_str(run.lead);
                        out.push_str(&digits);
                        if let Some(suffix) = suffix {
                            out.push_str(suffix.core);
                        }
                        out.push_str(run.trail);
                        index = end;
                        continue;
                    }
                }
                None => {
                    for token in &tokens[index..run.end] {
                        out.push_str(token.ws);
                        out.push_str(token.word);
                    }
                    index = run.end;
                    continue;
                }
            }
        }
        out.push_str(token.ws);
        out.push_str(token.word);
        index += 1;
    }
    out.push_str(tail);
    out
}

fn tokenize(text: &str) -> (Vec<Token<'_>>, &str) {
    let mut tokens = Vec::new();
    let mut from = 0;
    for word in text.split_whitespace() {
        let start = text[from..]
            .find(word)
            .map(|offset| from + offset)
            .unwrap_or(from);
        let lead_len: usize = word
            .chars()
            .take_while(|value| LEAD_PUNCTUATION.contains(value))
            .map(|value| value.len_utf8())
            .sum();
        let rest = &word[lead_len..];
        let trail_len: usize = rest
            .chars()
            .rev()
            .take_while(|value| TRAIL_PUNCTUATION.contains(value))
            .map(|value| value.len_utf8())
            .sum();
        tokens.push(Token {
            ws: &text[from..start],
            word,
            lead: &word[..lead_len],
            core: &rest[..rest.len() - trail_len],
            trail: &rest[rest.len() - trail_len..],
        });
        from = start + word.len();
    }
    (tokens, &text[from..])
}

fn collapse_duplicates(tokens: Vec<Token<'_>>) -> Vec<Token<'_>> {
    let mut kept: Vec<Token> = Vec::new();
    for token in tokens {
        let duplicate = token.lead.is_empty()
            && token.trail.is_empty()
            && DUPLICATE_WORDS.contains(&token.core.to_lowercase().as_str())
            && kept.last().is_some_and(|previous| {
                previous.lead.is_empty()
                    && previous.trail.is_empty()
                    && previous.core.to_lowercase() == token.core.to_lowercase()
            });
        if !duplicate {
            kept.push(token);
        }
    }
    kept
}

fn is_code_letter(token: &Token<'_>) -> bool {
    token.lead.is_empty()
        && token.trail.is_empty()
        && token.core.len() == 1
        && token.core.chars().all(|value| value.is_ascii_uppercase())
        && token.core != "A"
        && token.core != "I"
}

fn spoken_value(core: &str) -> Option<u32> {
    let lower = core.to_lowercase();
    if lower == "oh" || lower == "o" {
        return Some(0);
    }
    if let Some(value) = word_value(&lower) {
        return Some(value);
    }
    let mut parts = lower.split('-');
    let first = parts.next()?;
    let second = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let base = word_value(first)?;
    let unit = word_value(second)?;
    if (20..=90).contains(&base) && base % 10 == 0 && (1..=9).contains(&unit) {
        return Some(base + unit);
    }
    None
}

fn word_value(word: &str) -> Option<u32> {
    NUMBER_WORDS
        .iter()
        .find(|(name, _)| *name == word)
        .map(|(_, value)| *value)
}

fn read_run<'a>(tokens: &[Token<'a>], start: usize) -> Option<Run<'a>> {
    let first = tokens.get(start)?;
    let mut values = Vec::new();
    let mut has_zero = false;
    let mut scale_seen = false;
    let mut index = start;
    while index < tokens.len() {
        let token = &tokens[index];
        if index > start && !token.lead.is_empty() {
            break;
        }
        match spoken_value(token.core) {
            Some(value) => {
                values.push(value);
                has_zero |= value == 0;
                scale_seen = value >= 100;
                index += 1;
                if !token.trail.is_empty() {
                    break;
                }
            }
            None => {
                let lower = token.core.to_lowercase();
                let next_is_number = tokens
                    .get(index + 1)
                    .is_some_and(|next| spoken_value(next.core).is_some());
                if token.trail.is_empty() && lower == "and" && scale_seen && next_is_number {
                    scale_seen = false;
                    index += 1;
                } else {
                    break;
                }
            }
        }
    }
    if values.is_empty() {
        return None;
    }
    Some(Run {
        values,
        has_zero,
        end: index,
        lead: first.lead,
        trail: tokens[index - 1].trail,
    })
}

fn render(run: &Run<'_>) -> Option<(String, bool)> {
    let values = &run.values;
    if values.iter().any(|value| *value >= 100) {
        let digits = additive(values)?;
        return Some((digits, values.len() >= 2));
    }
    if values.iter().all(|value| *value <= 9) {
        let digits: String = values.iter().map(u32::to_string).collect();
        if digits.len() > 4 || digits.starts_with('0') {
            return None;
        }
        let strong = (run.has_zero || values.len() >= 3) && digits.len() >= 2;
        return Some((digits, strong));
    }
    let mut segments: Vec<u32> = Vec::new();
    let mut index = 0;
    while index < values.len() {
        let value = values[index];
        let merged = values
            .get(index + 1)
            .filter(|next| (1..=9).contains(*next) && (20..=90).contains(&value) && value % 10 == 0)
            .map(|next| value + next);
        match merged {
            Some(total) => {
                segments.push(total);
                index += 2;
            }
            None => {
                segments.push(value);
                index += 1;
            }
        }
    }
    let digits: String = segments.iter().map(u32::to_string).collect();
    if digits.len() > 4 || digits.starts_with('0') {
        return None;
    }
    let repeated = segments.len() == 2 && segments[0] == segments[1];
    let strong = segments.len() >= 2 && digits.len() >= 3 && !repeated;
    Some((digits, strong))
}

fn additive(values: &[u32]) -> Option<String> {
    let mut total: u64 = 0;
    let mut current: u64 = 0;
    for value in values {
        match *value {
            100 => {
                if current >= 100 {
                    return None;
                }
                current = current.max(1) * 100;
            }
            1_000 | 1_000_000 | 1_000_000_000 => {
                if current >= 1_000 {
                    return None;
                }
                total = total.checked_add(current.max(1) * u64::from(*value))?;
                current = 0;
            }
            small => current = current.checked_add(u64::from(small))?,
        }
    }
    total = total.checked_add(current)?;
    Some(total.to_string())
}

fn blocked(tokens: &[Token<'_>], start: usize, end: usize) -> bool {
    let before = start.checked_sub(1).and_then(|index| tokens.get(index));
    let after = tokens.get(end);
    before.is_some_and(|token| BEFORE_BLOCKS.contains(&token.core.to_lowercase().as_str()))
        || after.is_some_and(|token| AFTER_BLOCKS.contains(&token.core.to_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoken_product_numbers_become_digits() {
        let raw = "the AMD AI Pro Max three ninety five and Strix Halo and then also the Elite X two from Snapdragon and then the M Five Pro and Max and then also a Ryzen ninety nine fifty X and a a fifty eighty";
        let expected = "the AMD AI Pro Max 395 and Strix Halo and then also the Elite X2 from Snapdragon and then the M5 Pro and Max and then also a Ryzen 9950X and a 5080";
        assert_eq!(normalize(raw), expected);
    }

    #[test]
    fn multi_word_numbers_keep_scale_values() {
        assert_eq!(normalize("three hundred ninety five"), "395");
        assert_eq!(normalize("two thousand twenty six"), "2026");
        assert_eq!(normalize("one hundred and fifty"), "150");
        assert_eq!(normalize("twelve hundred"), "1200");
        assert_eq!(normalize("a hundred"), "a hundred");
        assert_eq!(
            normalize("five hundred five hundred"),
            "five hundred five hundred"
        );
    }

    #[test]
    fn digit_pair_readings_compact() {
        assert_eq!(normalize("fifty eighty"), "5080");
        assert_eq!(normalize("ninety nine fifty"), "9950");
        assert_eq!(normalize("three sixty"), "360");
        assert_eq!(normalize("nineteen ninety five"), "1995");
        assert_eq!(normalize("twenty twenty five"), "2025");
        assert_eq!(normalize("four oh two"), "402");
        assert_eq!(normalize("eight oh five"), "805");
        assert_eq!(normalize("one two three"), "123");
        assert_eq!(normalize("twenty-five apples"), "twenty-five apples");
    }

    #[test]
    fn lone_number_words_stay_words() {
        assert_eq!(normalize("three files"), "three files");
        assert_eq!(normalize("ninety five"), "ninety five");
        assert_eq!(normalize("twenty three apples"), "twenty three apples");
        assert_eq!(normalize("one two"), "one two");
        assert_eq!(normalize("fifty fifty"), "fifty fifty");
        assert_eq!(normalize("I have five"), "I have five");
    }

    #[test]
    fn letter_prefixes_and_suffixes_join() {
        assert_eq!(normalize("M Five"), "M5");
        assert_eq!(normalize("X two"), "X2");
        assert_eq!(normalize("five K"), "5K");
        assert_eq!(normalize("K ninety five"), "K95");
        assert_eq!(normalize("RTX fifty eighty"), "RTX 5080");
        assert_eq!(normalize("a five"), "a five");
        assert_eq!(normalize("I five"), "I five");
        assert_eq!(normalize("x two"), "x two");
    }

    #[test]
    fn cued_times_stay_words() {
        assert_eq!(normalize("at four thirty"), "at four thirty");
        assert_eq!(normalize("it's four thirty"), "it's four thirty");
        assert_eq!(normalize("four thirty am"), "four thirty am");
        assert_eq!(normalize("around fifty eighty"), "around fifty eighty");
        assert_eq!(normalize("between fifty eighty"), "between fifty eighty");
        assert_eq!(normalize("four thirty"), "430");
    }

    #[test]
    fn separators_end_number_runs() {
        assert_eq!(normalize("fifty, eighty"), "fifty, eighty");
        assert_eq!(normalize("fifty or eighty"), "fifty or eighty");
        assert_eq!(normalize("fifty. Eighty is next"), "fifty. Eighty is next");
        assert_eq!(normalize("fifty eighty,"), "5080,");
        assert_eq!(normalize("fifty eighty."), "5080.");
        assert_eq!(normalize("\"fifty eighty\""), "\"5080\"");
        assert_eq!(normalize("Oh, I see"), "Oh, I see");
    }

    #[test]
    fn duplicate_function_words_collapse() {
        assert_eq!(normalize("a a fifty eighty"), "a 5080");
        assert_eq!(normalize("the the report"), "the report");
        assert_eq!(normalize("to to go"), "to go");
        assert_eq!(normalize("very very good"), "very very good");
        assert_eq!(normalize("that that happened"), "that that happened");
        assert_eq!(normalize("no no"), "no no");
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   "), "   ");
        assert_eq!(normalize("hello world"), "hello world");
        assert_eq!(normalize("the report is ready"), "the report is ready");
        assert_eq!(
            normalize("I have two apples and three oranges"),
            "I have two apples and three oranges"
        );
    }
}
