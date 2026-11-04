import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  useRecommendationWorkbench,
  type RecommendationControlsCommitResult,
} from './recommendation-workbench'
import { recommendationControlsFromSettings } from './recommendation-url'

const { recommendations } = vi.hoisted(() => ({
  recommendations: vi.fn().mockResolvedValue([]),
}))

vi.mock('./api', async () => {
  const actual = await vi.importActual<typeof import('./api')>('./api')
  return {
    ...actual,
    api: { ...actual.api, recommendations },
  }
})

const settings = {
  includeFavoriteRecipes: true,
  includeOwn: true,
  includeFavoriteAccounts: true,
  includeOtherPublic: true,
  verificationDays: 14,
}

afterEach(() => {
  cleanup()
  recommendations.mockClear()
  window.history.replaceState({}, '', '/')
})

describe('recommendation workbench control commits', () => {
  it('returns rejected and preserves controls and URL for an oversized commit', async () => {
    const { result } = renderHook(() => useRecommendationWorkbench(settings))
    await waitFor(() => expect(result.current.controls).not.toBeNull())
    const originalControls = result.current.controls
    const originalUrl = window.location.href
    const oversized = {
      ...recommendationControlsFromSettings(settings),
      preferred: Array.from({ length: 100 }, (_, index) => ({
        name: `${index}-${'x'.repeat(110)}`,
        canonicalIngredientId: null,
      })),
    }
    let commit: RecommendationControlsCommitResult | undefined

    act(() => {
      commit = result.current.setControls(oversized)
    })

    expect(commit).toEqual(expect.objectContaining({
      reason: 'oversized',
      status: 'rejected',
    }))
    expect(result.current.controls).toBe(originalControls)
    expect(window.location.href).toBe(originalUrl)
    expect(result.current.urlIssues).toContain('oversized')
  })
})
