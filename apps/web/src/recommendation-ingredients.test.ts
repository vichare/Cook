import { describe, expect, it } from 'vitest'
import {
  addOrRequestMove,
  moveRecommendationIngredient,
  prepareRecommendationIngredientBatch,
  type RecommendationIngredientBucket,
} from './recommendation-ingredients'
import {
  RECOMMENDATION_FILTER_LIMIT,
  recommendationControlsFromSettings,
  type RecommendationControls,
} from './recommendation-url'

const defaults = recommendationControlsFromSettings({
  includeFavoriteRecipes: true,
  includeOwn: true,
  includeFavoriteAccounts: false,
  includeOtherPublic: true,
  verificationDays: 14,
})

function controls(overrides: Partial<RecommendationControls>): RecommendationControls {
  return { ...defaults, ...overrides }
}

describe('recommendation ingredient commands', () => {
  it.each([
    ['preferred', 'required'],
    ['required', 'excluded'],
    ['excluded', 'preferred'],
  ] as const)('moves an ingredient atomically from %s to %s', (from, to) => {
    const ingredient = { name: 'User-entered Rice', canonicalIngredientId: 'rice-id' }
    const result = moveRecommendationIngredient(
      controls({ [from]: [ingredient] }),
      'c:rice-id',
      from,
      to,
    )

    expect(result.outcome).toBe('moved')
    expect(result.controls[from]).toEqual([])
    expect(result.controls[to]).toEqual([ingredient])
    expect(result.controls[to][0]).toBe(ingredient)
    expect(result.focusIdentity).toBe('c:rice-id')
  })

  it('removes the source without duplicating an existing target identity', () => {
    const target = { name: 'Rice already there', canonicalIngredientId: 'rice-id' }
    const result = moveRecommendationIngredient(
      controls({
        preferred: [{ name: 'Original Rice', canonicalIngredientId: 'rice-id' }],
        excluded: [target],
      }),
      'c:rice-id',
      'preferred',
      'excluded',
    )

    expect(result.outcome).toBe('already-present')
    expect(result.controls.preferred).toEqual([])
    expect(result.controls.excluded).toEqual([target])
  })

  it('uses normalized unresolved identity without merging same-text canonical identity', () => {
    const unresolved = { name: ' ＲＩＣＥ  flour ', canonicalIngredientId: null }
    const canonical = { name: 'rice flour', canonicalIngredientId: 'rice-flour-id' }
    const original = controls({ preferred: [unresolved, canonical] })

    const result = moveRecommendationIngredient(
      original,
      'n:rice flour',
      'preferred',
      'required',
    )

    expect(result.controls.preferred).toEqual([canonical])
    expect(result.controls.required).toEqual([unresolved])
  })

  it('requests an explicit move when an added identity is in another bucket', () => {
    const existing = { name: 'Original Rice', canonicalIngredientId: 'rice-id' }
    const result = addOrRequestMove(
      controls({ required: [existing] }),
      { name: 'Different display text', canonicalIngredientId: 'rice-id' },
      'excluded',
    )

    expect(result.outcome).toBe('move-required')
    expect(result.controls.required).toEqual([existing])
    expect(result.request).toEqual({
      from: 'required' satisfies RecommendationIngredientBucket,
      to: 'excluded' satisfies RecommendationIngredientBucket,
      identity: 'c:rice-id',
      ingredient: existing,
    })
  })

  it('leaves every bucket unchanged when a move target is full', () => {
    const original = controls({
      preferred: [{ name: 'Rice', canonicalIngredientId: 'rice-id' }],
      required: Array.from({ length: RECOMMENDATION_FILTER_LIMIT }, (_, index) => ({
        name: `Required ${index}`,
        canonicalIngredientId: `required-${index}`,
      })),
    })
    const result = moveRecommendationIngredient(original, 'c:rice-id', 'preferred', 'required')

    expect(result.outcome).toBe('target-full')
    expect(result.controls).toBe(original)
    expect(result.controls.preferred).toHaveLength(1)
    expect(result.controls.required).toHaveLength(RECOMMENDATION_FILTER_LIMIT)
  })

  it('leaves controls unchanged when adding to a full target', () => {
    const original = controls({
      excluded: Array.from({ length: RECOMMENDATION_FILTER_LIMIT }, (_, index) => ({
        name: `Excluded ${index}`,
        canonicalIngredientId: null,
      })),
    })
    const result = addOrRequestMove(
      original,
      { name: 'Rice', canonicalIngredientId: null },
      'excluded',
    )

    expect(result.outcome).toBe('target-full')
    expect(result.controls).toBe(original)
  })

  it('stages mixed additions and multiple cross-bucket moves as one transaction', () => {
    const original = controls({
      preferred: [{ name: 'Rice', canonicalIngredientId: null }],
      required: [{ name: 'Pepper', canonicalIngredientId: null }],
    })
    const result = prepareRecommendationIngredientBatch(
      original,
      [
        { name: 'ＲＩＣＥ', canonicalIngredientId: null },
        { name: 'Salt', canonicalIngredientId: null },
        { name: 'pepper', canonicalIngredientId: null },
      ],
      'excluded',
    )

    expect(result.outcome).toBe('ready')
    expect(result.confirmations).toHaveLength(2)
    expect(result.controls).not.toBe(original)
    expect(original.preferred).toHaveLength(1)
    expect(original.required).toHaveLength(1)
    expect(original.excluded).toEqual([])
    expect(result.controls.preferred).toEqual([])
    expect(result.controls.required).toEqual([])
    expect(result.controls.excluded.map((value) => value.name)).toEqual([
      'Pepper',
      'Rice',
      'Salt',
    ])
  })

  it('rejects an entire staged batch when its final target would exceed capacity', () => {
    const original = controls({
      preferred: [{ name: 'Rice', canonicalIngredientId: null }],
      excluded: Array.from({ length: RECOMMENDATION_FILTER_LIMIT - 1 }, (_, index) => ({
        name: `Excluded ${index}`,
        canonicalIngredientId: `excluded-${index}`,
      })),
    })
    const result = prepareRecommendationIngredientBatch(
      original,
      [
        { name: 'Salt', canonicalIngredientId: null },
        { name: 'Rice', canonicalIngredientId: null },
      ],
      'excluded',
    )

    expect(result.outcome).toBe('target-full')
    expect(result.controls).toBe(original)
    expect(original.preferred).toHaveLength(1)
    expect(original.excluded).toHaveLength(RECOMMENDATION_FILTER_LIMIT - 1)
  })
})
