type JsonRecord = Record<string, unknown>

function decodeBase64Url(value: string): Uint8Array {
  const padded = value.replace(/-/g, '+').replace(/_/g, '/').padEnd(Math.ceil(value.length / 4) * 4, '=')
  const binary = atob(padded)
  return Uint8Array.from(binary, (character) => character.charCodeAt(0))
}

function encodeBase64Url(value: ArrayBuffer | null): string | null {
  if (value === null) return null
  const bytes = new Uint8Array(value)
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '')
}

function publicKeyOptions(value: unknown): JsonRecord {
  if (!value || typeof value !== 'object') throw new Error('Invalid passkey challenge')
  const publicKey = (value as JsonRecord).publicKey
  if (!publicKey || typeof publicKey !== 'object') throw new Error('Invalid passkey challenge')
  return publicKey as JsonRecord
}

export async function createPasskey(challenge: unknown): Promise<unknown> {
  if (!window.PublicKeyCredential || !navigator.credentials) {
    throw new Error('Passkeys are not supported by this browser')
  }
  const json = publicKeyOptions(challenge)
  const user = json.user as JsonRecord
  const excludeCredentials = (json.excludeCredentials as JsonRecord[] | undefined)?.map((credential) => ({
    ...credential,
    id: decodeBase64Url(String(credential.id)),
  }))
  const credential = (await navigator.credentials.create({
    publicKey: {
      ...(json as unknown as PublicKeyCredentialCreationOptions),
      challenge: decodeBase64Url(String(json.challenge)),
      user: {
        ...(user as unknown as PublicKeyCredentialUserEntity),
        id: decodeBase64Url(String(user.id)),
      },
      excludeCredentials: excludeCredentials as PublicKeyCredentialDescriptor[] | undefined,
    },
  })) as PublicKeyCredential | null
  if (!credential) throw new Error('Passkey registration was cancelled')
  const response = credential.response as AuthenticatorAttestationResponse
  return {
    id: credential.id,
    rawId: encodeBase64Url(credential.rawId),
    response: {
      attestationObject: encodeBase64Url(response.attestationObject),
      clientDataJSON: encodeBase64Url(response.clientDataJSON),
      transports: typeof response.getTransports === 'function' ? response.getTransports() : [],
    },
    type: credential.type,
    extensions: credential.getClientExtensionResults(),
  }
}

export async function getPasskey(challenge: unknown): Promise<unknown> {
  if (!window.PublicKeyCredential || !navigator.credentials) {
    throw new Error('Passkeys are not supported by this browser')
  }
  const json = publicKeyOptions(challenge)
  const allowCredentials = (json.allowCredentials as JsonRecord[] | undefined)?.map((credential) => ({
    ...credential,
    id: decodeBase64Url(String(credential.id)),
  }))
  const credential = (await navigator.credentials.get({
    publicKey: {
      ...(json as unknown as PublicKeyCredentialRequestOptions),
      challenge: decodeBase64Url(String(json.challenge)),
      allowCredentials: allowCredentials as PublicKeyCredentialDescriptor[] | undefined,
    },
  })) as PublicKeyCredential | null
  if (!credential) throw new Error('Passkey sign-in was cancelled')
  const response = credential.response as AuthenticatorAssertionResponse
  return {
    id: credential.id,
    rawId: encodeBase64Url(credential.rawId),
    response: {
      authenticatorData: encodeBase64Url(response.authenticatorData),
      clientDataJSON: encodeBase64Url(response.clientDataJSON),
      signature: encodeBase64Url(response.signature),
      userHandle: encodeBase64Url(response.userHandle),
    },
    type: credential.type,
    extensions: credential.getClientExtensionResults(),
  }
}
