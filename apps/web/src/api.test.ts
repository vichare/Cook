import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from './api'

describe('recipe cover API', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('sends local bytes and metadata as multipart without trusting a client content-type header', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 204,
    })
    vi.stubGlobal('fetch', fetchMock)
    const file = new File(['image-bytes'], 'untrusted-name.png', { type: 'image/png' })

    await api.uploadRecipeCover('recipe-id', file, {
      altText: 'A blue bowl of soup',
      sourceUrl: 'https://example.test/owned-image',
      sourceLicense: 'CC BY 4.0',
      sourceAttribution: 'Example Cook',
    })

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(init.headers).toBeUndefined()
    expect(init.body).toBeInstanceOf(FormData)
    const body = init.body as FormData
    expect(body.get('image')).toBe(file)
    expect(body.get('altText')).toBe('A blue bowl of soup')
    expect(body.get('sourceUrl')).toBe('https://example.test/owned-image')
  })
})
