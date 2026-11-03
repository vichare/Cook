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
    if canonical.starts_with(&query) || aliases.iter().any(|alias| alias.starts_with(&query)) {
        return Some(MatchReason::Prefix);
    }
    if canonical.contains(&query) || aliases.iter().any(|alias| alias.contains(&query)) {
        return Some(MatchReason::Substring);
    }
    let maximum_distance = if query.chars().count() <= 5 { 1 } else { 2 };
    std::iter::once(canonical.as_str())
        .chain(aliases.iter().map(String::as_str))
        .any(|candidate| bounded_levenshtein(&query, candidate, maximum_distance).is_some())
        .then_some(MatchReason::Similar)
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
    fn matching_order_keeps_fuzzy_bounded_and_advisory() {
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
        assert_eq!(
            match_reason("tomtao", "Tomato", &[]),
            Some(MatchReason::Similar)
        );
        assert_eq!(match_reason("soda", "Baking powder", &[]), None);
    }
}
