import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from './api'

describe('recipe cover API', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('sends local bytes and metadata as multipart without trusting a client content-type header', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: () => Promise.resolve({ version: 2 }),
    })
    vi.stubGlobal('fetch', fetchMock)
    const file = new File(['image-bytes'], 'untrusted-name.png', { type: 'image/png' })

    await api.uploadRecipeCover('recipe-id', file, {
      altText: 'A blue bowl of soup',
      sourceUrl: 'https://example.test/owned-image',
      sourceLicense: 'CC BY 4.0',
      sourceAttribution: 'Example Cook',
    }, 1, 'cover-operation-1')

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(init.headers).toEqual({
      'if-match': '"recipe-v1"',
      'idempotency-key': 'cover-operation-1',
    })
    expect(init.body).toBeInstanceOf(FormData)
    const body = init.body as FormData
    expect(body.get('image')).toBe(file)
    expect(body.get('altText')).toBe('A blue bowl of soup')
    expect(body.get('sourceUrl')).toBe('https://example.test/owned-image')
  })

  it('sends shopping deletion version in JSON with an idempotency key', async () => {
    const response = {
      outcome: 'removed',
      requiresDisableAutoRestock: false,
      shoppingItems: [],
      undoAvailable: false,
    }
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: () => Promise.resolve(response),
    })
    vi.stubGlobal('fetch', fetchMock)

    await expect(api.deleteShoppingItem('shopping-1', 7, 'delete-operation-1'))
      .resolves.toEqual(response)

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('/api/shopping-list/shopping-1')
    expect(init.method).toBe('DELETE')
    expect(init.headers).toEqual({
      'content-type': 'application/json',
      'Idempotency-Key': 'delete-operation-1',
    })
    expect(JSON.parse(String(init.body))).toEqual({ expectedVersion: 7 })
  })

  it('passes recommendation cancellation and request-scoped source controls to fetch', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: () => Promise.resolve([]),
    })
    vi.stubGlobal('fetch', fetchMock)
    const controller = new AbortController()

    await api.recommendations({
      preferred: [],
      required: [],
      excluded: [],
      ingredientScope: 'ANY',
      includeFavoriteRecipes: true,
      includeOwn: false,
      includeFavoriteAccounts: true,
      includeOtherPublic: false,
      today: '2026-11-05',
    }, controller.signal)

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(init.signal).toBe(controller.signal)
    expect(JSON.parse(String(init.body))).toEqual(expect.objectContaining({
      includeOwn: false,
      includeOtherPublic: false,
    }))
  })

  it('preserves unavailable canonical IDs from a recommendation validation error', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: false,
      status: 400,
      json: () => Promise.resolve({
        error: 'canonical_ingredient_unavailable',
        message: 'selected canonical ingredient is not available to this account',
        canonicalIngredientIds: ['private-id'],
      }),
    }))

    await expect(api.recommendations({
      preferred: [{ name: 'Private spice', canonicalIngredientId: 'private-id' }],
      required: [],
      excluded: [],
      ingredientScope: 'ANY',
      includeFavoriteRecipes: true,
      includeOwn: true,
      includeFavoriteAccounts: true,
      includeOtherPublic: true,
      today: '2026-11-05',
    })).rejects.toEqual(expect.objectContaining({
      code: 'canonical_ingredient_unavailable',
      canonicalIngredientIds: ['private-id'],
    }))
  })
})
