use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;

use crate::{
    ingredients::normalize_ingredient,
    models::{IngredientReference, PantryItem, PantryStatus, Recipe, RecommendationRequest},
};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecommendationStatus {
    Ready,
    Verify,
    Shop,
    Blocked,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub recipe: Recipe,
    pub status: RecommendationStatus,
    pub available: Vec<String>,
    pub verify: Vec<String>,
    pub missing: Vec<String>,
    pub reasons: Vec<String>,
    pub score: i32,
}

pub fn recommend(
    recipes: Vec<Recipe>,
    pantry: &[PantryItem],
    request: &RecommendationRequest,
    verification_days: i64,
    now: DateTime<Utc>,
) -> Vec<Recommendation> {
    let pantry: HashMap<_, _> = pantry
        .iter()
        .map(|item| {
            (
                ingredient_key(item.canonical_ingredient_id.as_deref(), &item.name),
                item,
            )
        })
        .collect();
    let required = normalized_set(&request.required);
    let excluded = normalized_set(&request.excluded);
    let preferred = normalized_set(&request.preferred);
    let stale_before = now - Duration::days(verification_days);

    let mut results: Vec<_> = recipes
        .into_iter()
        .filter_map(|recipe| {
            let ingredients: HashSet<_> = recipe
                .ingredients
                .iter()
                .map(|ingredient| {
                    ingredient_key(
                        ingredient.canonical_ingredient_id.as_deref(),
                        &ingredient.name,
                    )
                })
                .collect();
            if !excluded.is_disjoint(&ingredients) || !required.is_subset(&ingredients) {
                return None;
            }

            let mut available = Vec::new();
            let mut verify = Vec::new();
            let mut missing = Vec::new();
            for ingredient in &recipe.ingredients {
                let normalized = ingredient_key(
                    ingredient.canonical_ingredient_id.as_deref(),
                    &ingredient.name,
                );
                match pantry.get(&normalized) {
                    Some(item)
                        if item.status == PantryStatus::Have
                            && !is_expired(item, now.date_naive())
                            && is_fresh(item, stale_before) =>
                    {
                        available.push(ingredient.name.clone())
                    }
                    Some(item)
                        if item.status == PantryStatus::Unknown
                            || item.status == PantryStatus::Have =>
                    {
                        verify.push(ingredient.name.clone())
                    }
                    _ => missing.push(ingredient.name.clone()),
                }
            }

            let blocked = request.pantry_only && (!missing.is_empty() || !verify.is_empty());
            let status = if blocked {
                RecommendationStatus::Blocked
            } else if !missing.is_empty() {
                RecommendationStatus::Shop
            } else if !verify.is_empty() {
                RecommendationStatus::Verify
            } else {
                RecommendationStatus::Ready
            };
            let preferred_matches = ingredients.intersection(&preferred).count() as i32;
            let score = status_rank(status) * 100 + preferred_matches * 10 + available.len() as i32;
            let mut reasons = vec![format!(
                "{} of {} ingredients confirmed",
                available.len(),
                recipe.ingredients.len()
            )];
            if preferred_matches > 0 {
                reasons.push(format!(
                    "{preferred_matches} preferred ingredient(s) included"
                ));
            }
            for ingredient in &recipe.ingredients {
                let key = ingredient_key(
                    ingredient.canonical_ingredient_id.as_deref(),
                    &ingredient.name,
                );
                if let Some(item) = pantry.get(&key) {
                    if let Some(expiry_reason) = expiry_reason(item, now.date_naive()) {
                        reasons.push(format!("{}: {expiry_reason}", ingredient.name));
                    }
                }
            }
            if blocked {
                reasons.push(
                    "Blocked by pantry-only mode because ingredients are not confirmed available"
                        .into(),
                );
            }
            Some(Recommendation {
                recipe,
                status,
                available,
                verify,
                missing,
                reasons,
                score,
            })
        })
        .collect();
    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.recipe.title.cmp(&right.recipe.title))
    });
    results
}

fn normalized_set(values: &[IngredientReference]) -> HashSet<String> {
    values
        .iter()
        .map(|value| ingredient_key(value.canonical_ingredient_id(), value.name()))
        .filter(|value| !value.is_empty())
        .collect()
}

fn ingredient_key(canonical_id: Option<&str>, name: &str) -> String {
    canonical_id
        .map(str::to_string)
        .unwrap_or_else(|| normalize_ingredient(name))
}

fn is_expired(item: &PantryItem, today: NaiveDate) -> bool {
    item.expires_on
        .as_deref()
        .and_then(parse_date)
        .is_some_and(|date| date <= today)
}

fn expiry_reason(item: &PantryItem, today: NaiveDate) -> Option<String> {
    let expires_on = item.expires_on.as_deref()?;
    let date = parse_date(expires_on)?;
    if date < today {
        Some(format!("expired on {expires_on}; verify before using"))
    } else if date == today {
        Some("expires today; verify before using".into())
    } else {
        None
    }
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

fn is_fresh(item: &PantryItem, stale_before: DateTime<Utc>) -> bool {
    item.confirmed_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc) >= stale_before)
        .unwrap_or(false)
}

