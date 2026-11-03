import { afterEach, describe, expect, it, vi } from 'vitest'
import { createPasskey, getPasskey } from './webauthn'

const bytes = (...values: number[]) => new Uint8Array(values).buffer

afterEach(() => {
  Reflect.deleteProperty(navigator, 'credentials')
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('WebAuthn browser adapter', () => {
  it('decodes registration options and serializes the attestation response', async () => {
    vi.stubGlobal('PublicKeyCredential', class {})
    const create = vi.fn().mockResolvedValue({
      id: 'credential',
      rawId: bytes(1, 2),
      type: 'public-key',
      response: {
        attestationObject: bytes(3),
        clientDataJSON: bytes(4),
        getTransports: () => ['internal'],
      },
      getClientExtensionResults: () => ({ credProps: { rk: true } }),
    })
    Object.defineProperty(navigator, 'credentials', {
      configurable: true,
      value: { create, get: vi.fn() },
    })

    const result = await createPasskey({
      publicKey: {
        challenge: 'BQ',
        rp: { id: 'localhost', name: 'Cook' },
        user: { id: 'Bg', name: 'user@example.test', displayName: 'User' },
        pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
        excludeCredentials: [{ type: 'public-key', id: 'Bw' }],
      },
    })

    const options = create.mock.calls[0][0].publicKey
    expect(Array.from(options.challenge)).toEqual([5])
    expect(Array.from(options.user.id)).toEqual([6])
    expect(Array.from(options.excludeCredentials[0].id)).toEqual([7])
    expect(result).toEqual({
      id: 'credential',
      rawId: 'AQI',
      response: {
        attestationObject: 'Aw',
        clientDataJSON: 'BA',
        transports: ['internal'],
      },
      type: 'public-key',
      extensions: { credProps: { rk: true } },
    })
  })

  it('decodes authentication options and preserves a null user handle', async () => {
    vi.stubGlobal('PublicKeyCredential', class {})
    const get = vi.fn().mockResolvedValue({
      id: 'credential',
      rawId: bytes(8),
      type: 'public-key',
      response: {
        authenticatorData: bytes(9),
        clientDataJSON: bytes(10),
        signature: bytes(11),
        userHandle: null,
      },
      getClientExtensionResults: () => ({}),
    })
    Object.defineProperty(navigator, 'credentials', {
      configurable: true,
      value: { create: vi.fn(), get },
    })

    const result = await getPasskey({
      publicKey: {
        challenge: 'DA',
        allowCredentials: [{ type: 'public-key', id: 'DQ' }],
        userVerification: 'required',
      },
    })

    const options = get.mock.calls[0][0].publicKey
    expect(Array.from(options.challenge)).toEqual([12])
    expect(Array.from(options.allowCredentials[0].id)).toEqual([13])
    expect(result).toEqual({
      id: 'credential',
      rawId: 'CA',
      response: {
        authenticatorData: 'CQ',
        clientDataJSON: 'Cg',
        signature: 'Cw',
        userHandle: null,
      },
      type: 'public-key',
      extensions: {},
    })
  })
})
