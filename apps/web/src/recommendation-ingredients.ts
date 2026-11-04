import { ingredientIdentity } from './ingredients'
import type { IngredientReference } from './types'
import {
  RECOMMENDATION_FILTER_LIMIT,
  validateRecommendationControls,
  type RecommendationControls,
} from './recommendation-url'

export type RecommendationIngredientBucket = 'preferred' | 'required' | 'excluded'

export type RecommendationIngredientMoveOutcome =
  | 'moved'
  | 'already-present'
  | 'source-missing'
  | 'target-full'

export interface RecommendationIngredientMoveResult {
  controls: RecommendationControls
  ingredient: IngredientReference | null
  outcome: RecommendationIngredientMoveOutcome
  focusIdentity: string
}

export interface RecommendationIngredientMoveRequest {
  from: RecommendationIngredientBucket
  to: RecommendationIngredientBucket
  identity: string
  ingredient: IngredientReference
}

export type RecommendationIngredientAddResult =
  | {
    controls: RecommendationControls
    outcome: 'added' | 'already-present'
    request: null
  }
  | {
    controls: RecommendationControls
    outcome: 'move-required'
    request: RecommendationIngredientMoveRequest
  }
  | {
    controls: RecommendationControls
    outcome: 'target-full'
    request: null
  }

export type RecommendationIngredientBatchResult =
  | {
    controls: RecommendationControls
    confirmations: RecommendationIngredientMoveRequest[]
    outcome: 'ready'
  }
  | {
    controls: RecommendationControls
    confirmations: []
    outcome: 'target-full'
    ingredient: IngredientReference
  }
  | {
    controls: RecommendationControls
    confirmations: []
    outcome: 'oversized'
  }

const buckets: RecommendationIngredientBucket[] = ['preferred', 'required', 'excluded']

export function moveRecommendationIngredient(
  controls: RecommendationControls,
  identity: string,
  from: RecommendationIngredientBucket,
  to: RecommendationIngredientBucket,
): RecommendationIngredientMoveResult {
  const ingredient = controls[from].find((value) => ingredientIdentity(value) === identity) ?? null
  if (!ingredient) {
    return {
      controls,
      ingredient: null,
      outcome: 'source-missing',
      focusIdentity: identity,
    }
  }

  const targetHasIdentity = controls[to].some((value) => ingredientIdentity(value) === identity)
  if (!targetHasIdentity && controls[to].length >= RECOMMENDATION_FILTER_LIMIT) {
    return {
      controls,
      ingredient,
      outcome: 'target-full',
      focusIdentity: identity,
    }
  }
  return {
    controls: {
      ...controls,
      [from]: controls[from].filter((value) => ingredientIdentity(value) !== identity),
      [to]: targetHasIdentity ? controls[to] : [...controls[to], ingredient],
    },
    ingredient,
    outcome: targetHasIdentity ? 'already-present' : 'moved',
    focusIdentity: identity,
  }
}

export function addOrRequestMove(
  controls: RecommendationControls,
  ingredient: IngredientReference,
  to: RecommendationIngredientBucket,
): RecommendationIngredientAddResult {
  const identity = ingredientIdentity(ingredient)
  if (controls[to].some((value) => ingredientIdentity(value) === identity)) {
    return { controls, outcome: 'already-present', request: null }
  }

  const from = buckets.find((bucket) =>
    bucket !== to && controls[bucket].some((value) => ingredientIdentity(value) === identity)
  )
  if (from) {
    const existing = controls[from].find((value) => ingredientIdentity(value) === identity)!
    return {
      controls,
      outcome: 'move-required',
      request: { from, to, identity, ingredient: existing },
    }
  }

  if (controls[to].length >= RECOMMENDATION_FILTER_LIMIT) {
    return { controls, outcome: 'target-full', request: null }
  }

  return {
    controls: { ...controls, [to]: [...controls[to], ingredient] },
    outcome: 'added',
    request: null,
  }
}

export function prepareRecommendationIngredientBatch(
  controls: RecommendationControls,
  additions: IngredientReference[],
  to: RecommendationIngredientBucket,
): RecommendationIngredientBatchResult {
  let next = controls
  const confirmations: RecommendationIngredientMoveRequest[] = []
  const queuedIdentities = new Set<string>()

  for (const addition of additions) {
    const result = addOrRequestMove(next, addition, to)
    if (result.outcome === 'target-full') {
      return {
        controls,
        confirmations: [],
        outcome: 'target-full',
        ingredient: addition,
      }
    }
    if (result.outcome !== 'move-required') {
      next = result.controls
      continue
    }
    if (!queuedIdentities.has(result.request.identity)) {
      queuedIdentities.add(result.request.identity)
      confirmations.push(result.request)
    }
    const move = moveRecommendationIngredient(
      next,
      result.request.identity,
      result.request.from,
      result.request.to,
    )
    if (move.outcome === 'target-full') {
      return {
        controls,
        confirmations: [],
        outcome: 'target-full',
        ingredient: result.request.ingredient,
      }
    }
    next = move.controls
  }

  const validation = validateRecommendationControls(next)
  if (validation.status === 'oversized') {
    return { controls, confirmations: [], outcome: 'oversized' }
  }
  return {
    controls: validation.controls,
    confirmations,
    outcome: 'ready',
  }
}
