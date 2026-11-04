export type PantryStatus = 'have' | 'out' | 'unknown'

export interface User {
  id: string
  email: string
  displayName: string
  nickname: string | null
  onboardingCompleted: boolean
  locale: 'en' | 'zh-CN' | null
  capabilities: string[]
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

export interface IngredientAlternative extends Ingredient {
  id: string
  sortOrder?: number
}

export interface IngredientGroup {
  id: string
  optional: boolean
  note: string
  sortOrder?: number
  alternatives: IngredientAlternative[]
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
  publicationIntent: 'private' | 'public'
  moderationState: 'clear' | 'restricted'
  moderationReviewState: 'unreviewed' | 'reviewed'
  moderationVersion: number
  difficulty: number | null
  estimatedCalories: number | null
  ingredients: Ingredient[]
  ingredientModel?: 'groups-v1'
  ingredientGroups?: IngredientGroup[]
  isFavorite: boolean
  isFavoriteAuthor: boolean
  coverImage: RecipeImage | null
  source: RecipeSource | null
}

export interface OwnerNotification {
  id: string
  recipeId: string
  kind: 'recipe_restricted' | 'recipe_reinstated' | 'appeal_upheld'
  reasonCode: string
  createdAt: number
  readAt: number | null
}

export interface ModerationAppeal {
  id: string
  restrictionEpoch: number
  status: 'active' | 'reinstated' | 'upheld'
  reason: string
  resolutionReason: string | null
  version: number
  createdAt: number
  resolvedAt: number | null
}

export interface ModerationQueueItem {
  recipeId: string
  title: string
  authorHandle: string
  reviewState: 'unreviewed' | 'reviewed'
  moderationState: 'clear' | 'restricted'
  recipeVersion: number
  moderationVersion: number
  caseId: string | null
  caseVersion: number | null
  reportCount: number
  activeAppeal: boolean
  updatedAt: string
}

export interface ModerationQueuePage {
  items: ModerationQueueItem[]
  nextCursor: string | null
}

export interface ModerationCase {
  id: string
  recipeId: string
  status: string
  version: number
  createdAt: number
  updatedAt: number
  recipeVersion: number | null
  moderationVersion: number | null
  evidence: {
    recipeVersion: number
    title: string
    description: string
    instructions: string
    authorHandle: string
    imageId: string | null
  }
  reports: Array<{
    reason: string
    details: string
    recipeVersion: number
    createdAt: number
  }>
  appeal: {
    id: string
    reason: string
    version: number
    createdAt: number
  } | null
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

export interface ShoppingPurchaseOptions {
  resolution: 'reuse' | 'create' | 'choice' | 'selectAlternative'
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

export interface ShoppingChoiceAlternative extends IngredientReference {
  id: string
  purchaseOptions?: ShoppingPurchaseOptions
}

export interface ShoppingItem {
  id: string
  name: string
  canonicalIngredientId: string | null
  automatic: boolean
  pantryItemId: string | null
  sources: ShoppingItemSource[]
  version: number
  choiceAlternatives?: ShoppingChoiceAlternative[]
  purchaseOptions: ShoppingPurchaseOptions
}

export interface ShoppingDeleteResponse {
  outcome: 'removed' | 'automaticRemains' | 'automaticOnly'
  requiresDisableAutoRestock: boolean
  shoppingItems: ShoppingItem[]
  undoAvailable: boolean
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
  rankingIntent?: 'intent-v1' | {
    version: 'intent-v1'
    desiredMatched?: number
    desiredTotal?: number
  }
  ingredientReasons: Array<{
    ingredient: string
    code: 'AVAILABLE' | 'UNKNOWN' | 'STALE' | 'EXPIRES_TODAY' | 'EXPIRED' | 'OUT' | 'MISSING'
    expiresOn: string | null
  }>
  ingredientGroupReasons?: Array<{
    groupId: string
    label: string
    optional: boolean
    effective: boolean
    code: 'AVAILABLE' | 'UNKNOWN' | 'STALE' | 'EXPIRES_TODAY' | 'EXPIRED' | 'OUT' | 'MISSING'
    selectedAlternativeId: string | null
    alternatives: Array<{
      ingredient: string
      code: 'AVAILABLE' | 'UNKNOWN' | 'STALE' | 'EXPIRES_TODAY' | 'EXPIRED' | 'OUT' | 'MISSING'
      expiresOn: string | null
    }>
  }>
  appliedControls: {
    preferred: string[]
    required: string[]
    excluded: string[]
    ingredientScope?: IngredientScope
    pantryOnly?: boolean
    includeOwn: boolean
    includeFavoriteRecipes: boolean
    includeFavoriteAccounts: boolean
    includeOtherPublic: boolean
  }
}

export type IngredientScope = 'ANY' | 'PANTRY_ONLY' | 'PANTRY_OR_LIST'

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
  ingredientModel?: 'groups-v1'
  ingredientGroups?: IngredientGroup[]
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
  ingredientModel?: 'groups-v1'
  ingredientGroups?: IngredientGroup[]
  warnings: string[]
  confidence: number | null
  status: 'draft' | 'consumed' | 'discarded'
  acceptedRecipeId: string | null
  reviewUrl: string
  createdAt: string
  updatedAt: string
}
