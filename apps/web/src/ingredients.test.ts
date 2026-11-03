import { describe, expect, it } from 'vitest'
import {
  expirationLabel,
  expirationRelativeDays,
  expirationShortcut,
  normalizeIngredient,
  splitIngredientList,
} from './ingredients'
import { formatDateValue, translateMessage } from './i18n'

const i18n = {
  t: (key: Parameters<typeof translateMessage>[1], variables?: Parameters<typeof translateMessage>[2]) =>
    translateMessage('en', key, variables),
  formatDate: (value: string | Date) => formatDateValue('en', value),
}

describe('ingredient input helpers', () => {
  it('mirrors server NFKC normalization and delimiter splitting', () => {
    expect(normalizeIngredient('  ＭＩＬＫ　Tea  ')).toBe('milk tea')
    expect(splitIngredientList(' Ｍｉｌｋ，milk、青菜;\n Rice；')).toEqual(['Milk', '青菜', 'Rice'])
  })

  it('uses local calendar arithmetic across month, year, leap-day, DST, and midnight boundaries', () => {
    const januaryEnd = new Date(2026, 0, 31, 15, 30)
    expect(expirationShortcut(0, januaryEnd)).toBe('2026-01-31')
    expect(expirationShortcut(1, januaryEnd)).toBe('2026-02-01')
    expect(expirationShortcut(7, januaryEnd)).toBe('2026-02-07')
    expect(expirationShortcut(1, new Date(2026, 11, 31, 23, 59))).toBe('2027-01-01')
    expect(expirationShortcut(1, new Date(2028, 1, 28, 23, 30))).toBe('2028-02-29')
    expect(expirationShortcut(1, new Date(2026, 9, 4, 0, 30))).toBe('2026-10-05')
    expect(expirationRelativeDays('2026-10-05', new Date(2026, 9, 4, 23, 59, 59))).toBe(1)
    expect(expirationRelativeDays('2026-01-30', januaryEnd)).toBe(-1)
  })

  it('describes expiration status with text rather than color alone', () => {
    expect(expirationLabel(null, i18n, '2026-11-04')).toBe('No expiration date')
    expect(expirationLabel('2026-11-03', i18n, '2026-11-04')).toContain('Expired')
    expect(expirationLabel('2026-11-04', i18n, '2026-11-04')).toContain('Expires today')
    expect(expirationLabel('2026-11-05', i18n, '2026-11-04')).toContain('1 day')
    expect(expirationLabel('2026-12-01', i18n, '2026-11-04')).toContain('Dec 1, 2026')
  })
})
