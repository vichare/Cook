use std::collections::HashSet;

use serde::Serialize;
use unicode_normalization::UnicodeNormalization;

const LIST_SEPARATORS: [char; 7] = [',', '，', '、', ';', '；', '\n', '\r'];

pub fn normalize_ingredient(value: &str) -> String {
    clean_display_name(value)
        .chars()
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn clean_display_name(value: &str) -> String {
    value
        .nfkc()
        .map(|character| {
            if character == '\u{3000}' || character.is_whitespace() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn split_ingredient_list(value: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    value
        .split(|character| LIST_SEPARATORS.contains(&character))
        .map(clean_display_name)
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(normalize_ingredient(value)))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchReason {
    Exact,
    Alias,
    Prefix,
    Substring,
    Similar,
}

pub fn match_reason(query: &str, canonical: &str, aliases: &[String]) -> Option<MatchReason> {
    let query = normalize_ingredient(query);
    if query.is_empty() {
        return None;
    }
    let canonical = normalize_ingredient(canonical);
    let aliases: Vec<_> = aliases
        .iter()
        .map(|value| normalize_ingredient(value))
        .collect();
    if canonical == query {
        return Some(MatchReason::Exact);
    }
    if aliases.iter().any(|alias| alias == &query) {
        return Some(MatchReason::Alias);
    }
    let query_length = query.chars().count();
    let canonical_length = canonical.chars().count();
    let query_has_cjk = contains_cjk(&query);
    let canonical_has_cjk = contains_cjk(&canonical);
    if canonical_length > query_length
        && canonical.starts_with(&query)
        && (!query_has_cjk || query_length >= 2)
    {
        return Some(MatchReason::Prefix);
    }
    if !query_has_cjk
        && !canonical_has_cjk
        && canonical_length > query_length
        && canonical.contains(&query)
    {
        return Some(MatchReason::Substring);
    }
    if query_length < 4
        || query_has_cjk
        || canonical_has_cjk
        || !is_latin_term(&query)
        || !is_latin_term(&canonical)
        || canonical.contains(&query)
        || query.contains(&canonical)
    {
        return None;
    }
    let maximum_distance = if query_length <= 7 { 1 } else { 2 };
    bounded_levenshtein(&query, &canonical, maximum_distance)
        .is_some()
        .then_some(MatchReason::Similar)
}

pub fn contains_cjk(value: &str) -> bool {
    value.chars().any(|character| {
        matches!(
            character as u32,
            0x1100..=0x11ff
                | 0x2e80..=0x2fff
                | 0x3040..=0x30ff
                | 0x3130..=0x318f
                | 0x31f0..=0x31ff
                | 0x3400..=0x4dbf
                | 0x4e00..=0x9fff
                | 0xac00..=0xd7af
                | 0xf900..=0xfaff
                | 0x20000..=0x323af
        )
    })
}

fn is_latin_term(value: &str) -> bool {
    value.chars().all(|character| {
        character.is_whitespace()
            || matches!(character, '-' | '\'' | '’')
            || matches!(
                character as u32,
                0x0041..=0x005a
                    | 0x0061..=0x007a
                    | 0x00c0..=0x024f
                    | 0x0300..=0x036f
                    | 0x1e00..=0x1eff
            )
    })
}

pub fn bounded_levenshtein(left: &str, right: &str, maximum: usize) -> Option<usize> {
    let left: Vec<_> = left.chars().collect();
    let right: Vec<_> = right.chars().collect();
    if left.len().abs_diff(right.len()) > maximum {
        return None;
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (left_index, left_character) in left.iter().enumerate() {
        let mut current = vec![left_index + 1];
        let mut row_minimum = current[0];
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + usize::from(left_character != right_character);
            let insertion = current[right_index] + 1;
            let deletion = previous[right_index + 1] + 1;
            let distance = substitution.min(insertion).min(deletion);
            row_minimum = row_minimum.min(distance);
            current.push(distance);
        }
        if row_minimum > maximum {
            return None;
        }
        previous = current;
    }
    (previous[right.len()] <= maximum).then_some(previous[right.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_handles_nfkc_case_and_unicode_whitespace() {
        assert_eq!(
            normalize_ingredient("  ＴＯＭＡＴＯ\u{3000} Sauce  "),
            "tomato sauce"
        );
        assert_eq!(normalize_ingredient("Crème   Fraîche"), "crème fraîche");
    }

    #[test]
    fn pasted_lists_split_deduplicate_and_preserve_display_case() {
        assert_eq!(
            split_ingredient_list("Egg， milk、EGG；青菜\n  Rice "),
            vec!["Egg", "milk", "青菜", "Rice"]
        );
    }

    #[test]
    fn matching_order_keeps_advisory_matches_bounded_and_identity_safe() {
        assert_eq!(
            match_reason("tomato", "Tomato", &[]),
            Some(MatchReason::Exact)
        );
        assert_eq!(
            match_reason("scallion", "Green onion", &["Scallion".into()]),
            Some(MatchReason::Alias)
        );
        assert_eq!(
            match_reason("green", "Green onion", &[]),
            Some(MatchReason::Prefix)
        );
        assert_eq!(
            match_reason("onion", "Green onion", &[]),
            Some(MatchReason::Substring)
        );
        assert_eq!(match_reason("tomtao", "Tomato", &[]), None);
        assert_eq!(
            match_reason("tomoto", "Tomato", &[]),
            Some(MatchReason::Similar)
        );
        assert_eq!(match_reason("soda", "Baking powder", &[]), None);
        assert_eq!(match_reason("酱油", "油", &[]), None);
        assert_eq!(match_reason("油", "酱油", &[]), None);
        assert_eq!(match_reason("鸡", "鸡腿", &[]), None);
        assert_eq!(
            match_reason("鸡腿", "鸡腿肉", &[]),
            Some(MatchReason::Prefix)
        );
        assert_eq!(match_reason("鸡腿", "鸡翅", &[]), None);
        assert_eq!(match_reason("と", "とうふ", &[]), None);
        assert_eq!(
            match_reason("두부", "두부면", &[]),
            Some(MatchReason::Prefix)
        );
        assert_eq!(match_reason("두부", "두유", &[]), None);
        assert_eq!(match_reason("сахр", "сахар", &[]), None);
        assert_eq!(match_reason("chi", "chili", &[]), Some(MatchReason::Prefix));
        assert_eq!(
            match_reason("tom", "Tomato", &[]),
            Some(MatchReason::Prefix)
        );
        assert_eq!(match_reason("tmto", "Tomato", &[]), None);
        assert_eq!(
            match_reason("tomto", "Tomato", &[]),
            Some(MatchReason::Similar)
        );
        assert_eq!(
            match_reason("roma", "Tomato", &["Roma tomato".into()]),
            None
        );
    }
}
