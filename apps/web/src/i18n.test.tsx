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

  it('distinguishes Cook users from external source authors in Chinese', () => {
    const ownershipKeys = [
      'common.byAuthor',
      'recommend.source.favoriteAccount',
      'author.intro',
      'author.empty',
      'author.favorite',
      'author.unfavorite',
      'author.favoriteAccounts',
      'author.noFavorites',
      'sources.favoriteAccounts',
    ] as const
    for (const key of ownershipKeys) {
      expect(translateMessage('zh-CN', key)).not.toContain('作者')
    }
    expect(translateMessage('en', 'sources.favoriteAccounts')).toBe(
      'Recipes from favorite Cook users',
    )
    expect(translateMessage('zh-CN', 'recipe.sourceAuthor')).toBe('来源作者')
    expect(translateMessage('zh-CN', 'import.sourceAuthor', { author: '来源用户' }))
      .toContain('来源作者')
  })

  it('localizes recommendation ingredient moves without implementation terminology', () => {
    expect(translateMessage('en', 'recommend.moveConfirmDescription', {
      name: 'Rice',
      from: 'Must use',
      to: 'Do not use',
    })).toBe('Rice is in Must use. Move it to Do not use?')
    expect(translateMessage('zh-CN', 'recommend.moveConfirmDescription', {
      name: '米饭',
      from: '必须使用',
      to: '禁止使用',
    })).toBe('米饭 目前在必须使用。移到禁止使用吗？')
    expect(translateMessage('zh-CN', 'recommend.moveSuccess', {
      name: '米饭',
      bucket: '必须使用',
    })).not.toMatch(/canonical|normalization|identity/i)
  })
})
