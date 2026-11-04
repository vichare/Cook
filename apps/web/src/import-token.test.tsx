import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { LocaleProvider } from './i18n'
import {
  COOK_SKILL_URL,
  ImportTokenSettings,
  promptForPreset,
  resolveCookBaseUrl,
  skillInstructionsForPreset,
} from './router'

const { createImportToken, importTokens } = vi.hoisted(() => ({
  importTokens: vi.fn().mockResolvedValue([]),
  createImportToken: vi.fn().mockResolvedValue({
    token: {
      id: 'token-id',
      name: 'My AI client',
      prefix: 'cook_imp',
      scopes: ['recipes:import'],
      createdAt: '2026-11-04T00:00:00Z',
      lastUsedAt: null,
      expiresAt: '2027-02-02T00:00:00Z',
      revokedAt: null,
    },
    secret: 'cook_import_real_secret_must_not_leak',
  }),
}))
const secret = 'cook_import_real_secret_must_not_leak'

vi.mock('./api', async () => {
  const actual = await vi.importActual<typeof import('./api')>('./api')
  return {
    ...actual,
    api: {
      ...actual.api,
      importTokens,
      createImportToken,
      revokeImportToken: vi.fn(),
    },
  }
})

describe('import token reveal', () => {
  const writeText = vi.fn().mockResolvedValue(undefined)

  beforeEach(() => {
    writeText.mockClear()
    createImportToken.mockClear()
    Object.defineProperty(window, 'confirm', {
      configurable: true,
      value: vi.fn().mockReturnValue(true),
    })
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
  })
  afterEach(cleanup)

  it('keeps installation and prompt outputs token-free while retaining a separate one-time copy action', async () => {
    const baseUrl = resolveCookBaseUrl()
    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    fireEvent.click(screen.getByRole('button', { name: 'Create token' }))
    expect(await screen.findByText(secret)).toBeVisible()
    expect(createImportToken).toHaveBeenCalledWith('My AI client', 90, 'import')

    fireEvent.click(screen.getByRole('button', { name: 'Copy installation instructions' }))
    const installInstructions = skillInstructionsForPreset('import', baseUrl)
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(installInstructions))
    expect(installInstructions).toContain(`COOK_BASE_URL=${baseUrl}`)
    expect(installInstructions).not.toContain(secret)

    fireEvent.click(screen.getByRole('button', { name: 'Copy prompt template' }))
    const prompt = promptForPreset('import', baseUrl)
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(prompt))
    expect(prompt).toContain('COOK_IMPORT_TOKEN')
    expect(prompt).toContain(`${baseUrl}/api/import/drafts`)
    expect(prompt).not.toContain(secret)
    expect(COOK_SKILL_URL).toContain('github.com/vichare/Cook/blob/main/skills/')

    fireEvent.click(screen.getByRole('button', { name: 'Copy token' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(secret))
    const successMessage =
      'Token copied. Store it in your AI client secret store or COOK_IMPORT_TOKEN environment variable.'
    const firstAnnouncement = screen.getByText(successMessage)
    expect(firstAnnouncement).toHaveTextContent(
      successMessage,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Copy Cook base URL' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(baseUrl))
    fireEvent.click(screen.getByRole('button', { name: 'Copy token' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(secret))
    expect(writeText.mock.calls.filter(([value]) => value === secret)).toHaveLength(2)
    expect(screen.getByText(successMessage)).not.toBe(firstAnnouncement)
    fireEvent.click(screen.getByRole('button', { name: 'I have stored it securely' }))
    expect(screen.queryByText(secret)).not.toBeInTheDocument()
  })

  it('uses the manage preset without embedding the one-time token in its prompt', async () => {
    const baseUrl = resolveCookBaseUrl()
    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    fireEvent.click(screen.getByRole('radio', { name: /Manage my private recipes/ }))
    fireEvent.change(screen.getByLabelText('Token expiry'), { target: { value: 'none' } })
    fireEvent.click(screen.getByRole('button', { name: 'Create token' }))
    expect(await screen.findByText(secret)).toBeVisible()
    expect(createImportToken).toHaveBeenCalledWith('My AI client', null, 'manage')

    fireEvent.click(screen.getByRole('button', { name: 'Copy installation instructions' }))
    const instructions = skillInstructionsForPreset('manage', baseUrl)
    await waitFor(() =>
      expect(writeText).toHaveBeenLastCalledWith(instructions),
    )
    expect(instructions).not.toContain(secret)

    fireEvent.click(screen.getByRole('button', { name: 'Copy prompt template' }))
    const prompt = promptForPreset('manage', baseUrl)
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(prompt))
    expect(prompt).toContain('COOK_IMPORT_TOKEN')
    expect(prompt).not.toContain(secret)
    expect(prompt).toContain('pending revision')
  })

  it('requires explicit confirmation and uses short-lived token-free publish guidance', async () => {
    const baseUrl = resolveCookBaseUrl()
    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    fireEvent.click(screen.getByRole('radio', { name: /Manage and publish recipes/ }))
    expect(screen.getByLabelText('Token expiry')).toHaveValue('30')
    expect(screen.getByRole('alert')).toHaveTextContent('AI content will become public immediately')
    fireEvent.change(screen.getByLabelText('Token expiry'), { target: { value: 'none' } })
    expect(screen.getAllByRole('alert')[1]).toHaveTextContent('has no automatic expiry')
    fireEvent.change(screen.getByLabelText('Token expiry'), { target: { value: '30' } })

    vi.mocked(window.confirm).mockReturnValueOnce(false)
    fireEvent.click(screen.getByRole('button', { name: 'Create token' }))
    expect(createImportToken).not.toHaveBeenCalled()

    vi.mocked(window.confirm).mockReturnValueOnce(true)
    fireEvent.click(screen.getByRole('button', { name: 'Create token' }))
    expect(await screen.findByText(secret)).toBeVisible()
    expect(createImportToken).toHaveBeenCalledWith('My AI client', 30, 'publish')

    fireEvent.click(screen.getByRole('button', { name: 'Copy installation instructions' }))
    const instructions = skillInstructionsForPreset('publish', baseUrl)
    await waitFor(() =>
      expect(writeText).toHaveBeenLastCalledWith(instructions),
    )
    expect(instructions).toContain('recipes:publish')
    expect(instructions).not.toContain(secret)

    fireEvent.click(screen.getByRole('button', { name: 'Copy prompt template' }))
    const prompt = promptForPreset('publish', baseUrl)
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(prompt))
    expect(prompt).toContain('created, updated, skipped, and failed')
    expect(prompt).toContain('recipe ID or URL')
    expect(prompt).not.toContain(secret)
  })

  it('generates every reusable output with the resolved base URL and no base placeholder', () => {
    const baseUrl = 'https://cook.example.test'
    for (const preset of ['import', 'manage', 'publish'] as const) {
      for (const output of [
        skillInstructionsForPreset(preset, baseUrl),
        promptForPreset(preset, baseUrl),
      ]) {
        expect(output).toContain(`COOK_BASE_URL=${baseUrl}`)
        expect(output).toContain(baseUrl)
        expect(output).not.toContain('{COOK_BASE_URL}')
        expect(output).not.toContain('COOK_BASE_URL=https://cook.vichare.net')
        expect(output).not.toContain(secret)
      }
    }
  })

  it('re-announces repeated successful token copies in Chinese', async () => {
    render(
      <LocaleProvider initialLocale="zh-CN" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    fireEvent.click(screen.getByRole('button', { name: '创建 token' }))
    expect(await screen.findByText(secret)).toBeVisible()
    const copyButton = screen.getByRole('button', { name: '复制 token' })
    fireEvent.click(copyButton)
    const firstAnnouncement = await screen.findByText(
      'Token 已复制。请保存到 AI 客户端 secret store 或 COOK_IMPORT_TOKEN 环境变量。',
    )
    fireEvent.click(copyButton)
    await waitFor(() => expect(writeText.mock.calls.filter(([value]) => value === secret)).toHaveLength(2))
    expect(screen.getByText(
      'Token 已复制。请保存到 AI 客户端 secret store 或 COOK_IMPORT_TOKEN 环境变量。',
    )).not.toBe(firstAnnouncement)
  })

  it.each([
    ['en', 'Copy failed. Select the text manually or check clipboard permission.'],
    ['zh-CN', '复制失败。请手动选择文字，或检查剪贴板权限。'],
  ] as const)('announces token copy failures accessibly in %s without claiming success', async (locale, message) => {
    writeText.mockRejectedValueOnce(new Error('clipboard denied'))
    render(
      <LocaleProvider initialLocale={locale} storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    fireEvent.click(screen.getByRole('button', { name: locale === 'en' ? 'Create token' : '创建 token' }))
    expect(await screen.findByText(secret)).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: locale === 'en' ? 'Copy token' : '复制 token' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(message)
    expect(screen.queryByText(/Token copied|Token 已复制/)).not.toBeInTheDocument()
    expect(screen.getByText(secret)).toBeVisible()
  })
})
