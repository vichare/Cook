import type {
  PantryItem,
  Recipe,
  CreatedImportToken,
  ImportDraft,
  ImportToken,
  Recommendation,
  RecommendationSettings,
  RecipeRevision,
  AuthConfig,
  EmailOtpChallenge,
  IngredientSuggestion,
  FavoriteAuthor,
  PasskeySummary,
  SecuritySettings,
  ShoppingItem,
  PublicAuthorProfile,
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
  authorProfile: (handle: string, cursor?: string) =>
    request<PublicAuthorProfile>(
      `/api/authors/${encodeURIComponent(handle)}${cursor ? `?cursor=${encodeURIComponent(cursor)}` : ''}`,
    ),
  favoriteAccounts: () => request<FavoriteAuthor[]>('/api/favorite-accounts'),
  favoriteAccount: (handle: string, favorite: boolean) =>
    request<void>(`/api/authors/${encodeURIComponent(handle)}/favorite`, {
      method: favorite ? 'PUT' : 'DELETE',
    }),
  saveRecipe: (
    value: Pick<
      Recipe,
      | 'title'
      | 'description'
      | 'instructions'
      | 'visibility'
      | 'difficulty'
      | 'estimatedCalories'
      | 'ingredients'
    >,
    id?: string,
    expectedVersion?: number,
  ) =>
    request<Recipe>(id ? `/api/recipes/${id}` : '/api/recipes', {
      method: id ? 'PUT' : 'POST',
      body: JSON.stringify(value),
      headers: id && expectedVersion
        ? { 'if-match': `"recipe-v${expectedVersion}"` }
        : undefined,
    }),
  uploadRecipeCover: async (
    id: string,
    image: File,
    metadata: {
      altText: string
      sourceUrl: string
      sourceLicense: string
      sourceAttribution: string
    },
    expectedVersion: number,
  ) => {
    const body = new FormData()
    body.set('image', image)
    body.set('altText', metadata.altText)
    if (metadata.sourceUrl) body.set('sourceUrl', metadata.sourceUrl)
    if (metadata.sourceLicense) body.set('sourceLicense', metadata.sourceLicense)
    if (metadata.sourceAttribution) body.set('sourceAttribution', metadata.sourceAttribution)
    const response = await fetch(`/api/recipes/${id}/cover`, {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'if-match': `"recipe-v${expectedVersion}"` },
      body,
    })
    if (!response.ok) {
      const error = (await response.json().catch(() => null)) as { error?: string; message?: string } | null
      throw new ApiError(
        error?.message ?? `Request failed (${response.status})`,
        response.status,
        error?.error ?? 'unknown_error',
      )
    }
    return response.json() as Promise<{ version: number }>
  },
  deleteRecipeCover: (id: string, expectedVersion: number) =>
    request<{ version: number }>(`/api/recipes/${id}/cover`, {
      method: 'DELETE',
      headers: { 'if-match': `"recipe-v${expectedVersion}"` },
    }),
  updateRecipeCoverMetadata: (
    id: string,
    metadata: {
      altText: string
      sourceUrl: string
      sourceLicense: string
      sourceAttribution: string
    },
    expectedVersion: number,
  ) =>
    request<{ version: number }>(`/api/recipes/${id}/cover`, {
      method: 'PUT',
      body: JSON.stringify(metadata),
      headers: { 'if-match': `"recipe-v${expectedVersion}"` },
    }),
  recipeRevisions: (recipeId: string) =>
    request<RecipeRevision[]>(`/api/recipe-revisions?recipeId=${encodeURIComponent(recipeId)}`),
  acceptRecipeRevision: (id: string) =>
    request<Recipe>(`/api/recipe-revisions/${id}/accept`, { method: 'POST' }),
  rejectRecipeRevision: (id: string) =>
    request<void>(`/api/recipe-revisions/${id}`, { method: 'DELETE' }),
  favorite: (id: string, favorite: boolean) =>
    request<void>(`/api/recipes/${id}/favorite`, { method: favorite ? 'PUT' : 'DELETE' }),
  pantry: () => request<PantryItem[]>('/api/pantry'),
  savePantry: (items: Array<Pick<PantryItem, 'name' | 'status' | 'quantityNote'> & {
    expiresOn?: string | null
    canonicalIngredientId?: string | null
    autoAddToShoppingListWhenOut?: boolean
  }>) =>
    request<PantryItem[]>('/api/pantry', { method: 'PUT', body: JSON.stringify(items) }),
  shoppingItems: () => request<ShoppingItem[]>('/api/shopping-list'),
  addShoppingItem: (value: { name: string; canonicalIngredientId: string | null }) =>
    request<ShoppingItem[]>('/api/shopping-list', {
      method: 'POST',
      body: JSON.stringify(value),
    }),
  updateShoppingItem: (
    id: string,
    value: { name: string; canonicalIngredientId: string | null },
  ) =>
    request<ShoppingItem[]>(`/api/shopping-list/${id}`, {
      method: 'PUT',
      body: JSON.stringify(value),
    }),
  completeShoppingItem: (id: string) =>
    request<ShoppingItem[]>(`/api/shopping-list/${id}/complete`, { method: 'POST' }),
  deleteShoppingItem: (id: string) =>
    request<ShoppingItem[]>(`/api/shopping-list/${id}`, { method: 'DELETE' }),
  markAutomaticShoppingPurchased: (pantryItemId: string) =>
    request<ShoppingItem[]>(`/api/shopping-list/automatic/${pantryItemId}/purchased`, {
      method: 'POST',
    }),
  addRecipeIngredientToShopping: (
    recipeId: string,
    value: { name: string; canonicalIngredientId: string | null },
  ) =>
    request<ShoppingItem[]>(`/api/recipes/${recipeId}/shopping-list-items`, {
      method: 'POST',
      body: JSON.stringify(value),
    }),
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
  createImportToken: (
    name: string,
    expiresInDays: number | null,
    preset: 'import' | 'manage' | 'publish',
  ) =>
    request<CreatedImportToken>('/api/settings/import-tokens', {
      method: 'POST',
      body: JSON.stringify({
        name,
        expiresInDays,
        preset,
        confirmPublishRisk: preset === 'publish',
      }),
    }),
  revokeImportToken: (id: string) =>
    request<void>(`/api/settings/import-tokens/${id}`, { method: 'DELETE' }),
  importDrafts: () => request<ImportDraft[]>('/api/import-drafts'),
  importDraft: (id: string) => request<ImportDraft>(`/api/import-drafts/${id}`),
  saveImportDraft: (
    id: string,
    value: Pick<
      ImportDraft,
      'title' | 'description' | 'instructions' | 'difficulty' | 'estimatedCalories' | 'ingredients'
    >,
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
