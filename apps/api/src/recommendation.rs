use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::models::{
    normalize_ingredient, PantryItem, PantryStatus, Recipe, RecommendationRequest,
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
        .map(|item| (normalize_ingredient(&item.name), item))
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
                .map(|ingredient| normalize_ingredient(&ingredient.name))
                .collect();
            if !excluded.is_disjoint(&ingredients) || !required.is_subset(&ingredients) {
                return None;
            }

            let mut available = Vec::new();
            let mut verify = Vec::new();
            let mut missing = Vec::new();
            for ingredient in &recipe.ingredients {
                let normalized = normalize_ingredient(&ingredient.name);
                match pantry.get(&normalized) {
                    Some(item)
                        if item.status == PantryStatus::Have && is_fresh(item, stale_before) =>
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

fn normalized_set(values: &[String]) -> HashSet<String> {
    values
        .iter()
        .map(|value| normalize_ingredient(value))
        .filter(|value| !value.is_empty())
        .collect()
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
            title: title.into(),
            description: String::new(),
            instructions: String::new(),
            visibility: "private".into(),
            ingredients: ingredients
                .iter()
                .map(|name| IngredientInput {
                    name: (*name).into(),
                    quantity_note: String::new(),
                })
                .collect(),
            is_favorite: false,
        }
    }

    fn pantry(name: &str, status: PantryStatus, confirmed_at: Option<&str>) -> PantryItem {
        PantryItem {
            id: name.into(),
            name: name.into(),
            status,
            quantity_note: String::new(),
            confirmed_at: confirmed_at.map(str::to_string),
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
}
