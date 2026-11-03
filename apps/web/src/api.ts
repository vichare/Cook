import type {
  PantryItem,
  Recipe,
  CreatedImportToken,
  ImportDraft,
  ImportToken,
  Recommendation,
  RecommendationSettings,
  User,
} from './types'

export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
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
    const body = (await response.json().catch(() => null)) as { message?: string } | null
    throw new ApiError(body?.message ?? `Request failed (${response.status})`, response.status)
  }
  if (response.status === 204) return undefined as T
  return response.json() as Promise<T>
}

export const api = {
  authConfig: () => request<{ mode: 'development' | 'google' }>('/api/auth/config'),
  me: () => request<User>('/api/me'),
  devLogin: (displayName: string) =>
    request<User>('/api/auth/dev-login', {
      method: 'POST',
      body: JSON.stringify({ displayName }),
    }),
  logout: () => request<void>('/api/auth/logout', { method: 'POST' }),
  saveProfile: (nickname: string | null) =>
    request<User>('/api/settings/profile', {
      method: 'PUT',
      body: JSON.stringify({ nickname }),
    }),
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
  savePantry: (items: Array<Pick<PantryItem, 'name' | 'status' | 'quantityNote'>>) =>
    request<PantryItem[]>('/api/pantry', { method: 'PUT', body: JSON.stringify(items) }),
  settings: () => request<RecommendationSettings>('/api/settings/recommendations'),
  saveSettings: (value: RecommendationSettings) =>
    request<RecommendationSettings>('/api/settings/recommendations', {
      method: 'PUT',
      body: JSON.stringify(value),
    }),
  recommendations: (value: {
    preferred: string[]
    required: string[]
    excluded: string[]
    pantryOnly: boolean
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
