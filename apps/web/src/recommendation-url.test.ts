import { describe, expect, it, vi } from 'vitest'
import {
  RECOMMENDATION_FILTER_LIMIT,
  canonicalizeRecommendationControls,
  encodeRecommendationControls,
  parseRecommendationControls,
  recommendationControlsFromSettings,
  recommendationControlsKey,
  tryDowngradeRecommendationCanonicalIds,
  type RecommendationControls,
} from './recommendation-url'

const defaults = recommendationControlsFromSettings({
  includeFavoriteRecipes: true,
  includeOwn: true,
  includeFavoriteAccounts: false,
  includeOtherPublic: true,
  verificationDays: 14,
})

describe('recommendation URL codec', () => {
  it('round trips canonical and unresolved identities with the same display text', () => {
    const controls: RecommendationControls = {
      ...defaults,
      preferred: [
        { name: 'Spring onion', canonicalIngredientId: null },
        { name: 'Spring onion', canonicalIngredientId: 'ingredient-42' },
      ],
    }

    const query = encodeRecommendationControls(controls)
    const parsed = parseRecommendationControls(query, defaults)

    expect(parsed.issues).toEqual([])
    expect(parsed.controls.preferred).toEqual([
      { name: 'Spring onion', canonicalIngredientId: 'ingredient-42' },
      { name: 'Spring onion', canonicalIngredientId: null },
    ])
    expect(query).toContain('rv=1')
    expect(query).not.toContain('ingredient-42')
  })

  it('sorts and deduplicates bucket sets so reordering does not change semantics', () => {
    const first: RecommendationControls = {
      ...defaults,
      preferred: [
        { name: 'Rice', canonicalIngredientId: null },
        { name: 'egg', canonicalIngredientId: null },
        { name: 'Ｒｉｃｅ', canonicalIngredientId: null },
      ],
    }
    const reordered: RecommendationControls = {
      ...defaults,
      preferred: [...first.preferred].reverse(),
    }

    expect(recommendationControlsKey(first)).toBe(recommendationControlsKey(reordered))
    expect(canonicalizeRecommendationControls(first).preferred).toEqual([
      { name: 'egg', canonicalIngredientId: null },
      { name: 'Rice', canonicalIngredientId: null },
    ])
    expect(recommendationControlsKey({
      ...defaults,
      preferred: [
        { name: 'Scallion', canonicalIngredientId: 'onion-id' },
        { name: 'Green onion', canonicalIngredientId: 'onion-id' },
      ],
    })).toBe(recommendationControlsKey({
      ...defaults,
      preferred: [
        { name: 'Green onion', canonicalIngredientId: 'onion-id' },
        { name: 'Scallion', canonicalIngredientId: 'onion-id' },
      ],
    }))
  })

  it('uses locale-independent Unicode code-unit ordering', () => {
    const localeCompare = vi.spyOn(String.prototype, 'localeCompare')
      .mockImplementation(() => {
        throw new Error('locale-sensitive comparison must not be used')
      })
    let query = ''
    try {
      query = encodeRecommendationControls({
        ...defaults,
        preferred: [
          { name: 'ä', canonicalIngredientId: null },
          { name: 'z', canonicalIngredientId: null },
        ],
      })
    } finally {
      localeCompare.mockRestore()
    }

    expect(parseRecommendationControls(query, defaults).controls.preferred).toEqual([
      { name: 'z', canonicalIngredientId: null },
      { name: 'ä', canonicalIngredientId: null },
    ])
  })

  it('keeps canonical identity distinct while rejecting cross-bucket conflicts', () => {
    const canonical = { name: 'Rice', canonicalIngredientId: 'rice-id' }
    const unresolved = { name: 'Rice', canonicalIngredientId: null }
    const query = encodeRecommendationControls({
      ...defaults,
      preferred: [canonical, unresolved],
    })
    const preferred = new URLSearchParams(query).getAll('p')
    const conflicting = `${query}&r=${encodeURIComponent(preferred[0])}`
    const parsed = parseRecommendationControls(conflicting, defaults)

    expect(parsed.controls.preferred).toEqual([canonical, unresolved])
    expect(parsed.controls.required).toEqual([])
    expect(parsed.issues).toContain('conflictingIngredient')
  })

  it('fails closed when canonical downgrade would merge bucket identities', () => {
    const canonicalUnresolvedConflict = tryDowngradeRecommendationCanonicalIds({
      ...defaults,
      required: [{ name: 'Rice', canonicalIngredientId: 'private-rice' }],
      excluded: [{ name: 'Rice', canonicalIngredientId: null }],
    }, new Set(['private-rice']))
    expect(canonicalUnresolvedConflict.controls).toBeNull()
    expect(canonicalUnresolvedConflict.conflictingIdentities).toEqual(['n:rice'])

    const multipleCanonicalConflict = tryDowngradeRecommendationCanonicalIds({
      ...defaults,
      preferred: [{ name: 'Rice', canonicalIngredientId: 'private-rice-a' }],
      required: [{ name: 'Rice', canonicalIngredientId: 'private-rice-b' }],
    }, new Set(['private-rice-a', 'private-rice-b']))
    expect(multipleCanonicalConflict.controls).toBeNull()
    expect(multipleCanonicalConflict.conflictingIdentities).toEqual(['n:rice'])
  })

  it('downgrades only unavailable IDs while preserving non-conflicting controls across buckets', () => {
    const result = tryDowngradeRecommendationCanonicalIds({
      ...defaults,
      preferred: [{ name: 'Rice', canonicalIngredientId: 'private-rice' }],
      required: [{ name: 'Beans', canonicalIngredientId: 'visible-beans' }],
      excluded: [{ name: 'Salt', canonicalIngredientId: null }],
    }, new Set(['private-rice']))

    expect(result.conflictingIdentities).toEqual([])
    expect(result.controls).toEqual({
      ...defaults,
      preferred: [{ name: 'Rice', canonicalIngredientId: null }],
      required: [{ name: 'Beans', canonicalIngredientId: 'visible-beans' }],
      excluded: [{ name: 'Salt', canonicalIngredientId: null }],
    })
  })

  it('fails safe for unknown versions, malformed values, duplicates, and oversized sets', () => {
    expect(parseRecommendationControls('rv=99&p=bad', defaults)).toEqual({
      controls: defaults,
      issues: ['unknownVersion'],
      hadUrlState: true,
    })

    const oversized = new URLSearchParams({ rv: '1', scope: 'invalid', src: 'own,own,secret' })
    for (let index = 0; index <= RECOMMENDATION_FILTER_LIMIT; index += 1) {
      oversized.append('p', 'u.UmljZQ')
    }
    const parsed = parseRecommendationControls(oversized.toString(), defaults)
    expect(parsed.controls.preferred).toEqual([{ name: 'Rice', canonicalIngredientId: null }])
    expect(parsed.controls.ingredientScope).toBe('ANY')
    expect(parsed.controls.sources).toEqual({
      includeFavoriteRecipes: false,
      includeOwn: true,
      includeFavoriteAccounts: false,
      includeOtherPublic: false,
    })
    expect(parsed.issues).toEqual(expect.arrayContaining([
      'duplicateParameter',
      'invalidParameter',
      'oversized',
    ]))
    expect(parseRecommendationControls(`rv=1&noise=${'x'.repeat(12_000)}`, defaults).issues)
      .toContain('oversized')
  })

  it('encodes only explicit filter controls and no private workspace state', () => {
    const query = decodeURIComponent(encodeRecommendationControls({
      ...defaults,
      preferred: [{ name: 'secret-filter-word', canonicalIngredientId: null }],
    }))

    expect(query).not.toMatch(/shopping|account|private|recipe|token|email/i)
    expect(query).not.toContain('secret-filter-word')
    expect(parseRecommendationControls(query, defaults).controls.preferred[0].name)
      .toBe('secret-filter-word')
  })
})
