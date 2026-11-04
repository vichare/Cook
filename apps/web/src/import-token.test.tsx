import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { LocaleProvider } from './i18n'
import {
  COOK_SKILL_URL,
  createRecipeExample,
  importDraftExample,
  ImportTokenSettings,
  promptForPreset,
  resolveCookBaseUrl,
  skillInstructionsForPreset,
  unixPersistentBaseUrlSetup,
  unixTemporarySetup,
  windowsPersistentBaseUrlSetup,
  windowsTemporarySetup,
} from './router'

const { aiCapabilities, createImportToken, importTokens } = vi.hoisted(() => ({
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
  aiCapabilities: vi.fn().mockResolvedValue({
    prefix: 'cook_imp',
    scopes: ['recipes:import'],
    expiresAt: '2027-02-02T00:00:00Z',
    preset: 'import',
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
      aiCapabilities,
      revokeImportToken: vi.fn(),
    },
  }
})

describe('import token reveal', () => {
  const writeText = vi.fn().mockResolvedValue(undefined)

  beforeEach(() => {
    writeText.mockClear()
    createImportToken.mockClear()
    aiCapabilities.mockClear()
    importTokens.mockReset()
    importTokens.mockResolvedValue([])
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

    expect(screen.getByRole('heading', { name: 'AI client setup' })).toBeVisible()
    expect(screen.getByDisplayValue(`COOK_BASE_URL=${baseUrl}`)).toBeVisible()
    fireEvent.click(screen.getByText('Configure environment variables or a client secret store'))
    expect(screen.getByText(/does not establish an arbitrary Secret Store/)).toBeVisible()
    expect(screen.getByText(/Never put the token in chat/)).toBeVisible()
    fireEvent.click(screen.getByText('Request format'))
    expect(screen.getByText(/total kcal for the whole recipe, not per serving/)).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Copy temporary PowerShell setup' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(windowsTemporarySetup(baseUrl)))
    expect(windowsTemporarySetup(baseUrl)).toContain("Read-Host 'COOK_IMPORT_TOKEN' -AsSecureString")
    expect(windowsTemporarySetup(baseUrl)).not.toContain(secret)
    fireEvent.click(screen.getByRole('button', { name: 'Copy temporary macOS/Linux setup' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(unixTemporarySetup(baseUrl)))
    expect(unixTemporarySetup(baseUrl)).toContain('stty -echo')
    expect(unixTemporarySetup(baseUrl)).toContain('IFS= read -r COOK_IMPORT_TOKEN')
    expect(unixTemporarySetup(baseUrl)).not.toContain(secret)
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
    expect(COOK_SKILL_URL).toContain('github.com/vichare/Cook/tree/v0.4.2/skills/')
    expect(installInstructions).toContain(
      'gh skill install vichare/Cook cook-recipe-import --agent github-copilot --scope user --pin v0.4.2',
    )
    expect(installInstructions).toContain(
      'gh skill preview vichare/Cook cook-recipe-import@v0.4.2',
    )
    expect(installInstructions).toContain('Do not copy only SKILL.md')
    expect(installInstructions).not.toContain('install skill from GitHub URL')

    fireEvent.click(screen.getByRole('button', { name: 'Copy token' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(secret))
    const successMessage =
      'Token copied. Store it as the COOK_IMPORT_TOKEN environment variable; the app may need a restart to inherit it.'
    const firstAnnouncement = screen.getByText(successMessage)
    expect(firstAnnouncement).toHaveTextContent(
      successMessage,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Copy Cook base URL' }))
    const assignment = `COOK_BASE_URL=${baseUrl}`
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(assignment))
    const firstBaseUrlAnnouncement = screen.getByText('COOK_BASE_URL assignment copied.')
    fireEvent.click(screen.getByRole('button', { name: 'Copy Cook base URL' }))
    await waitFor(() => expect(
      writeText.mock.calls.filter(([value]) => value === assignment),
    ).toHaveLength(2))
    expect(screen.getByText('COOK_BASE_URL assignment copied.')).not.toBe(firstBaseUrlAnnouncement)
    fireEvent.click(screen.getByRole('button', { name: 'Copy token' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(secret))
    expect(writeText.mock.calls.filter(([value]) => value === secret)).toHaveLength(2)
    expect(screen.getByText(successMessage)).not.toBe(firstAnnouncement)
    fireEvent.click(screen.getByRole('button', { name: 'I have stored it securely' }))
    expect(screen.queryByText(secret)).not.toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'AI client setup' })).toBeVisible()
    expect(screen.getByDisplayValue(assignment)).toBeVisible()
  })

  it('keeps setup available on a return visit and can select an existing token preset without a secret', async () => {
    const baseUrl = resolveCookBaseUrl()
    importTokens.mockResolvedValueOnce([
      {
        id: 'existing-token',
        name: 'Recipe manager',
        prefix: 'cook_man',
        scopes: ['recipes:read', 'recipes:write', 'recipes:images:write'],
        createdAt: '2026-11-04T00:00:00Z',
        lastUsedAt: null,
        expiresAt: '2027-02-02T00:00:00Z',
        revokedAt: null,
      },
      {
        id: 'publish-token',
        name: 'Recipe publisher',
        prefix: 'cook_pub',
        scopes: ['recipes:read', 'recipes:write', 'recipes:images:write', 'recipes:publish'],
        createdAt: '2026-11-04T00:00:00Z',
        lastUsedAt: null,
        expiresAt: '2026-12-04T00:00:00Z',
        revokedAt: null,
      },
      {
        id: 'duplicate-name-token',
        name: 'Recipe manager',
        prefix: 'cook_man_2',
        scopes: ['recipes:read', 'recipes:write', 'recipes:images:write'],
        createdAt: '2026-11-04T00:00:00Z',
        lastUsedAt: null,
        expiresAt: '2027-02-02T00:00:00Z',
        revokedAt: null,
      },
    ])
    const { unmount } = render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    expect(await screen.findAllByText('Setup preset: Manage my private recipes and cover images'))
      .toHaveLength(2)
    expect(screen.getByRole('button', {
      name: 'Use Manage my private recipes and cover images setup for token Recipe manager (cook_man)',
    })).toBeVisible()
    expect(screen.getByRole('button', {
      name: 'Use Manage and publish recipes setup for token Recipe publisher (cook_pub)',
    })).toBeVisible()
    expect(screen.getByRole('button', {
      name: 'Use Manage my private recipes and cover images setup for token Recipe manager (cook_man_2)',
    })).toBeVisible()
    expect(screen.queryByText(secret)).not.toBeInTheDocument()
    expect(screen.getByDisplayValue(`COOK_BASE_URL=${baseUrl}`)).toBeVisible()
    fireEvent.click(screen.getByRole('button', {
      name: 'Use Manage my private recipes and cover images setup for token Recipe manager (cook_man)',
    }))
    fireEvent.click(screen.getByRole('button', { name: 'Copy prompt template' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(
      promptForPreset('manage', baseUrl),
    ))
    expect(writeText.mock.calls.flat().join('\n')).not.toContain(secret)

    unmount()
    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )
    expect(screen.getByRole('heading', { name: 'AI client setup' })).toBeVisible()
    expect(screen.getByDisplayValue(`COOK_BASE_URL=${baseUrl}`)).toBeVisible()
    expect(screen.queryByText(secret)).not.toBeInTheDocument()
  })

  it('keeps setup guidance aligned with the preset submitted while creation is pending', async () => {
    const baseUrl = resolveCookBaseUrl()
    let resolveCreation: ((value: Awaited<ReturnType<typeof createImportToken>>) => void) | undefined
    createImportToken.mockImplementationOnce(() => new Promise((resolve) => {
      resolveCreation = resolve
    }))
    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    fireEvent.click(screen.getByRole('button', { name: 'Create token' }))
    await waitFor(() => expect(createImportToken).toHaveBeenCalledWith(
      'My AI client',
      90,
      'import',
    ))
    fireEvent.click(screen.getByRole('radio', { name: /Manage my private recipes/ }))
    await act(async () => {
      resolveCreation?.({
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
        secret,
      })
    })

    expect(await screen.findByText(secret)).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Copy prompt template' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(
      promptForPreset('import', baseUrl),
    ))
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
    expect(prompt).toContain('created, updated, skipped, failed, and needs_review')
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
        expect(output).toContain('/api/ai/capabilities')
        expect(output).toContain(JSON.stringify(importDraftExample, null, 2))
        expect(output).toContain(JSON.stringify(createRecipeExample, null, 2))
        expect(output).toContain('Cover upload is multipart, not JSON')
        expect(output).toContain('Stop the batch on 401/403')
        expect(output).toContain('Difficulty and estimatedCalories are optional and may be omitted or null')
        expect(output).toContain('total kcal for the whole recipe, never per serving')
      }
    }
    for (const output of [
      windowsTemporarySetup(baseUrl),
      unixTemporarySetup(baseUrl),
      windowsPersistentBaseUrlSetup(baseUrl),
      unixPersistentBaseUrlSetup(baseUrl),
    ]) {
      expect(output).toContain(baseUrl)
      expect(output).not.toContain(secret)
    }
    expect(windowsPersistentBaseUrlSetup(baseUrl)).not.toContain('COOK_IMPORT_TOKEN')
    expect(unixPersistentBaseUrlSetup(baseUrl)).not.toContain('COOK_IMPORT_TOKEN')
    expect(unixPersistentBaseUrlSetup(baseUrl)).toContain('.zshrc')
    expect(unixPersistentBaseUrlSetup(baseUrl)).toContain('.bashrc')
    expect(unixPersistentBaseUrlSetup(baseUrl)).toContain('launch the client from that shell')
  })

  it('shows self-contained request examples and tests capabilities without leaking the token', async () => {
    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    fireEvent.click(screen.getByText('Request format'))
    expect(screen.getByText(/"schemaVersion": "1.0"/)).toBeVisible()
    expect(screen.getByText(/"expectedVersion": 3/)).toBeVisible()
    expect(screen.getByRole('link', { name: 'Open authoritative import-draft schema' }))
      .toHaveAttribute('href', expect.stringContaining('import-draft-v1.schema.json'))

    fireEvent.click(screen.getByRole('button', { name: 'Create token' }))
    expect(await screen.findByText(secret)).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Test connection' }))
    expect(await screen.findByText('Connection succeeded.')).toBeVisible()
    expect(screen.getByText('Capabilities: recipes:import')).toBeVisible()
    expect(aiCapabilities).toHaveBeenCalledWith(secret)
    expect(screen.getByText('Connection succeeded.').closest('.notice')).not.toHaveTextContent(secret)
  })

  it('shows an accessible localized connection failure without exposing the token', async () => {
    aiCapabilities.mockRejectedValueOnce(new Error('invalid token'))
    render(
      <LocaleProvider initialLocale="zh-CN" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    fireEvent.click(screen.getByRole('button', { name: '创建 token' }))
    expect(await screen.findByText(secret)).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: '测试连接' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('连接测试失败')
    expect(screen.getByRole('alert')).not.toHaveTextContent(secret)
  })

  it('re-announces repeated successful token copies in Chinese', async () => {
    render(
      <LocaleProvider initialLocale="zh-CN" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    expect(screen.getByRole('heading', { name: 'AI 客户端设置' })).toBeVisible()
    expect(screen.getByDisplayValue(`COOK_BASE_URL=${resolveCookBaseUrl()}`)).toBeVisible()
    fireEvent.click(screen.getByText('配置环境变量或客户端 Secret Store'))
    expect(screen.getByText(/完全退出并重启/)).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: '创建 token' }))
    expect(await screen.findByText(secret)).toBeVisible()
    const copyButton = screen.getByRole('button', { name: '复制 token' })
    fireEvent.click(copyButton)
    const firstAnnouncement = await screen.findByText(
      'Token 已复制。请保存为 COOK_IMPORT_TOKEN 环境变量；应用可能需要重启才能读取新变量。',
    )
    fireEvent.click(copyButton)
    await waitFor(() => expect(writeText.mock.calls.filter(([value]) => value === secret)).toHaveLength(2))
    expect(screen.getByText(
      'Token 已复制。请保存为 COOK_IMPORT_TOKEN 环境变量；应用可能需要重启才能读取新变量。',
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
