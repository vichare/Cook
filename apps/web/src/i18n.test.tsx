import { describe, expect, it } from 'vitest'
import {
  catalogs,
  detectInitialLocale,
  formatDateValue,
  localeStorageKey,
  translateMessage,
} from './i18n'

describe('i18n', () => {
  it('keeps every locale catalog complete', () => {
    expect(Object.keys(catalogs['zh-CN']).sort()).toEqual(Object.keys(catalogs.en).sort())
  })

  it('uses a saved preference before browser languages and otherwise detects Chinese', () => {
    expect(detectInitialLocale({ getItem: () => 'en' }, ['zh-CN', 'en'])).toBe('en')
    expect(detectInitialLocale({ getItem: (key) => key === localeStorageKey ? null : null }, ['zh-Hans-CN'])).toBe('zh-CN')
    expect(detectInitialLocale({ getItem: () => null }, ['fr-FR'])).toBe('en')
  })

  it('renders anonymous identity and dates by locale without storing translated identity text', () => {
    expect(translateMessage('en', 'common.anonymousUser')).toBe('Anonymous user')
    expect(translateMessage('zh-CN', 'common.anonymousUser')).toBe('匿名用户')
    expect(formatDateValue('en', '2026-11-04')).toContain('Nov')
    expect(formatDateValue('en', '2026-11-04T03:00:00Z')).toContain('Nov')
    expect(formatDateValue('zh-CN', '2026-11-04')).toContain('2026')
    expect(formatDateValue('zh-CN', '2026-11-04')).toContain('11')
  })
})
