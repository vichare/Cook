use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;

use crate::{
    ingredients::normalize_ingredient,
    models::{
        IngredientReference, PantryItem, PantryStatus, Recipe, RecommendationRequest,
        RecommendationSettings,
    },
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
    pub reason_details: Vec<RecommendationReason>,
    pub score: i32,
    pub score_breakdown: ScoreBreakdown,
    pub primary_source: RecommendationSource,
    pub matched_sources: Vec<RecommendationSource>,
    pub desired_matches: Vec<String>,
    pub ingredient_reasons: Vec<IngredientReason>,
    pub applied_controls: AppliedControls,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecommendationSource {
    Own,
    FavoriteRecipe,
    FavoriteAccount,
    OtherPublic,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    pub status_base: i32,
    pub source: i32,
    pub availability: i32,
    pub desired: i32,
    pub certainty: i32,
    pub simplicity: i32,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IngredientReasonCode {
    Available,
    Unknown,
    Stale,
    ExpiresToday,
    Expired,
    Out,
    Missing,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IngredientReason {
    pub ingredient: String,
    pub code: IngredientReasonCode,
    pub expires_on: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedControls {
    pub preferred: Vec<String>,
    pub required: Vec<String>,
    pub excluded: Vec<String>,
    pub pantry_only: bool,
    pub include_own: bool,
    pub include_favorite_recipes: bool,
    pub include_favorite_accounts: bool,
    pub include_other_public: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum RecommendationReason {
    ConfirmedIngredients {
        confirmed: usize,
        total: usize,
    },
    PreferredIngredients {
        count: usize,
    },
    IngredientExpired {
        ingredient: String,
        #[serde(rename = "expiresOn")]
        expires_on: String,
    },
    IngredientExpiresToday {
        ingredient: String,
    },
    PantryOnlyBlocked,
}

pub fn recommend(
    recipes: Vec<Recipe>,
    pantry: &[PantryItem],
    request: &RecommendationRequest,
    settings: &RecommendationSettings,
    now: DateTime<Utc>,
) -> Vec<Recommendation> {
    recommend_inner(recipes, pantry, request, settings, now, None)
}

pub fn recommend_limited(
    recipes: Vec<Recipe>,
    pantry: &[PantryItem],
    request: &RecommendationRequest,
    settings: &RecommendationSettings,
    now: DateTime<Utc>,
    limit: usize,
) -> Vec<Recommendation> {
    recommend_inner(recipes, pantry, request, settings, now, Some(limit))
}

fn recommend_inner(
    recipes: Vec<Recipe>,
    pantry: &[PantryItem],
    request: &RecommendationRequest,
    settings: &RecommendationSettings,
    now: DateTime<Utc>,
    limit: Option<usize>,
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
    let stale_before = now - Duration::days(settings.verification_days);

    let results = recipes.into_iter().filter_map(|recipe| {
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
        let matched_sources = matched_sources(&recipe, settings);
        let primary_source = *matched_sources.first()?;

        let mut available = Vec::new();
        let mut verify = Vec::new();
        let mut missing = Vec::new();
        let mut ingredient_reasons = Vec::new();
        for ingredient in &recipe.ingredients {
            let normalized = ingredient_key(
                ingredient.canonical_ingredient_id.as_deref(),
                &ingredient.name,
            );
            let (code, expires_on) = match pantry.get(&normalized) {
                None => {
                    missing.push(ingredient.name.clone());
                    (IngredientReasonCode::Missing, None)
                }
                Some(item) if item.status == PantryStatus::Out => {
                    missing.push(ingredient.name.clone());
                    (IngredientReasonCode::Out, item.expires_on.clone())
                }
                Some(item) if item.status == PantryStatus::Unknown => {
                    verify.push(ingredient.name.clone());
                    (IngredientReasonCode::Unknown, item.expires_on.clone())
                }
                Some(item) => {
                    let expiry = item.expires_on.as_deref().and_then(parse_date);
                    if expiry.is_some_and(|date| date < now.date_naive()) {
                        verify.push(ingredient.name.clone());
                        (IngredientReasonCode::Expired, item.expires_on.clone())
                    } else if expiry.is_some_and(|date| date == now.date_naive()) {
                        verify.push(ingredient.name.clone());
                        (IngredientReasonCode::ExpiresToday, item.expires_on.clone())
                    } else if !is_fresh(item, stale_before) {
                        verify.push(ingredient.name.clone());
                        (IngredientReasonCode::Stale, item.expires_on.clone())
                    } else {
                        available.push(ingredient.name.clone());
                        (IngredientReasonCode::Available, item.expires_on.clone())
                    }
                }
            };
            ingredient_reasons.push(IngredientReason {
                ingredient: ingredient.name.clone(),
                code,
                expires_on,
            });
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
        let desired_matches = recipe
            .ingredients
            .iter()
            .filter(|ingredient| {
                preferred.contains(&ingredient_key(
                    ingredient.canonical_ingredient_id.as_deref(),
                    &ingredient.name,
                ))
            })
            .map(|ingredient| ingredient.name.clone())
            .collect::<Vec<_>>();
        let preferred_matches = i32::try_from(desired_matches.len()).unwrap_or(i32::MAX);
        let preferred_count = i32::try_from(preferred.len()).unwrap_or(i32::MAX);
        let ingredient_count = i32::try_from(recipe.ingredients.len())
            .unwrap_or(i32::MAX)
            .max(1);
        let available_count = i32::try_from(available.len()).unwrap_or(i32::MAX);
        let missing_count = i32::try_from(missing.len()).unwrap_or(i32::MAX);
        let breakdown = ScoreBreakdown {
            status_base: status_base(status),
            source: source_points(primary_source),
            availability: (available_count * 80 / ingredient_count).clamp(0, 80),
            desired: if preferred_count == 0 {
                0
            } else {
                (preferred_matches * 40 / preferred_count).clamp(0, 40)
            },
            certainty: (available_count * 20 / ingredient_count).clamp(0, 20),
            simplicity: (9 - missing_count).clamp(0, 9),
        };
        let score = breakdown.status_base
            + breakdown.source
            + breakdown.availability
            + breakdown.desired
            + breakdown.certainty
            + breakdown.simplicity;
        debug_assert!((0..=999).contains(&score));
        let mut reasons = vec![format!(
            "{} of {} ingredients confirmed",
            available.len(),
            recipe.ingredients.len()
        )];
        let mut reason_details = vec![RecommendationReason::ConfirmedIngredients {
            confirmed: available.len(),
            total: recipe.ingredients.len(),
        }];
        if preferred_matches > 0 {
            reasons.push(format!(
                "{preferred_matches} preferred ingredient(s) included"
            ));
            reason_details.push(RecommendationReason::PreferredIngredients {
                count: preferred_matches as usize,
            });
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
                if let Some(expires_on) = item.expires_on.as_deref() {
                    if let Some(date) = parse_date(expires_on) {
                        if date < now.date_naive() {
                            reason_details.push(RecommendationReason::IngredientExpired {
                                ingredient: ingredient.name.clone(),
                                expires_on: expires_on.to_string(),
                            });
                        } else if date == now.date_naive() {
                            reason_details.push(RecommendationReason::IngredientExpiresToday {
                                ingredient: ingredient.name.clone(),
                            });
                        }
                    }
                }
            }
        }
        if blocked {
            reasons.push(
                "Blocked by pantry-only mode because ingredients are not confirmed available"
                    .into(),
            );
            reason_details.push(RecommendationReason::PantryOnlyBlocked);
        }
        Some(Recommendation {
            recipe,
            status,
            available,
            verify,
            missing,
            reasons,
            reason_details,
            score,
            score_breakdown: breakdown,
            primary_source,
            matched_sources,
            desired_matches,
            ingredient_reasons,
            applied_controls: AppliedControls {
                preferred: request
                    .preferred
                    .iter()
                    .map(|value| value.name().to_string())
                    .collect(),
                required: request
                    .required
                    .iter()
                    .map(|value| value.name().to_string())
                    .collect(),
                excluded: request
                    .excluded
                    .iter()
                    .map(|value| value.name().to_string())
                    .collect(),
                pantry_only: request.pantry_only,
                include_own: settings.include_own,
                include_favorite_recipes: settings.include_favorite_recipes,
                include_favorite_accounts: settings.include_favorite_accounts,
                include_other_public: settings.include_other_public,
            },
        })
    });
    if let Some(limit) = limit {
        let mut heap = BinaryHeap::with_capacity(limit.saturating_add(1));
        for result in results {
            heap.push(WorstFirst(result));
            if heap.len() > limit {
                heap.pop();
            }
        }
        let mut results = heap.into_iter().map(|ranked| ranked.0).collect::<Vec<_>>();
        results.sort_by(compare_recommendations);
        results
    } else {
        let mut results = results.collect::<Vec<_>>();
        results.sort_by(compare_recommendations);
        results
    }
}

struct WorstFirst(Recommendation);

impl PartialEq for WorstFirst {
    fn eq(&self, other: &Self) -> bool {
        compare_recommendations(&self.0, &other.0) == Ordering::Equal
    }
}

impl Eq for WorstFirst {}

impl PartialOrd for WorstFirst {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for WorstFirst {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_recommendations(&self.0, &other.0)
    }
}

fn compare_recommendations(left: &Recommendation, right: &Recommendation) -> Ordering {
    status_rank(right.status)
        .cmp(&status_rank(left.status))
        .then_with(|| right.score.cmp(&left.score))
        .then_with(|| {
            normalize_ingredient(&left.recipe.title).cmp(&normalize_ingredient(&right.recipe.title))
        })
        .then_with(|| left.recipe.id.cmp(&right.recipe.id))
}

fn matched_sources(
    recipe: &Recipe,
    settings: &RecommendationSettings,
) -> Vec<RecommendationSource> {
    let mut sources = Vec::new();
    if recipe.is_owner && settings.include_own {
        sources.push(RecommendationSource::Own);
    }
    if recipe.is_favorite && settings.include_favorite_recipes {
        sources.push(RecommendationSource::FavoriteRecipe);
    }
    if recipe.is_favorite_author && settings.include_favorite_accounts {
        sources.push(RecommendationSource::FavoriteAccount);
    }
    if !recipe.is_owner && recipe.visibility == "public" && settings.include_other_public {
        sources.push(RecommendationSource::OtherPublic);
    }
    sources.sort_by_key(|source| std::cmp::Reverse(source_points(*source)));
    sources
}

fn source_points(source: RecommendationSource) -> i32 {
    match source {
        RecommendationSource::Own => 100,
        RecommendationSource::FavoriteRecipe | RecommendationSource::FavoriteAccount => 75,
        RecommendationSource::OtherPublic => 25,
    }
}

fn status_base(status: RecommendationStatus) -> i32 {
    match status {
        RecommendationStatus::Ready => 750,
        RecommendationStatus::Verify => 500,
        RecommendationStatus::Shop => 250,
        RecommendationStatus::Blocked => 0,
    }
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
            author_handle: "author-handle".into(),
            author_nickname: None,
            title: title.into(),
            description: String::new(),
            instructions: String::new(),
            visibility: "private".into(),
            difficulty: None,
            estimated_calories: None,
            version: 1,
            ingredients: ingredients
                .iter()
                .map(|name| IngredientInput {
                    name: (*name).into(),
                    quantity_note: String::new(),
                    canonical_ingredient_id: None,
                })
                .collect(),
            is_favorite: false,
            is_favorite_author: false,
            source: None,
            cover_image: None,
        }
    }

    fn settings() -> RecommendationSettings {
        RecommendationSettings::default()
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
            auto_add_to_shopping_list_when_out: false,
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
            &settings(),
            now,
        );
        assert_eq!(results[0].recipe.title, "Egg toast");
        assert_eq!(results[0].status, RecommendationStatus::Verify);
        assert_eq!(results[0].available, vec!["egg"]);
        assert_eq!(results[0].verify, vec!["bread"]);
        assert_eq!(
            results[0].score_breakdown,
            ScoreBreakdown {
                status_base: 500,
                source: 100,
                availability: 40,
                desired: 40,
                certainty: 10,
                simplicity: 9,
            }
        );
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
            &settings(),
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
            &settings(),
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
            &settings(),
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
            &settings(),
            now,
        );
        assert_eq!(results[0].status, RecommendationStatus::Verify);
        assert_eq!(results[0].verify, vec!["milk"]);
        assert!(results[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("expired on 2026-11-03")));
        assert!(results[0]
            .reason_details
            .contains(&RecommendationReason::IngredientExpired {
                ingredient: "milk".into(),
                expires_on: "2026-11-03".into(),
            }));
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
            &settings(),
            now,
        );
        assert_eq!(results[0].status, RecommendationStatus::Blocked);
        assert!(results[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("expires today")));
        assert!(results[0].reason_details.contains(
            &RecommendationReason::IngredientExpiresToday {
                ingredient: "milk".into(),
            }
        ));
        assert!(results[0]
            .reason_details
            .contains(&RecommendationReason::PantryOnlyBlocked));
    }

    #[test]
    fn favorite_recipe_and_author_sources_do_not_stack_points() {
        let mut candidate = recipe("1", "Favorite toast", &["bread"]);
        candidate.is_owner = false;
        candidate.visibility = "public".into();
        candidate.is_favorite = true;
        candidate.is_favorite_author = true;
        let results = recommend(
            vec![candidate],
            &[pantry(
                "bread",
                PantryStatus::Have,
                Some("2026-01-19T00:00:00Z"),
            )],
            &RecommendationRequest::default(),
            &settings(),
            DateTime::parse_from_rfc3339("2026-01-20T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        );
        assert_eq!(
            results[0].matched_sources,
            vec![
                RecommendationSource::FavoriteRecipe,
                RecommendationSource::FavoriteAccount,
                RecommendationSource::OtherPublic,
            ]
        );
        assert_eq!(
            results[0].primary_source,
            RecommendationSource::FavoriteRecipe
        );
        assert_eq!(results[0].score_breakdown.source, 75);
    }

    #[test]
    fn disabled_sources_remove_candidates_before_scoring() {
        let own = recipe("1", "Own", &["bread"]);
        let mut favorite_author = recipe("2", "Favorite author", &["bread"]);
        favorite_author.is_owner = false;
        favorite_author.visibility = "public".into();
        favorite_author.is_favorite_author = true;
        let settings = RecommendationSettings {
            include_favorite_recipes: false,
            include_own: false,
            include_favorite_accounts: true,
            include_other_public: false,
            verification_days: 14,
        };
        let results = recommend(
            vec![own, favorite_author],
            &[],
            &RecommendationRequest::default(),
            &settings,
            Utc::now(),
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].recipe.title, "Favorite author");
        assert_eq!(
            results[0].primary_source,
            RecommendationSource::FavoriteAccount
        );
    }

    #[test]
    fn status_bases_dominate_extras_and_scores_stay_bounded() {
        let now = DateTime::parse_from_rfc3339("2026-01-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut ready = recipe("ready", "Ready", &["bread"]);
        ready.is_favorite = false;
        let mut verify = recipe("verify", "Verify", &["milk"]);
        verify.is_favorite = true;
        let mut shop = recipe("shop", "Shop", &["egg"]);
        shop.is_favorite = true;
        let results = recommend(
            vec![shop, verify, ready],
            &[
                pantry("bread", PantryStatus::Have, Some("2026-01-19T00:00:00Z")),
                pantry("milk", PantryStatus::Unknown, Some("2026-01-19T00:00:00Z")),
            ],
            &RecommendationRequest {
                preferred: vec!["milk".into(), "egg".into()],
                ..Default::default()
            },
            &settings(),
            now,
        );
        assert_eq!(
            results
                .iter()
                .map(|result| result.status)
                .collect::<Vec<_>>(),
            vec![
                RecommendationStatus::Ready,
                RecommendationStatus::Verify,
                RecommendationStatus::Shop,
            ]
        );
        assert!(results
            .iter()
            .all(|result| (0..=999).contains(&result.score)));
        assert!(results[0].score > results[1].score);
        assert!(results[1].score > results[2].score);
    }

    #[test]
    fn bounded_ranking_matches_the_unbounded_prefix() {
        let recipes = (0..20)
            .map(|index| {
                recipe(
                    &index.to_string(),
                    &format!("Recipe {index:02}"),
                    &["bread"],
                )
            })
            .collect::<Vec<_>>();
        let pantry = [pantry(
            "bread",
            PantryStatus::Have,
            Some("2026-01-19T00:00:00Z"),
        )];
        let now = DateTime::parse_from_rfc3339("2026-01-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let all = recommend(
            recipes.clone(),
            &pantry,
            &RecommendationRequest::default(),
            &settings(),
            now,
        );
        let limited = recommend_limited(
            recipes,
            &pantry,
            &RecommendationRequest::default(),
            &settings(),
            now,
            5,
        );
        assert_eq!(
            limited
                .iter()
                .map(|result| &result.recipe.id)
                .collect::<Vec<_>>(),
            all.iter()
                .take(5)
                .map(|result| &result.recipe.id)
                .collect::<Vec<_>>()
        );
    }
}
