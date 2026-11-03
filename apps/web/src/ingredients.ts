import type { IngredientReference } from './types'

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

export function oneMonthAfter(from = new Date()): string {
  const targetMonth = from.getMonth() + 1
  const lastDay = new Date(from.getFullYear(), targetMonth + 1, 0).getDate()
  const date = new Date(from.getFullYear(), targetMonth, Math.min(from.getDate(), lastDay))
  return localDateValue(date)
}

export function expirationLabel(expiresOn: string | null, today = localDateValue(new Date())): string {
  if (!expiresOn) return 'No expiration date'
  if (expiresOn < today) return `⚠ Expired ${expiresOn}`
  if (expiresOn === today) return '⚠ Expires today'
  const [todayYear, todayMonth, todayDay] = today.split('-').map(Number)
  const [expiryYear, expiryMonth, expiryDay] = expiresOn.split('-').map(Number)
  const days = Math.round(
    (Date.UTC(expiryYear, expiryMonth - 1, expiryDay) -
      Date.UTC(todayYear, todayMonth - 1, todayDay)) /
      86_400_000,
  )
  if (days <= 7) return `◷ Expires in ${days} day${days === 1 ? '' : 's'} (${expiresOn})`
  return `◷ Expires on ${expiresOn}`
}
