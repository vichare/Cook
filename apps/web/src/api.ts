import type {
  PantryItem,
  Recipe,
  CreatedImportToken,
  ImportDraft,
  ImportToken,
  Recommendation,
  RecommendationSettings,
  AuthConfig,
  EmailOtpChallenge,
  IngredientSuggestion,
  PasskeySummary,
  SecuritySettings,
  User,
} from './types'

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
    public readonly code: string,
  ) {
    super(message)
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: 'same-origin',
    ...init,
    headers: {
      ...(init?.body ? { 'content-type': 'application/json' } : {}),
      ...init?.headers,
    },
  })
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string; message?: string } | null
    throw new ApiError(
      body?.message ?? `Request failed (${response.status})`,
      response.status,
      body?.error ?? 'unknown_error',
    )
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

export const api = {
  authConfig: () => request<AuthConfig>('/api/auth/config'),
  me: () => request<User>('/api/me'),
  devLogin: (displayName: string) =>
    request<User>('/api/auth/dev-login', {
      method: 'POST',
      body: JSON.stringify({ displayName }),
    }),
  logout: () => request<void>('/api/auth/logout', { method: 'POST' }),
  requestEmailOtp: (email: string) =>
    request<EmailOtpChallenge>('/api/auth/email/request', {
      method: 'POST',
      body: JSON.stringify({ email }),
    }),
  verifyEmailOtp: (challengeId: string, code: string) =>
    request<User>('/api/auth/email/verify', {
      method: 'POST',
      body: JSON.stringify({ challengeId, code }),
    }),
  startPasskeyLogin: (email: string) =>
    request<unknown>('/api/auth/passkey/start', {
      method: 'POST',
      body: JSON.stringify({ email }),
    }),
  finishPasskeyLogin: (credential: unknown) =>
    request<User>('/api/auth/passkey/finish', {
      method: 'POST',
      body: JSON.stringify(credential),
    }),
  saveProfile: (nickname: string | null) =>
    request<User>('/api/settings/profile', {
      method: 'PUT',
      body: JSON.stringify({ nickname }),
    }),
  saveLocale: (locale: 'en' | 'zh-CN') =>
    request<User>('/api/settings/locale', {
      method: 'PUT',
      body: JSON.stringify({ locale }),
    }),
  securitySettings: () => request<SecuritySettings>('/api/settings/security'),
  requestEmailLink: (email: string) =>
    request<EmailOtpChallenge>('/api/settings/security/email/request', {
      method: 'POST',
      body: JSON.stringify({ email }),
    }),
  verifyEmailLink: (challengeId: string, code: string) =>
    request<SecuritySettings>('/api/settings/security/email/verify', {
      method: 'POST',
      body: JSON.stringify({ challengeId, code }),
    }),
  startPasskeyRegistration: (name: string) =>
    request<unknown>('/api/settings/security/passkeys/register/start', {
      method: 'POST',
      body: JSON.stringify({ name }),
    }),
  finishPasskeyRegistration: (credential: unknown) =>
    request<PasskeySummary>('/api/settings/security/passkeys/register/finish', {
      method: 'POST',
      body: JSON.stringify(credential),
    }),
  revokePasskey: (id: string) =>
    request<void>(`/api/settings/security/passkeys/${id}`, { method: 'DELETE' }),
  recipes: () => request<Recipe[]>('/api/recipes'),
  recipe: (id: string) => request<Recipe>(`/api/recipes/${id}`),
  saveRecipe: (
    value: Pick<Recipe, 'title' | 'description' | 'instructions' | 'visibility' | 'ingredients'>,
    id?: string,
  ) =>
    request<Recipe>(id ? `/api/recipes/${id}` : '/api/recipes', {
      method: id ? 'PUT' : 'POST',
      body: JSON.stringify(value),
    }),
  favorite: (id: string, favorite: boolean) =>
    request<void>(`/api/recipes/${id}/favorite`, { method: favorite ? 'PUT' : 'DELETE' }),
  pantry: () => request<PantryItem[]>('/api/pantry'),
  savePantry: (items: Array<Pick<PantryItem, 'name' | 'status' | 'quantityNote'> & {
    expiresOn?: string | null
    canonicalIngredientId?: string | null
  }>) =>
    request<PantryItem[]>('/api/pantry', { method: 'PUT', body: JSON.stringify(items) }),
  ingredientSuggestions: (query: string) =>
    request<IngredientSuggestion[]>(`/api/ingredients/suggestions?q=${encodeURIComponent(query)}`),
  settings: () => request<RecommendationSettings>('/api/settings/recommendations'),
  saveSettings: (value: RecommendationSettings) =>
    request<RecommendationSettings>('/api/settings/recommendations', {
      method: 'PUT',
      body: JSON.stringify(value),
    }),
  recommendations: (value: {
    preferred: Array<{ name: string; canonicalIngredientId: string | null }>
    required: Array<{ name: string; canonicalIngredientId: string | null }>
    excluded: Array<{ name: string; canonicalIngredientId: string | null }>
    pantryOnly: boolean
    today: string
  }) =>
    request<Recommendation[]>('/api/recommendations', {
      method: 'POST',
      body: JSON.stringify(value),
    }),
  importTokens: () => request<ImportToken[]>('/api/settings/import-tokens'),
  createImportToken: (name: string, expiresInDays: number | null) =>
    request<CreatedImportToken>('/api/settings/import-tokens', {
      method: 'POST',
      body: JSON.stringify({ name, expiresInDays }),
    }),
  revokeImportToken: (id: string) =>
    request<void>(`/api/settings/import-tokens/${id}`, { method: 'DELETE' }),
  importDrafts: () => request<ImportDraft[]>('/api/import-drafts'),
  importDraft: (id: string) => request<ImportDraft>(`/api/import-drafts/${id}`),
  saveImportDraft: (
    id: string,
    value: Pick<ImportDraft, 'title' | 'description' | 'instructions' | 'ingredients'>,
  ) =>
    request<ImportDraft>(`/api/import-drafts/${id}`, {
      method: 'PUT',
      body: JSON.stringify(value),
    }),
  acceptImportDraft: (id: string) =>
    request<Recipe>(`/api/import-drafts/${id}/accept`, { method: 'POST' }),
  discardImportDraft: (id: string) =>
    request<void>(`/api/import-drafts/${id}`, { method: 'DELETE' }),
}
