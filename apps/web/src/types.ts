export type PantryStatus = 'have' | 'out' | 'unknown'

export interface User {
  id: string
  email: string
  displayName: string
  nickname: string | null
  onboardingCompleted: boolean
  locale: 'en' | 'zh-CN' | null
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
  googleEmail: string | null
  googleSignInStatus: 'linked' | 'eligible' | 'unsupported' | 'unavailable'
  passkeys: PasskeySummary[]
}

export interface Ingredient {
  name: string
  quantityNote: string
  canonicalIngredientId: string | null
}

export interface IngredientReference {
  name: string
  canonicalIngredientId: string | null
}

export interface IngredientSuggestion {
  canonicalIngredientId: string
  displayName: string
  matchedText: string
  reason: 'exact' | 'alias' | 'prefix' | 'substring' | 'similar'
}

export interface Recipe {
  id: string
  version: number
  isOwner: boolean
  authorNickname: string | null
  title: string
  description: string
  instructions: string
  visibility: 'private' | 'public'
  ingredients: Ingredient[]
  isFavorite: boolean
  coverImage: RecipeImage | null
  source: RecipeSource | null
}

export interface RecipeImage {
  id: string
  url: string
  width: number
  height: number
  altText: string
  sourceUrl: string | null
  sourceLicense: string | null
  sourceAttribution: string | null
  version: number
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
  expiresOn: string | null
  canonicalIngredientId: string | null
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
  reasonDetails: RecommendationReason[]
  score: number
}

export type RecommendationReason =
  | { code: 'confirmedIngredients'; confirmed: number; total: number }
  | { code: 'preferredIngredients'; count: number }
  | { code: 'ingredientExpired'; ingredient: string; expiresOn: string }
  | { code: 'ingredientExpiresToday'; ingredient: string }
  | { code: 'pantryOnlyBlocked' }

export interface ImportToken {
  id: string
  name: string
  prefix: string
  scopes: Array<
    | 'recipes:import'
    | 'recipes:read'
    | 'recipes:write'
    | 'recipes:images:write'
    | 'recipes:publish'
  >
  createdAt: string
  lastUsedAt: string | null
  expiresAt: string | null
  revokedAt: string | null
}

export interface RecipeRevision {
  id: string
  recipeId: string
  expectedVersion: number
  title: string
  description: string
  instructions: string
  ingredients: Ingredient[]
  imageAction: 'keep' | 'replace' | 'delete'
  imageAltText: string | null
  imageUrl: string | null
  imageSourceUrl: string | null
  imageSourceLicense: string | null
  imageSourceAttribution: string | null
  status: 'pending' | 'accepted' | 'rejected'
  createdAt: string
  updatedAt: string
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
