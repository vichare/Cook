import { ingredientIdentity, normalizeIngredient } from './ingredients'
import type {
  IngredientReference,
  IngredientScope,
  RecommendationSettings,
} from './types'

export const RECOMMENDATION_URL_VERSION = '1'
export const RECOMMENDATION_FILTER_LIMIT = 100
export const RECOMMENDATION_URL_MAX_LENGTH = 12_000

const MAX_INGREDIENT_NAME_LENGTH = 120
const MAX_CANONICAL_ID_LENGTH = 200

export interface RecommendationSourceControls {
  includeFavoriteRecipes: boolean
  includeOwn: boolean
  includeFavoriteAccounts: boolean
  includeOtherPublic: boolean
}

export interface RecommendationControls {
  preferred: IngredientReference[]
  required: IngredientReference[]
  excluded: IngredientReference[]
  ingredientScope: IngredientScope
  sources: RecommendationSourceControls
}

export type RecommendationUrlIssue =
  | 'canonicalDowngraded'
  | 'canonicalDowngradeConflict'
  | 'conflictingIngredient'
  | 'duplicateParameter'
  | 'invalidIngredient'
  | 'invalidParameter'
  | 'oversized'
  | 'unknownVersion'

export interface RecommendationUrlParseResult {
  controls: RecommendationControls
  issues: RecommendationUrlIssue[]
  hadUrlState: boolean
}

export type RecommendationControlsValidationResult =
  | {
    controls: RecommendationControls
    query: string
    status: 'valid'
  }
  | {
    controls: RecommendationControls
    status: 'oversized'
  }

const bucketParameters = [
  ['preferred', 'p'],
  ['required', 'r'],
  ['excluded', 'x'],
] as const

const sourceTokens = [
  ['includeOwn', 'own'],
  ['includeFavoriteRecipes', 'saved'],
  ['includeFavoriteAccounts', 'accounts'],
  ['includeOtherPublic', 'public'],
] as const

export function recommendationControlsFromSettings(
  settings: RecommendationSettings,
): RecommendationControls {
  return {
    preferred: [],
    required: [],
    excluded: [],
    ingredientScope: 'ANY',
    sources: {
      includeFavoriteRecipes: settings.includeFavoriteRecipes,
      includeOwn: settings.includeOwn,
      includeFavoriteAccounts: settings.includeFavoriteAccounts,
      includeOtherPublic: settings.includeOtherPublic,
    },
  }
}

export function canonicalizeRecommendationControls(
  controls: RecommendationControls,
): RecommendationControls {
  const seen = new Set<string>()
  const canonicalizeBucket = (values: IngredientReference[]) => {
    const unique = new Map<string, IngredientReference>()
    for (const value of values) {
      const normalized = canonicalizeIngredientReference(value)
      if (!normalized) continue
      const identity = ingredientIdentity(normalized)
      if (seen.has(identity)) continue
      const previous = unique.get(identity)
      if (!previous || compareUnicodeCodeUnits(normalized.name, previous.name) < 0) {
        unique.set(identity, normalized)
      }
    }
    const sorted = [...unique.entries()]
      .sort(([left], [right]) => compareUnicodeCodeUnits(left, right))
      .slice(0, RECOMMENDATION_FILTER_LIMIT)
    sorted.forEach(([identity]) => seen.add(identity))
    return sorted.map(([, value]) => value)
  }

  return {
    preferred: canonicalizeBucket(controls.preferred),
    required: canonicalizeBucket(controls.required),
    excluded: canonicalizeBucket(controls.excluded),
    ingredientScope: controls.ingredientScope,
    sources: { ...controls.sources },
  }
}

export function recommendationControlsKey(controls: RecommendationControls): string {
  return encodeRecommendationControls(controls)
}

export function validateRecommendationControls(
  controls: RecommendationControls,
): RecommendationControlsValidationResult {
  const canonical = canonicalizeRecommendationControls(controls)
  const query = encodeRecommendationControls(canonical)
  return query.length > RECOMMENDATION_URL_MAX_LENGTH
    ? { controls: canonical, status: 'oversized' }
    : { controls: canonical, query, status: 'valid' }
}

export interface RecommendationCanonicalDowngradeResult {
  controls: RecommendationControls | null
  conflictingIdentities: string[]
}

