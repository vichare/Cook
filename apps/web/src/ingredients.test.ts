import { describe, expect, it } from 'vitest'
import {
  expirationLabel,
  expirationShortcut,
  normalizeIngredient,
  oneMonthAfter,
  splitIngredientList,
} from './ingredients'

describe('ingredient input helpers', () => {
  it('mirrors server NFKC normalization and delimiter splitting', () => {
    expect(normalizeIngredient('  ＭＩＬＫ　Tea  ')).toBe('milk tea')
    expect(splitIngredientList(' Ｍｉｌｋ，milk、青菜;\n Rice；')).toEqual(['Milk', '青菜', 'Rice'])
  })

  it('uses local calendar values for date shortcuts and clamps month ends', () => {
    const januaryEnd = new Date(2026, 0, 31, 15, 30)
    expect(expirationShortcut(0, januaryEnd)).toBe('2026-01-31')
    expect(expirationShortcut(1, januaryEnd)).toBe('2026-02-01')
    expect(expirationShortcut(7, januaryEnd)).toBe('2026-02-07')
    expect(oneMonthAfter(januaryEnd)).toBe('2026-02-28')
    expect(oneMonthAfter(new Date(2028, 0, 31, 15, 30))).toBe('2028-02-29')
    expect(expirationShortcut(1, new Date(2028, 1, 28, 23, 30))).toBe('2028-02-29')
  })

  it('describes expiration status with text rather than color alone', () => {
    expect(expirationLabel(null, '2026-11-04')).toBe('No expiration date')
    expect(expirationLabel('2026-11-03', '2026-11-04')).toContain('Expired')
    expect(expirationLabel('2026-11-04', '2026-11-04')).toContain('Expires today')
    expect(expirationLabel('2026-11-05', '2026-11-04')).toContain('Expires in 1 day')
    expect(expirationLabel('2026-12-01', '2026-11-04')).toContain('2026-12-01')
  })
})
