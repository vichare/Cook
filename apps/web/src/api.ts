import type {
  PantryItem,
  Recipe,
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
  recipes: () => request<Recipe[]>('/api/recipes'),
  recipe: (id: string) => request<Recipe>(`/api/recipes/${id}`),
  saveRecipe: (value: Omit<Recipe, 'id' | 'ownerId' | 'isFavorite'>, id?: string) =>
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
}
