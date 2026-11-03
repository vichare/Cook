export type PantryStatus = 'have' | 'out' | 'unknown'

export interface User {
  id: string
  email: string
  displayName: string
}

export interface Ingredient {
  name: string
  quantityNote: string
}

export interface Recipe {
  id: string
  ownerId: string
  title: string
  description: string
  instructions: string
  visibility: 'private' | 'public'
  ingredients: Ingredient[]
  isFavorite: boolean
}

export interface PantryItem {
  id: string
  name: string
  status: PantryStatus
  quantityNote: string
  confirmedAt: string | null
}

export interface RecommendationSettings {
  includeFavorites: boolean
  includeOwn: boolean
  includePublic: boolean
  verificationDays: number
}

export interface Recommendation {
  recipe: Recipe
  status: 'READY' | 'VERIFY' | 'SHOP' | 'BLOCKED'
  available: string[]
  verify: string[]
  missing: string[]
  reasons: string[]
  score: number
}
