import type {
  Ingredient,
  IngredientAlternative,
  IngredientGroup,
  IngredientReference,
  Recipe,
} from './types'
import type { Translate } from './i18n'

export function normalizeIngredient(value: string): string {
  return value
    .normalize('NFKC')
    .replace(/\u3000/g, ' ')
    .trim()
    .replace(/\s+/g, ' ')
    .toLowerCase()
}

export function splitIngredientList(value: string): string[] {
  const seen = new Set<string>()
  return value
    .split(/[,，、;；\r\n]+/)
    .map((item) => item.normalize('NFKC').replace(/\u3000/g, ' ').trim().replace(/\s+/g, ' '))
    .filter((item) => item.length > 0)
    .filter((item) => {
      const normalized = normalizeIngredient(item)
      if (seen.has(normalized)) return false
      seen.add(normalized)
      return true
    })
}

export function ingredientReferences(value: string): IngredientReference[] {
  return splitIngredientList(value).map((name) => ({ name, canonicalIngredientId: null }))
}

export function newIngredientId(prefix: 'group' | 'alternative'): string {
  return `${prefix}-${crypto.randomUUID()}`
}

export function ingredientIdentity(value: IngredientReference): string {
  return value.canonicalIngredientId
    ? `c:${value.canonicalIngredientId}`
    : `n:${normalizeIngredient(value.name)}`
}

export function legacyIngredientGroups(ingredients: Ingredient[]): IngredientGroup[] {
  return ingredients.map((ingredient, index) => ({
    id: `legacy-group-${index + 1}`,
    optional: false,
    note: '',
    alternatives: [{
      ...ingredient,
      id: `legacy-alternative-${index + 1}`,
    }],
  }))
}

export function recipeIngredientGroups(
  recipe: Pick<Recipe, 'ingredientModel' | 'ingredientGroups' | 'ingredients'>,
): IngredientGroup[] {
  if (recipe.ingredientModel === 'groups-v1' && recipe.ingredientGroups?.length) {
    return recipe.ingredientGroups
  }
  return legacyIngredientGroups(recipe.ingredients ?? [])
}

export function primaryIngredients(groups: IngredientGroup[]): Ingredient[] {
  return groups.flatMap((group) => group.alternatives.slice(0, 1).map((alternative) => ({
    name: alternative.name,
    quantityNote: alternative.quantityNote,
    canonicalIngredientId: alternative.canonicalIngredientId,
  })))
}

export function emptyIngredientGroup(name = ''): IngredientGroup {
  return {
    id: newIngredientId('group'),
    optional: false,
    note: '',
    alternatives: [{
      id: newIngredientId('alternative'),
      name,
      quantityNote: '',
      canonicalIngredientId: null,
    }],
  }
}

export function asIngredientReference(alternative: IngredientAlternative): IngredientReference {
  return {
    name: alternative.name,
    canonicalIngredientId: alternative.canonicalIngredientId,
  }
}

export function localDateValue(date: Date): string {
  const year = date.getFullYear()
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  return `${year}-${month}-${day}`
}

export function expirationShortcut(days: number, from = new Date()): string {
  const date = new Date(from.getFullYear(), from.getMonth(), from.getDate())
  date.setDate(date.getDate() + days)
  return localDateValue(date)
}

export function expirationRelativeDays(expiresOn: string, from = new Date()): number {
  const [expiryYear, expiryMonth, expiryDay] = expiresOn.split('-').map(Number)
  const today = Date.UTC(from.getFullYear(), from.getMonth(), from.getDate())
  const expiry = Date.UTC(expiryYear, expiryMonth - 1, expiryDay)
  return Math.round((expiry - today) / 86_400_000)
}

export function expirationLabel(
  expiresOn: string | null,
  i18n: { t: Translate; formatDate: (value: string | Date) => string },
  today = localDateValue(new Date()),
): string {
  if (!expiresOn) return i18n.t('expiry.none')
  if (expiresOn < today) return i18n.t('expiry.expired', { date: i18n.formatDate(expiresOn) })
  if (expiresOn === today) return i18n.t('expiry.expiresToday')
  const [todayYear, todayMonth, todayDay] = today.split('-').map(Number)
  const [expiryYear, expiryMonth, expiryDay] = expiresOn.split('-').map(Number)
  const days = Math.round(
    (Date.UTC(expiryYear, expiryMonth - 1, expiryDay) -
      Date.UTC(todayYear, todayMonth - 1, todayDay)) /
      86_400_000,
  )
  if (days <= 7) {
    return i18n.t('expiry.expiresIn', {
      days: days === 1 ? i18n.t('common.day') : i18n.t('common.days', { count: days }),
      date: i18n.formatDate(expiresOn),
    })
  }
  return i18n.t('expiry.expiresOn', { date: i18n.formatDate(expiresOn) })
}
