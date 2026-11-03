export type PantryStatus = 'have' | 'out' | 'unknown'

export interface User {
  id: string
  email: string
  displayName: string
  nickname: string | null
  onboardingCompleted: boolean
}

export interface AuthConfig {
  mode: 'development' | 'production'
  google: boolean
  emailOtp: boolean
  passkey: boolean
}

export interface EmailOtpChallenge {
  challengeId: string
  expiresAt: string
  resendAllowedAt: string
}

export interface PasskeySummary {
  id: string
  name: string
  createdAt: string
  lastUsedAt: string | null
}

export interface SecuritySettings {
  linkedEmails: string[]
  passkeys: PasskeySummary[]
}

export interface Ingredient {
  name: string
  quantityNote: string
}

export interface Recipe {
  id: string
  isOwner: boolean
  authorNickname: string | null
  title: string
  description: string
  instructions: string
  visibility: 'private' | 'public'
  ingredients: Ingredient[]
  isFavorite: boolean
  source: RecipeSource | null
}

export interface RecipeSource {
  url: string
  title: string | null
  author: string | null
  warnings: string[]
  confidence: number | null
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

export interface ImportToken {
  id: string
  name: string
  prefix: string
  scope: 'recipes:import'
  createdAt: string
  lastUsedAt: string | null
  expiresAt: string | null
  revokedAt: string | null
}

export interface CreatedImportToken {
  token: ImportToken
  secret: string
}

export interface ImportDraft {
  id: string
  sourceUrl: string
  sourceTitle: string | null
  sourceAuthor: string | null
  title: string
  description: string
  instructions: string
  ingredients: Ingredient[]
  warnings: string[]
  confidence: number | null
  status: 'draft' | 'consumed' | 'discarded'
  acceptedRecipeId: string | null
  reviewUrl: string
  createdAt: string
  updatedAt: string
}