export function tryDowngradeRecommendationCanonicalIds(
  controls: RecommendationControls,
  canonicalIngredientIds: ReadonlySet<string>,
): RecommendationCanonicalDowngradeResult {
  const entries = bucketParameters.flatMap(([bucket]) =>
    controls[bucket].map((value) => {
      const downgraded = Boolean(
        value.canonicalIngredientId && canonicalIngredientIds.has(value.canonicalIngredientId),
      )
      const next = downgraded ? { name: value.name, canonicalIngredientId: null } : value
      return { bucket, downgraded, next, nextIdentity: ingredientIdentity(next) }
    }),
  )
  const conflicts = new Set<string>()
  entries.forEach((entry, index) => {
    if (!entry.downgraded) return
    if (entries.some((candidate, candidateIndex) =>
      candidateIndex !== index && candidate.nextIdentity === entry.nextIdentity
    )) {
      conflicts.add(entry.nextIdentity)
    }
  })
  if (conflicts.size > 0) {
    return {
      controls: null,
      conflictingIdentities: [...conflicts].sort(compareUnicodeCodeUnits),
    }
  }
  const downgraded = {
    ...controls,
    preferred: entries.filter((entry) => entry.bucket === 'preferred').map((entry) => entry.next),
    required: entries.filter((entry) => entry.bucket === 'required').map((entry) => entry.next),
    excluded: entries.filter((entry) => entry.bucket === 'excluded').map((entry) => entry.next),
  }
  return {
    controls: canonicalizeRecommendationControls(downgraded),
    conflictingIdentities: [],
  }
}

export function encodeRecommendationControls(controls: RecommendationControls): string {
  const canonical = canonicalizeRecommendationControls(controls)
  const parameters = new URLSearchParams()
  parameters.set('rv', RECOMMENDATION_URL_VERSION)
  for (const [bucket, parameter] of bucketParameters) {
    for (const ingredient of canonical[bucket]) {
      parameters.append(parameter, encodeIngredient(ingredient))
    }
  }
  parameters.set('scope', encodeScope(canonical.ingredientScope))
  parameters.set(
    'src',
    sourceTokens
      .filter(([field]) => canonical.sources[field])
      .map(([, token]) => token)
      .join(','),
  )
  return parameters.toString()
}

export function parseRecommendationControls(
  search: string,
  defaults: RecommendationControls,
): RecommendationUrlParseResult {
  const fallback = canonicalizeRecommendationControls(defaults)
  const query = search.startsWith('?') ? search.slice(1) : search
  if (!query) return { controls: fallback, issues: [], hadUrlState: false }
  if (query.length > RECOMMENDATION_URL_MAX_LENGTH) {
    return { controls: fallback, issues: ['oversized'], hadUrlState: true }
  }

  const parameters = new URLSearchParams(query)
  const recognized = new Set(['rv', 'p', 'r', 'x', 'scope', 'src'])
  const hasRecognizedState = [...parameters.keys()].some((key) => recognized.has(key))
  if (!hasRecognizedState) {
    return {
      controls: fallback,
      issues: ['invalidParameter'],
      hadUrlState: false,
    }
  }

  const issues = new Set<RecommendationUrlIssue>()
  if ([...parameters.keys()].some((key) => !recognized.has(key))) {
    issues.add('invalidParameter')
  }
  const versions = parameters.getAll('rv')
  if (versions.length !== 1) issues.add('duplicateParameter')
  if (versions[0] !== RECOMMENDATION_URL_VERSION) {
    return {
      controls: fallback,
      issues: [
        ...(versions[0] ? ['unknownVersion' as const] : ['invalidParameter' as const]),
        ...issues,
      ],
      hadUrlState: true,
    }
  }

  const controls: RecommendationControls = {
    preferred: [],
    required: [],
    excluded: [],
    ingredientScope: fallback.ingredientScope,
    sources: { ...fallback.sources },
  }
  const seen = new Map<string, keyof Pick<RecommendationControls, 'preferred' | 'required' | 'excluded'>>()
  for (const [bucket, parameter] of bucketParameters) {
    const values = parameters.getAll(parameter)
    if (values.length > RECOMMENDATION_FILTER_LIMIT) issues.add('oversized')
    for (const encoded of values.slice(0, RECOMMENDATION_FILTER_LIMIT)) {
      const ingredient = decodeIngredient(encoded)
      if (!ingredient) {
        issues.add('invalidIngredient')
        continue
      }
      const identity = ingredientIdentity(ingredient)
      const previousBucket = seen.get(identity)
      if (previousBucket) {
        issues.add(previousBucket === bucket ? 'duplicateParameter' : 'conflictingIngredient')
        continue
      }
      seen.set(identity, bucket)
      controls[bucket].push(ingredient)
    }
  }

  const scopes = parameters.getAll('scope')
  if (scopes.length > 1) issues.add('duplicateParameter')
  if (scopes[0]) {
    const scope = decodeScope(scopes[0])
    if (scope) controls.ingredientScope = scope
    else issues.add('invalidParameter')
  }

  const sourceValues = parameters.getAll('src')
  if (sourceValues.length > 1) issues.add('duplicateParameter')
  if (sourceValues.length > 0) {
    controls.sources = {
      includeFavoriteRecipes: false,
      includeOwn: false,
      includeFavoriteAccounts: false,
      includeOtherPublic: false,
    }
    const tokens = sourceValues[0].split(',').filter(Boolean)
    if (new Set(tokens).size !== tokens.length) issues.add('duplicateParameter')
    for (const token of tokens) {
      const source = sourceTokens.find(([, candidate]) => candidate === token)
      if (source) controls.sources[source[0]] = true
      else issues.add('invalidParameter')
    }
  }

  return {
    controls: canonicalizeRecommendationControls(controls),
    issues: [...issues],
    hadUrlState: true,
  }
}

