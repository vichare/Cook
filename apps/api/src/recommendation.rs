use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet},
};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;

use crate::{
    ingredients::normalize_ingredient,
    models::{
        IngredientReference, IngredientScope, PantryItem, PantryStatus, Recipe,
        RecommendationRequest, RecommendationSettings,
    },
    shopping::{exact_choice_key, ActiveShoppingCoverage},
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
    pub in_shopping_list: Vec<String>,
    pub reasons: Vec<String>,
    pub reason_details: Vec<RecommendationReason>,
    pub score: i32,
    pub score_breakdown: ScoreBreakdown,
    pub primary_source: RecommendationSource,
    pub matched_sources: Vec<RecommendationSource>,
    pub desired_matches: Vec<String>,
    pub ranking_intent: RankingIntent,
    pub ingredient_reasons: Vec<IngredientReason>,
    pub ingredient_group_reasons: Vec<IngredientGroupReason>,
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

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IngredientGroupAction {
    Available,
    Verify,
    ShoppingPlanned,
    Missing,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IngredientReason {
    pub ingredient: String,
    pub code: IngredientReasonCode,
    pub expires_on: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IngredientGroupReason {
    pub group_id: String,
    pub label: String,
    pub optional: bool,
    pub effective: bool,
    pub action: IngredientGroupAction,
    pub code: IngredientReasonCode,
    pub selected_alternative_id: Option<String>,
    pub alternatives: Vec<IngredientReason>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedControls {
    pub preferred: Vec<String>,
    pub required: Vec<String>,
    pub excluded: Vec<String>,
    pub ingredient_scope: IngredientScope,
    pub pantry_only: bool,
    pub include_own: bool,
    pub include_favorite_recipes: bool,
    pub include_favorite_accounts: bool,
    pub include_other_public: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RankingIntent {
    pub version: &'static str,
    pub scope_eligible: bool,
    pub desired_tier: IngredientGroupAction,
    pub desired_matched: usize,
    pub desired_total: usize,
    pub desired_coverage: u16,
    pub fulfillment: RecommendationStatus,
    pub unplanned_missing: usize,
    pub shopping_covered: usize,
    pub verify: usize,
    pub effective_groups: usize,
    pub source: RecommendationSource,
    pub available: usize,
    pub available_coverage: u16,
    pub reasons: Vec<RankingReason>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "camelCase")]
pub enum RankingReason {
    ScopeEligible {
        eligible: bool,
    },
    DesiredCoverage {
        matched: usize,
        total: usize,
    },
    Fulfillment {
        available: usize,
        verify: usize,
        shopping_covered: usize,
        unplanned_missing: usize,
    },
    Source {
        source: RecommendationSource,
    },
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
    IngredientScopeBlocked {
        scope: IngredientScope,
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
    recommend_inner(
        recipes,
        pantry,
        &ActiveShoppingCoverage::default(),
        request,
        settings,
        now,
        None,
    )
}

pub fn recommend_limited(
    recipes: Vec<Recipe>,
    pantry: &[PantryItem],
    request: &RecommendationRequest,
    settings: &RecommendationSettings,
    now: DateTime<Utc>,
    limit: usize,
) -> Vec<Recommendation> {
    recommend_inner(
        recipes,
        pantry,
        &ActiveShoppingCoverage::default(),
        request,
        settings,
        now,
        Some(limit),
    )
}

pub(crate) fn recommend_limited_with_shopping(
    recipes: Vec<Recipe>,
    pantry: &[PantryItem],
    shopping: &ActiveShoppingCoverage,
    request: &RecommendationRequest,
    settings: &RecommendationSettings,
    now: DateTime<Utc>,
    limit: usize,
) -> Vec<Recommendation> {
    recommend_inner(
        recipes,
        pantry,
        shopping,
        request,
        settings,
        now,
        Some(limit),
    )
}

fn recommend_inner(
    recipes: Vec<Recipe>,
    pantry: &[PantryItem],
    shopping: &ActiveShoppingCoverage,
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
        let group_keys = recipe
            .ingredient_groups
            .iter()
            .map(|group| {
                group
                    .alternatives
                    .iter()
                    .map(|alternative| {
                        ingredient_key(
                            alternative.canonical_ingredient_id.as_deref(),
                            &alternative.name,
                        )
                    })
                    .collect::<HashSet<_>>()
            })
            .collect::<Vec<_>>();
        if group_keys
            .iter()
            .any(|identities| !excluded.is_disjoint(identities))
        {
            return None;
        }
        let required_assignment = assign_distinct_groups(&required, &group_keys, None, None, true)?;
        let matched_sources = matched_sources(&recipe, settings);
        let primary_source = *matched_sources.first()?;

        let evaluated_groups = recipe
            .ingredient_groups
            .iter()
            .map(|group| evaluate_group(group, &pantry, shopping, stale_before, now.date_naive()))
            .collect::<Vec<_>>();
        let ingredient_scope = request.ingredient_scope.unwrap_or_default();
        let desired_assignment = assign_distinct_groups(
            &preferred,
            &group_keys,
            Some(
                &evaluated_groups
                    .iter()
                    .map(|group| action_qualifies_scope(group.action, ingredient_scope))
                    .collect::<Vec<_>>(),
            ),
            Some(
                &evaluated_groups
                    .iter()
                    .map(|group| action_rank(group.action))
                    .collect::<Vec<_>>(),
            ),
            false,
        )
        .unwrap_or_default();
        let mut promoted_groups = required_assignment
            .values()
            .chain(desired_assignment.values())
            .copied()
            .collect::<HashSet<_>>();
        promoted_groups.shrink_to_fit();

        let mut available = Vec::new();
        let mut verify = Vec::new();
        let mut missing = Vec::new();
        let mut in_shopping_list = Vec::new();
        let mut ingredient_reasons = Vec::new();
        let mut ingredient_group_reasons = Vec::new();
        let mut effective_count = 0usize;
        let mut available_count = 0usize;
        let mut verify_count = 0usize;
        let mut shopping_covered = 0usize;
        let mut unplanned_missing = 0usize;
        for (group_index, (group, evaluated)) in recipe
            .ingredient_groups
            .iter()
            .zip(evaluated_groups.iter())
            .enumerate()
        {
            let effective = !group.optional || promoted_groups.contains(&group_index);
            if effective {
                effective_count += 1;
                match evaluated.action {
                    IngredientGroupAction::Available => {
                        available_count += 1;
                        available.push(evaluated.label.clone());
                    }
                    IngredientGroupAction::Verify => {
                        verify_count += 1;
                        verify.push(evaluated.label.clone());
                    }
                    IngredientGroupAction::ShoppingPlanned => {
                        shopping_covered += 1;
                        missing.push(evaluated.label.clone());
                        in_shopping_list.push(evaluated.label.clone());
                    }
                    IngredientGroupAction::Missing => {
                        unplanned_missing += 1;
                        missing.push(evaluated.label.clone());
                    }
                }
            }
            ingredient_reasons.push(evaluated.selected_reason.clone());
            ingredient_group_reasons.push(IngredientGroupReason {
                group_id: group.id.clone(),
                label: evaluated.label.clone(),
                optional: group.optional,
                effective,
                action: evaluated.action,
                code: evaluated.selected_reason.code,
                selected_alternative_id: evaluated.selected_alternative_id.clone(),
                alternatives: evaluated.alternatives.clone(),
            });
        }

        let scope_eligible = ingredient_group_reasons
            .iter()
            .filter(|reason| reason.effective)
            .all(|reason| action_qualifies_scope(reason.action, ingredient_scope));
        let status = if !scope_eligible {
            RecommendationStatus::Blocked
        } else if unplanned_missing > 0 || shopping_covered > 0 {
            RecommendationStatus::Shop
        } else if verify_count > 0 {
            RecommendationStatus::Verify
        } else {
            RecommendationStatus::Ready
        };
        let mut desired_assignments = desired_assignment.into_iter().collect::<Vec<_>>();
        desired_assignments.sort_by(|(left, _), (right, _)| left.cmp(right));
        let desired_matches = desired_assignments
            .iter()
            .map(|(identity, _)| {
                request
                    .preferred
                    .iter()
                    .find(|reference| {
                        ingredient_key(reference.canonical_ingredient_id(), reference.name())
                            == *identity
                    })
                    .map(|reference| reference.name().to_string())
                    .unwrap_or_else(|| identity.clone())
            })
            .collect::<Vec<_>>();
        let preferred_matches = i32::try_from(desired_matches.len()).unwrap_or(i32::MAX);
        let legacy_preferred_matches = i32::try_from(
            recipe
                .ingredients
                .iter()
                .filter(|ingredient| {
                    preferred.contains(&ingredient_key(
                        ingredient.canonical_ingredient_id.as_deref(),
                        &ingredient.name,
                    ))
                })
                .count(),
        )
        .unwrap_or(i32::MAX);
        let preferred_count = i32::try_from(preferred.len()).unwrap_or(i32::MAX);
        let ingredient_count = i32::try_from(recipe.ingredients.len())
            .unwrap_or(i32::MAX)
            .max(1);
        let legacy_available_count = i32::try_from(available_count).unwrap_or(i32::MAX);
        let missing_count = i32::try_from(unplanned_missing + shopping_covered).unwrap_or(i32::MAX);
        let breakdown = ScoreBreakdown {
            status_base: status_base(status),
            source: source_points(primary_source),
            availability: (legacy_available_count * 80 / ingredient_count).clamp(0, 80),
            desired: if preferred_count == 0 {
                0
            } else {
                (legacy_preferred_matches * 40 / preferred_count).clamp(0, 40)
            },
            certainty: (legacy_available_count * 20 / ingredient_count).clamp(0, 20),
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
            available_count, effective_count
        )];
        let mut reason_details = vec![RecommendationReason::ConfirmedIngredients {
            confirmed: available_count,
            total: effective_count,
        }];
        if preferred_matches > 0 {
            reasons.push(format!(
                "{preferred_matches} preferred ingredient(s) included"
            ));
            reason_details.push(RecommendationReason::PreferredIngredients {
                count: preferred_matches as usize,
            });
        }
        for group_reason in &ingredient_group_reasons {
            if !group_reason.effective {
                continue;
            }
            for alternative in &group_reason.alternatives {
                match alternative.code {
                    IngredientReasonCode::Expired => {
                        if let Some(expires_on) = &alternative.expires_on {
                            reasons.push(format!(
                                "{}: expired on {expires_on}",
                                alternative.ingredient
                            ));
                            reason_details.push(RecommendationReason::IngredientExpired {
                                ingredient: alternative.ingredient.clone(),
                                expires_on: expires_on.clone(),
                            });
                        }
                    }
                    IngredientReasonCode::ExpiresToday => {
                        reasons.push(format!("{}: expires today", alternative.ingredient));
                        reason_details.push(RecommendationReason::IngredientExpiresToday {
                            ingredient: alternative.ingredient.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        if !scope_eligible {
            reasons.push(
                "Blocked by ingredient scope because effective groups are not covered".into(),
            );
            reason_details.push(RecommendationReason::IngredientScopeBlocked {
                scope: ingredient_scope,
            });
            if ingredient_scope == IngredientScope::PantryOnly {
                reason_details.push(RecommendationReason::PantryOnlyBlocked);
            }
        }
        let desired_tier = desired_assignments
            .iter()
            .map(|(_, group_index)| evaluated_groups[*group_index].action)
            .max_by_key(|action| action_rank(*action))
            .unwrap_or(IngredientGroupAction::Missing);
        let desired_total = preferred.len();
        let desired_matched = desired_matches.len();
        let desired_coverage = coverage_basis_points(desired_matched, desired_total);
        let available_coverage = coverage_basis_points(available_count, effective_count);
        let ranking_intent = RankingIntent {
            version: "intent-v1",
            scope_eligible,
            desired_tier,
            desired_matched,
            desired_total,
            desired_coverage,
            fulfillment: status,
            unplanned_missing,
            shopping_covered,
            verify: verify_count,
            effective_groups: effective_count,
            source: primary_source,
            available: available_count,
            available_coverage,
            reasons: vec![
                RankingReason::ScopeEligible {
                    eligible: scope_eligible,
                },
                RankingReason::DesiredCoverage {
                    matched: desired_matched,
                    total: desired_total,
                },
                RankingReason::Fulfillment {
                    available: available_count,
                    verify: verify_count,
                    shopping_covered,
                    unplanned_missing,
                },
                RankingReason::Source {
                    source: primary_source,
                },
            ],
        };
        Some(Recommendation {
            recipe,
            status,
            available,
            verify,
            missing,
            in_shopping_list,
            reasons,
            reason_details,
            score,
            score_breakdown: breakdown,
            primary_source,
            matched_sources,
            desired_matches,
            ranking_intent,
            ingredient_reasons,
            ingredient_group_reasons,
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
                ingredient_scope,
                pantry_only: ingredient_scope == IngredientScope::PantryOnly,
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

fn assign_distinct_groups(
    requested: &HashSet<String>,
    group_keys: &[HashSet<String>],
    qualified_groups: Option<&[bool]>,
    group_priorities: Option<&[i32]>,
    require_all: bool,
) -> Option<HashMap<String, usize>> {
    fn augment(
        identity_index: usize,
        identities: &[String],
        group_keys: &[HashSet<String>],
        qualified_groups: Option<&[bool]>,
        group_priorities: Option<&[i32]>,
        seen: &mut [bool],
        group_matches: &mut [Option<usize>],
    ) -> bool {
        let mut group_indices = (0..group_keys.len()).collect::<Vec<_>>();
        group_indices.sort_by_key(|index| {
            (
                std::cmp::Reverse(group_priorities.map_or(0, |priorities| priorities[*index])),
                *index,
            )
        });
        for group_index in group_indices {
            let group_identities = &group_keys[group_index];
            if seen[group_index]
                || qualified_groups.is_some_and(|qualified| !qualified[group_index])
                || !group_identities.contains(&identities[identity_index])
            {
                continue;
            }
            seen[group_index] = true;
            if group_matches[group_index].is_none()
                || augment(
                    group_matches[group_index].expect("checked above"),
                    identities,
                    group_keys,
                    qualified_groups,
                    group_priorities,
                    seen,
                    group_matches,
                )
            {
                group_matches[group_index] = Some(identity_index);
                return true;
            }
        }
        false
    }
    let mut identities = requested.iter().cloned().collect::<Vec<_>>();
    identities.sort_by(|left, right| {
        let candidate_count = |identity: &String| {
            group_keys
                .iter()
                .enumerate()
                .filter(|(index, group)| {
                    qualified_groups.is_none_or(|qualified| qualified[*index])
                        && group.contains(identity)
                })
                .count()
        };
        candidate_count(left)
            .cmp(&candidate_count(right))
            .then_with(|| left.cmp(right))
    });
    let mut group_matches = vec![None; group_keys.len()];
    let mut matched = 0usize;
    for identity_index in 0..identities.len() {
        if augment(
            identity_index,
            &identities,
            group_keys,
            qualified_groups,
            group_priorities,
            &mut vec![false; group_keys.len()],
            &mut group_matches,
        ) {
            matched += 1;
        }
    }
    if require_all && matched != identities.len() {
        return None;
    }
    Some(
        group_matches
            .into_iter()
            .enumerate()
            .filter_map(|(group_index, identity_index)| {
                identity_index
                    .map(|identity_index| (identities[identity_index].clone(), group_index))
            })
            .collect(),
    )
}

#[derive(Clone)]
struct EvaluatedGroup {
    label: String,
    action: IngredientGroupAction,
    selected_alternative_id: Option<String>,
    selected_reason: IngredientReason,
    alternatives: Vec<IngredientReason>,
}

fn evaluate_group(
    group: &crate::models::IngredientGroupInput,
    pantry: &HashMap<String, &PantryItem>,
    shopping: &ActiveShoppingCoverage,
    stale_before: DateTime<Utc>,
    today: NaiveDate,
) -> EvaluatedGroup {
    let label = group
        .alternatives
        .iter()
        .map(|alternative| alternative.name.as_str())
        .collect::<Vec<_>>()
        .join(" / ");
    let mut alternatives = Vec::with_capacity(group.alternatives.len());
    let mut selected: Option<(usize, String, IngredientReason)> = None;
    let mut concrete_shopping_alternative = None;
    let choice_identities = group
        .alternatives
        .iter()
        .map(|alternative| {
            alternative
                .canonical_ingredient_id
                .as_deref()
                .map(|id| format!("id:{id}"))
                .unwrap_or_else(|| format!("name:{}", normalize_ingredient(&alternative.name)))
        })
        .collect::<Vec<_>>();
    for alternative in &group.alternatives {
        let key = ingredient_key(
            alternative.canonical_ingredient_id.as_deref(),
            &alternative.name,
        );
        let reason = evaluate_ingredient(
            &alternative.name,
            pantry.get(&key).copied(),
            stale_before,
            today,
        );
        let rank = reason_rank(reason.code);
        if selected
            .as_ref()
            .is_none_or(|(selected_rank, _, _)| rank > *selected_rank)
        {
            selected = Some((rank, alternative.id.clone(), reason.clone()));
        }
        let concrete_key = crate::shopping::dedupe_key(
            alternative.canonical_ingredient_id.as_deref(),
            &normalize_ingredient(&alternative.name),
        );
        if concrete_shopping_alternative.is_none()
            && shopping.concrete_identity_keys.contains(&concrete_key)
        {
            concrete_shopping_alternative = Some(alternative.id.clone());
        }
        alternatives.push(reason);
    }
    let (mut selected_alternative_id, selected_reason) = selected
        .map(|(_, id, reason)| (Some(id), reason))
        .unwrap_or((
            None,
            IngredientReason {
                ingredient: label.clone(),
                code: IngredientReasonCode::Missing,
                expires_on: None,
            },
        ));
    let pantry_action = match selected_reason.code {
        IngredientReasonCode::Available => IngredientGroupAction::Available,
        IngredientReasonCode::Unknown
        | IngredientReasonCode::Stale
        | IngredientReasonCode::ExpiresToday
        | IngredientReasonCode::Expired => IngredientGroupAction::Verify,
        IngredientReasonCode::Out | IngredientReasonCode::Missing => IngredientGroupAction::Missing,
    };
    let shopping_planned = concrete_shopping_alternative.is_some()
        || shopping
            .exact_choice_keys
            .contains(&exact_choice_key(choice_identities));
    let action = if pantry_action == IngredientGroupAction::Missing && shopping_planned {
        if concrete_shopping_alternative.is_some() {
            selected_alternative_id = concrete_shopping_alternative;
        }
        IngredientGroupAction::ShoppingPlanned
    } else {
        pantry_action
    };
    EvaluatedGroup {
        label,
        action,
        selected_alternative_id,
        selected_reason,
        alternatives,
    }
}

fn evaluate_ingredient(
    name: &str,
    item: Option<&PantryItem>,
    stale_before: DateTime<Utc>,
    today: NaiveDate,
) -> IngredientReason {
    let (code, expires_on) = match item {
        None => (IngredientReasonCode::Missing, None),
        Some(item) if item.status == PantryStatus::Out => {
            (IngredientReasonCode::Out, item.expires_on.clone())
        }
        Some(item) if item.status == PantryStatus::Unknown => {
            (IngredientReasonCode::Unknown, item.expires_on.clone())
        }
        Some(item) => {
            let expiry = item.expires_on.as_deref().and_then(parse_date);
            if expiry.is_some_and(|date| date < today) {
                (IngredientReasonCode::Expired, item.expires_on.clone())
            } else if expiry.is_some_and(|date| date == today) {
                (IngredientReasonCode::ExpiresToday, item.expires_on.clone())
            } else if !is_fresh(item, stale_before) {
                (IngredientReasonCode::Stale, item.expires_on.clone())
            } else {
                (IngredientReasonCode::Available, item.expires_on.clone())
            }
        }
    };
    IngredientReason {
        ingredient: name.to_string(),
        code,
        expires_on,
    }
}

fn reason_rank(code: IngredientReasonCode) -> usize {
    match code {
        IngredientReasonCode::Available => 3,
        IngredientReasonCode::Unknown
        | IngredientReasonCode::Stale
        | IngredientReasonCode::ExpiresToday
        | IngredientReasonCode::Expired => 2,
        IngredientReasonCode::Out | IngredientReasonCode::Missing => 1,
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
    let left_rank = &left.ranking_intent;
    let right_rank = &right.ranking_intent;
    right_rank
        .scope_eligible
        .cmp(&left_rank.scope_eligible)
        .then_with(|| {
            if left_rank.desired_total > 0 {
                action_rank(right_rank.desired_tier).cmp(&action_rank(left_rank.desired_tier))
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| {
            if left_rank.desired_total > 0 {
                right_rank.desired_matched.cmp(&left_rank.desired_matched)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| {
            if left_rank.desired_total > 0 {
                right_rank.desired_coverage.cmp(&left_rank.desired_coverage)
            } else {
                Ordering::Equal
            }
        })
        .then_with(|| status_rank(right_rank.fulfillment).cmp(&status_rank(left_rank.fulfillment)))
        .then_with(|| {
            left_rank
                .unplanned_missing
                .cmp(&right_rank.unplanned_missing)
        })
        .then_with(|| left_rank.shopping_covered.cmp(&right_rank.shopping_covered))
        .then_with(|| left_rank.verify.cmp(&right_rank.verify))
        .then_with(|| left_rank.effective_groups.cmp(&right_rank.effective_groups))
        .then_with(|| source_rank(right_rank.source).cmp(&source_rank(left_rank.source)))
        .then_with(|| {
            right_rank
                .available_coverage
                .cmp(&left_rank.available_coverage)
        })
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

fn action_rank(action: IngredientGroupAction) -> i32 {
    match action {
        IngredientGroupAction::Available => 4,
        IngredientGroupAction::Verify => 3,
        IngredientGroupAction::ShoppingPlanned => 2,
        IngredientGroupAction::Missing => 1,
    }
}

fn source_rank(source: RecommendationSource) -> i32 {
    match source {
        RecommendationSource::Own => 3,
        RecommendationSource::FavoriteRecipe | RecommendationSource::FavoriteAccount => 2,
        RecommendationSource::OtherPublic => 1,
    }
}

fn action_qualifies_scope(action: IngredientGroupAction, scope: IngredientScope) -> bool {
    match scope {
        IngredientScope::Any => true,
        IngredientScope::PantryOnly => action == IngredientGroupAction::Available,
        IngredientScope::PantryOrList => matches!(
            action,
            IngredientGroupAction::Available | IngredientGroupAction::ShoppingPlanned
        ),
    }
}

fn coverage_basis_points(count: usize, total: usize) -> u16 {
    if total == 0 {
        0
    } else {
        u16::try_from(count.saturating_mul(10_000) / total)
            .unwrap_or(10_000)
            .min(10_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{IngredientAlternativeInput, IngredientGroupInput, IngredientInput};

    fn recipe(id: &str, title: &str, ingredients: &[&str]) -> Recipe {
        let groups = ingredients
            .iter()
            .enumerate()
            .map(|(index, name)| IngredientGroupInput {
                id: format!("group-{index}"),
                optional: false,
                note: String::new(),
                sort_order: index as i64,
                alternatives: vec![IngredientAlternativeInput {
                    id: format!("alternative-{index}"),
                    name: (*name).into(),
                    quantity_note: String::new(),
                    canonical_ingredient_id: None,
                    sort_order: 0,
                }],
            })
            .collect();
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
            publication_intent: "private".into(),
            moderation_state: "clear".into(),
            moderation_review_state: "unreviewed".into(),
            moderation_version: 1,
            difficulty: None,
            estimated_calories: None,
            version: 1,
            ingredient_model: "groups-v1".into(),
            ingredient_groups: groups,
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
            aliases: Vec::new(),
            auto_add_to_shopping_list_when_out: false,
            version: 1,
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
                ingredient_scope: Some(IngredientScope::PantryOnly),
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
                ingredient_scope: Some(IngredientScope::PantryOnly),
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
                ingredient_scope: Some(IngredientScope::PantryOnly),
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
    fn desired_intent_precedes_fulfillment_and_scores_are_compatibility_only() {
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
                RecommendationStatus::Verify,
                RecommendationStatus::Shop,
                RecommendationStatus::Ready,
            ]
        );
        assert!(results
            .iter()
            .all(|result| (0..=999).contains(&result.score)));
        assert!(results[2].score > results[0].score);
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

    #[test]
    fn alternatives_choose_best_state_and_avoid_matches_any_alternative() {
        let mut candidate = recipe("or", "OR", &["butter"]);
        candidate.ingredient_groups[0]
            .alternatives
            .push(IngredientAlternativeInput {
                id: "oil".into(),
                name: "olive oil".into(),
                quantity_note: String::new(),
                canonical_ingredient_id: None,
                sort_order: 1,
            });
        let now = Utc::now();
        let available = recommend(
            vec![candidate.clone()],
            &[pantry(
                "olive oil",
                PantryStatus::Have,
                Some(&now.to_rfc3339()),
            )],
            &RecommendationRequest::default(),
            &settings(),
            now,
        );
        assert_eq!(available[0].status, RecommendationStatus::Ready);
        assert_eq!(available[0].available, vec!["butter / olive oil"]);
        assert_eq!(
            available[0].ingredient_group_reasons[0].selected_alternative_id,
            Some("oil".into())
        );
        assert!(recommend(
            vec![candidate],
            &[],
            &RecommendationRequest {
                excluded: vec![IngredientReference::from("butter")],
                ..Default::default()
            },
            &settings(),
            now,
        )
        .is_empty());
    }

    #[test]
    fn optional_groups_are_ignored_unless_required_promotes_them() {
        let mut candidate = recipe("optional", "Optional", &["salt", "basil"]);
        candidate.ingredient_groups[1].optional = true;
        let now = Utc::now();
        let pantry = [pantry("salt", PantryStatus::Have, Some(&now.to_rfc3339()))];
        let base = recommend(
            vec![candidate.clone()],
            &pantry,
            &RecommendationRequest::default(),
            &settings(),
            now,
        );
        assert_eq!(base[0].status, RecommendationStatus::Ready);
        assert!(!base[0].ingredient_group_reasons[1].effective);
        let promoted = recommend(
            vec![candidate],
            &pantry,
            &RecommendationRequest {
                required: vec![IngredientReference::from("basil")],
                ..Default::default()
            },
            &settings(),
            now,
        );
        assert_eq!(promoted[0].status, RecommendationStatus::Shop);
        assert!(promoted[0].ingredient_group_reasons[1].effective);
    }

    #[test]
    fn required_filters_need_distinct_group_assignments() {
        let mut candidate = recipe("must", "Must", &["butter"]);
        candidate.ingredient_groups[0]
            .alternatives
            .push(IngredientAlternativeInput {
                id: "oil".into(),
                name: "oil".into(),
                quantity_note: String::new(),
                canonical_ingredient_id: None,
                sort_order: 1,
            });
        assert!(recommend(
            vec![candidate],
            &[],
            &RecommendationRequest {
                required: vec![
                    IngredientReference::from("butter"),
                    IngredientReference::from("oil"),
                ],
                ..Default::default()
            },
            &settings(),
            Utc::now(),
        )
        .is_empty());
    }

    #[test]
    fn pantry_or_list_requires_exact_choice_or_concrete_coverage_and_verify_wins() {
        let mut candidate = recipe("or", "OR spread", &["butter"]);
        candidate.ingredient_groups[0]
            .alternatives
            .push(IngredientAlternativeInput {
                id: "oil".into(),
                name: "oil".into(),
                quantity_note: String::new(),
                canonical_ingredient_id: None,
                sort_order: 1,
            });
        let request = RecommendationRequest {
            ingredient_scope: Some(IngredientScope::PantryOrList),
            ..Default::default()
        };
        let exact_choice = ActiveShoppingCoverage {
            concrete_identity_keys: HashSet::new(),
            exact_choice_keys: HashSet::from([exact_choice_key([
                "name:butter".into(),
                "name:oil".into(),
            ])]),
        };
        let covered = recommend_limited_with_shopping(
            vec![candidate.clone()],
            &[],
            &exact_choice,
            &request,
            &settings(),
            Utc::now(),
            10,
        );
        assert!(covered[0].ranking_intent.scope_eligible);
        assert_eq!(covered[0].status, RecommendationStatus::Shop);
        assert_eq!(
            covered[0].ingredient_group_reasons[0].action,
            IngredientGroupAction::ShoppingPlanned
        );

        let wrong_choice = ActiveShoppingCoverage {
            concrete_identity_keys: HashSet::new(),
            exact_choice_keys: HashSet::from([exact_choice_key(["name:butter".into()])]),
        };
        let blocked = recommend_limited_with_shopping(
            vec![candidate.clone()],
            &[],
            &wrong_choice,
            &request,
            &settings(),
            Utc::now(),
            10,
        );
        assert!(!blocked[0].ranking_intent.scope_eligible);
        assert_eq!(blocked[0].status, RecommendationStatus::Blocked);

        let concrete = ActiveShoppingCoverage {
            concrete_identity_keys: HashSet::from(["n:oil".into()]),
            exact_choice_keys: HashSet::new(),
        };
        let concrete_covered = recommend_limited_with_shopping(
            vec![candidate.clone()],
            &[],
            &concrete,
            &request,
            &settings(),
            Utc::now(),
            10,
        );
        assert!(concrete_covered[0].ranking_intent.scope_eligible);

        let verify = recommend_limited_with_shopping(
            vec![candidate],
            &[pantry("butter", PantryStatus::Unknown, None)],
            &exact_choice,
            &request,
            &settings(),
            Utc::now(),
            10,
        );
        assert_eq!(
            verify[0].ingredient_group_reasons[0].action,
            IngredientGroupAction::Verify
        );
        assert!(!verify[0].ranking_intent.scope_eligible);
    }

    #[test]
    fn desired_and_must_promote_optional_groups_only_when_scope_qualified() {
        let now = Utc::now();
        let mut candidate = recipe("optional", "Optional basil", &["salt", "basil"]);
        candidate.ingredient_groups[1].optional = true;
        let pantry = [pantry("salt", PantryStatus::Have, Some(&now.to_rfc3339()))];
        let any = recommend(
            vec![candidate.clone()],
            &pantry,
            &RecommendationRequest {
                preferred: vec!["basil".into()],
                ..Default::default()
            },
            &settings(),
            now,
        );
        assert!(any[0].ingredient_group_reasons[1].effective);
        assert_eq!(any[0].ranking_intent.desired_matched, 1);
        assert_eq!(any[0].status, RecommendationStatus::Shop);

        let pantry_only = recommend(
            vec![candidate.clone()],
            &pantry,
            &RecommendationRequest {
                preferred: vec!["basil".into()],
                ingredient_scope: Some(IngredientScope::PantryOnly),
                ..Default::default()
            },
            &settings(),
            now,
        );
        assert!(!pantry_only[0].ingredient_group_reasons[1].effective);
        assert_eq!(pantry_only[0].ranking_intent.desired_matched, 0);
        assert!(pantry_only[0].ranking_intent.scope_eligible);

        let must = recommend(
            vec![candidate],
            &pantry,
            &RecommendationRequest {
                required: vec!["basil".into()],
                ingredient_scope: Some(IngredientScope::PantryOnly),
                ..Default::default()
            },
            &settings(),
            now,
        );
        assert!(must[0].ingredient_group_reasons[1].effective);
        assert!(!must[0].ranking_intent.scope_eligible);
    }

    #[test]
    fn desired_assignment_is_distinct_maximum_and_handles_one_hundred_inputs() {
        let mut overlap = recipe("overlap", "Overlap", &["a", "c"]);
        overlap.ingredient_groups[0]
            .alternatives
            .push(IngredientAlternativeInput {
                id: "b-0".into(),
                name: "b".into(),
                quantity_note: String::new(),
                canonical_ingredient_id: None,
                sort_order: 1,
            });
        overlap.ingredient_groups[1]
            .alternatives
            .push(IngredientAlternativeInput {
                id: "b-1".into(),
                name: "b".into(),
                quantity_note: String::new(),
                canonical_ingredient_id: None,
                sort_order: 1,
            });
        let matched = recommend(
            vec![overlap],
            &[],
            &RecommendationRequest {
                preferred: vec!["a".into(), "b".into(), "c".into()],
                ..Default::default()
            },
            &settings(),
            Utc::now(),
        );
        assert_eq!(matched[0].ranking_intent.desired_matched, 2);

        let names = (0..100)
            .map(|index| format!("item-{index}"))
            .collect::<Vec<_>>();
        let name_refs = names.iter().map(String::as_str).collect::<Vec<_>>();
        let hundred = recommend(
            vec![recipe("hundred", "Hundred", &name_refs)],
            &[],
            &RecommendationRequest {
                preferred: names.into_iter().map(IngredientReference::Name).collect(),
                ..Default::default()
            },
            &settings(),
            Utc::now(),
        );
        assert_eq!(hundred[0].ranking_intent.desired_matched, 100);
        assert_eq!(hundred[0].ranking_intent.desired_coverage, 10_000);
    }

    #[test]
    fn intent_comparator_is_transitive_and_heap_matches_every_prefix() {
        let now = DateTime::parse_from_rfc3339("2026-01-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let recipes = (0..60)
            .map(|index| {
                let ingredient = match index % 4 {
                    0 => "available",
                    1 => "verify",
                    2 => "desired",
                    _ => "missing",
                };
                let mut candidate = recipe(
                    &format!("{index:03}"),
                    &format!("Recipe {index:03}"),
                    &[ingredient],
                );
                if index % 5 == 0 {
                    candidate.is_owner = false;
                    candidate.visibility = "public".into();
                    candidate.is_favorite = true;
                }
                candidate
            })
            .collect::<Vec<_>>();
        let pantry = vec![
            pantry(
                "available",
                PantryStatus::Have,
                Some("2026-01-19T00:00:00Z"),
            ),
            pantry(
                "verify",
                PantryStatus::Unknown,
                Some("2026-01-19T00:00:00Z"),
            ),
        ];
        let request = RecommendationRequest {
            preferred: vec!["desired".into()],
            ..Default::default()
        };
        let all = recommend(recipes.clone(), &pantry, &request, &settings(), now);
        for left in &all {
            for right in &all {
                assert_eq!(
                    compare_recommendations(left, right),
                    compare_recommendations(right, left).reverse()
                );
            }
        }
        for window in all.windows(3) {
            assert_ne!(
                compare_recommendations(&window[0], &window[1]),
                Ordering::Greater
            );
            assert_ne!(
                compare_recommendations(&window[1], &window[2]),
                Ordering::Greater
            );
            assert_ne!(
                compare_recommendations(&window[0], &window[2]),
                Ordering::Greater
            );
        }
        for limit in 1..=all.len() {
            let limited =
                recommend_limited(recipes.clone(), &pantry, &request, &settings(), now, limit);
            assert_eq!(
                limited
                    .iter()
                    .map(|result| &result.recipe.id)
                    .collect::<Vec<_>>(),
                all.iter()
                    .take(limit)
                    .map(|result| &result.recipe.id)
                    .collect::<Vec<_>>()
            );
        }
    }
}