fn status_rank(status: RecommendationStatus) -> i32 {
    match status {
        RecommendationStatus::Ready => 4,
        RecommendationStatus::Verify => 3,
        RecommendationStatus::Shop => 2,
        RecommendationStatus::Blocked => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::IngredientInput;

    fn recipe(id: &str, title: &str, ingredients: &[&str]) -> Recipe {
        Recipe {
            id: id.into(),
            owner_id: "owner".into(),
            is_owner: true,
            author_nickname: None,
            title: title.into(),
            description: String::new(),
            instructions: String::new(),
            visibility: "private".into(),
            ingredients: ingredients
                .iter()
                .map(|name| IngredientInput {
                    name: (*name).into(),
                    quantity_note: String::new(),
                    canonical_ingredient_id: None,
                })
                .collect(),
            is_favorite: false,
            source: None,
        }
    }

    fn pantry(name: &str, status: PantryStatus, confirmed_at: Option<&str>) -> PantryItem {
        PantryItem {
            id: name.into(),
            name: name.into(),
            status,
            quantity_note: String::new(),
            confirmed_at: confirmed_at.map(str::to_string),
            expires_on: None,
            canonical_ingredient_id: None,
        }
    }

    #[test]
    fn golden_recommendation_order_and_explanations() {
        let now = DateTime::parse_from_rfc3339("2026-01-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let results = recommend(
            vec![
                recipe("2", "Toast", &["bread"]),
                recipe("1", "Egg toast", &["egg", "bread"]),
            ],
            &[
                pantry("egg", PantryStatus::Have, Some("2026-01-19T00:00:00Z")),
                pantry("bread", PantryStatus::Have, Some("2025-12-01T00:00:00Z")),
            ],
            &RecommendationRequest {
                preferred: vec!["egg".into()],
                ..Default::default()
            },
            14,
            now,
        );
        assert_eq!(results[0].recipe.title, "Egg toast");
        assert_eq!(results[0].status, RecommendationStatus::Verify);
        assert_eq!(results[0].available, vec!["egg"]);
        assert_eq!(results[0].verify, vec!["bread"]);
    }

    #[test]
    fn hard_filters_apply_before_scoring() {
        let results = recommend(
            vec![
                recipe("1", "Egg toast", &["egg", "bread"]),
                recipe("2", "Toast", &["bread"]),
            ],
            &[],
            &RecommendationRequest {
                required: vec!["egg".into()],
                excluded: vec!["bread".into()],
                ..Default::default()
            },
            14,
            Utc::now(),
        );
        assert!(results.is_empty());
    }

    #[test]
    fn pantry_only_marks_missing_recipe_blocked() {
        let results = recommend(
            vec![recipe("1", "Toast", &["bread"])],
            &[],
            &RecommendationRequest {
                pantry_only: true,
                ..Default::default()
            },
            14,
            Utc::now(),
        );
        assert_eq!(results[0].status, RecommendationStatus::Blocked);
    }

    #[test]
    fn pantry_only_marks_uncertain_recipe_blocked() {
        let results = recommend(
            vec![recipe("1", "Toast", &["bread"])],
            &[pantry("bread", PantryStatus::Unknown, None)],
            &RecommendationRequest {
                pantry_only: true,
                ..Default::default()
            },
            14,
            Utc::now(),
        );
        assert_eq!(results[0].status, RecommendationStatus::Blocked);
        assert_eq!(results[0].verify, vec!["bread"]);
    }

    #[test]
    fn expired_have_requires_verification_without_becoming_used_up() {
        let now = DateTime::parse_from_rfc3339("2026-11-04T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut item = pantry("milk", PantryStatus::Have, Some("2026-11-04T00:00:00Z"));
        item.expires_on = Some("2026-11-03".into());
        let results = recommend(
            vec![recipe("1", "Milk drink", &["milk"])],
            &[item],
            &RecommendationRequest::default(),
            14,
            now,
        );
        assert_eq!(results[0].status, RecommendationStatus::Verify);
        assert_eq!(results[0].verify, vec!["milk"]);
        assert!(results[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("expired on 2026-11-03")));
    }

    #[test]
    fn expires_today_blocks_pantry_only_with_clear_reason() {
        let now = DateTime::parse_from_rfc3339("2026-11-04T23:59:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut item = pantry("milk", PantryStatus::Have, Some("2026-11-04T00:00:00Z"));
        item.expires_on = Some("2026-11-04".into());
        let results = recommend(
            vec![recipe("1", "Milk drink", &["milk"])],
            &[item],
            &RecommendationRequest {
                pantry_only: true,
                ..Default::default()
            },
            14,
            now,
        );
        assert_eq!(results[0].status, RecommendationStatus::Blocked);
        assert!(results[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("expires today")));
    }
}
