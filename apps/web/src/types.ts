export type PantryStatus = 'have' | 'out' | 'unknown'

export interface User {
  id: string
  email: string
  displayName: string
  nickname: string | null
  onboardingCompleted: boolean
  locale: 'en' | 'zh-CN' | null
}

export interface AiCapabilities {
  prefix: string
  scopes: string[]
  expiresAt: string | null
  preset: 'import' | 'manage' | 'publish' | null
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
  authorHandle: string
  authorNickname: string | null
  title: string
  description: string
  instructions: string
  visibility: 'private' | 'public'
  difficulty: number | null
  estimatedCalories: number | null
  ingredients: Ingredient[]
  isFavorite: boolean
  isFavoriteAuthor: boolean
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
  url: string | null
  title: string | null
  author: string | null
  license: string | null
  attribution: string | null
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
  aliases?: string[]
  autoAddToShoppingListWhenOut: boolean
  version: number
}

export interface ShoppingItemSource {
  kind: 'automatic' | 'manual' | 'recipe'
  label: string
  recipeId: string | null
}

export interface ShoppingItem {
  id: string
  name: string
  canonicalIngredientId: string | null
  automatic: boolean
  pantryItemId: string | null
  sources: ShoppingItemSource[]
  version: number
  purchaseOptions: {
    resolution: 'reuse' | 'create' | 'choice'
    candidates: Array<{
      id: string
      name: string
      status: 'have' | 'out' | 'unknown'
      quantityNote: string
      expiresOn: string | null
      canonicalIngredientId: string | null
      autoAddToShoppingListWhenOut: boolean
      version: number
    }>
  }
}

export interface ShoppingPurchaseResponse {
  pantry: PantryItem
  shoppingItems: ShoppingItem[]
  purchaseActionId: string
  undoAvailable: boolean
}

export interface RecommendationSettings {
  includeFavoriteRecipes: boolean
  includeOwn: boolean
  includeFavoriteAccounts: boolean
  includeOtherPublic: boolean
  verificationDays: number
}

export interface Recommendation {
  recipe: Recipe
  status: 'READY' | 'VERIFY' | 'SHOP' | 'BLOCKED'
  available: string[]
  verify: string[]
  missing: string[]
  inShoppingList: string[]
  reasons: string[]
  reasonDetails: RecommendationReason[]
  score: number
  scoreBreakdown: {
    statusBase: number
    source: number
    availability: number
    desired: number
    certainty: number
    simplicity: number
  }
  primarySource: RecommendationSource
  matchedSources: RecommendationSource[]
  desiredMatches: string[]
  ingredientReasons: Array<{
    ingredient: string
    code: 'AVAILABLE' | 'UNKNOWN' | 'STALE' | 'EXPIRES_TODAY' | 'EXPIRED' | 'OUT' | 'MISSING'
    expiresOn: string | null
  }>
  appliedControls: {
    preferred: string[]
    required: string[]
    excluded: string[]
    pantryOnly: boolean
    includeOwn: boolean
    includeFavoriteRecipes: boolean
    includeFavoriteAccounts: boolean
    includeOtherPublic: boolean
  }
}

export type RecommendationSource = 'OWN' | 'FAVORITE_RECIPE' | 'FAVORITE_ACCOUNT' | 'OTHER_PUBLIC'

export interface PublicAuthorProfile {
  handle: string
  nickname: string | null
  recipes: Recipe[]
  nextCursor: string | null
}

export interface FavoriteAuthor {
  handle: string
  nickname: string | null
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
  difficulty: number | null
  estimatedCalories: number | null
  sourceUrl: string | null
  sourceTitle: string | null
  sourceAuthor: string | null
  sourceLicense: string | null
  sourceAttribution: string | null
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
  sourceLicense: string | null
  sourceAttribution: string | null
  title: string
  description: string
  instructions: string
  difficulty: number | null
  estimatedCalories: number | null
  ingredients: Ingredient[]
  warnings: string[]
  confidence: number | null
  status: 'draft' | 'consumed' | 'discarded'
  acceptedRecipeId: string | null
  reviewUrl: string
  createdAt: string
  updatedAt: string
}