function canonicalizeIngredientReference(
  value: IngredientReference,
): IngredientReference | null {
  const name = value.name
    .normalize('NFKC')
    .replace(/\u3000/g, ' ')
    .trim()
    .replace(/\s+/g, ' ')
  if (!name || [...name].length > MAX_INGREDIENT_NAME_LENGTH) return null
  const canonicalIngredientId = value.canonicalIngredientId?.trim() || null
  if (canonicalIngredientId && canonicalIngredientId.length > MAX_CANONICAL_ID_LENGTH) return null
  return { name, canonicalIngredientId }
}

function encodeIngredient(value: IngredientReference): string {
  const name = encodeBase64Url(value.name)
  return value.canonicalIngredientId
    ? `c.${encodeBase64Url(value.canonicalIngredientId)}.${name}`
    : `u.${name}`
}

function decodeIngredient(value: string): IngredientReference | null {
  const parts = value.split('.')
  try {
    if (parts.length === 2 && parts[0] === 'u') {
      const ingredient = canonicalizeIngredientReference({
        name: decodeBase64Url(parts[1]),
        canonicalIngredientId: null,
      })
      return ingredient && normalizeIngredient(ingredient.name) ? ingredient : null
    }
    if (parts.length === 3 && parts[0] === 'c') {
      return canonicalizeIngredientReference({
        name: decodeBase64Url(parts[2]),
        canonicalIngredientId: decodeBase64Url(parts[1]),
      })
    }
  } catch {
    return null
  }
  return null
}

function encodeScope(scope: IngredientScope): string {
  if (scope === 'PANTRY_ONLY') return 'pantry'
  if (scope === 'PANTRY_OR_LIST') return 'pantry-list'
  return 'any'
}

function decodeScope(value: string): IngredientScope | null {
  if (value === 'any') return 'ANY'
  if (value === 'pantry') return 'PANTRY_ONLY'
  if (value === 'pantry-list') return 'PANTRY_OR_LIST'
  return null
}

function encodeBase64Url(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, '')
}

function decodeBase64Url(value: string): string {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) throw new Error('invalid base64url')
  const padded = value.replace(/-/g, '+').replace(/_/g, '/').padEnd(Math.ceil(value.length / 4) * 4, '=')
  const binary = atob(padded)
  return new TextDecoder('utf-8', { fatal: true }).decode(
    Uint8Array.from(binary, (character) => character.charCodeAt(0)),
  )
}

function compareUnicodeCodeUnits(left: string, right: string): number {
  if (left === right) return 0
  return left < right ? -1 : 1
}
