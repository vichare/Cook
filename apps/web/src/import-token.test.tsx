import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { LocaleProvider } from './i18n'
import {
  COOK_IMPORT_PROMPT_TEMPLATE,
  COOK_MANAGE_PROMPT_TEMPLATE,
  COOK_MANAGE_SKILL_INSTALL_INSTRUCTIONS,
  COOK_PUBLISH_PROMPT_TEMPLATE,
  COOK_PUBLISH_SKILL_INSTALL_INSTRUCTIONS,
  COOK_SKILL_INSTALL_INSTRUCTIONS,
  COOK_SKILL_URL,
  ImportTokenSettings,
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
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(COOK_SKILL_INSTALL_INSTRUCTIONS))
    expect(COOK_SKILL_INSTALL_INSTRUCTIONS).not.toContain(secret)

    fireEvent.click(screen.getByRole('button', { name: 'Copy prompt template' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(COOK_IMPORT_PROMPT_TEMPLATE))
    expect(COOK_IMPORT_PROMPT_TEMPLATE).toContain('COOK_IMPORT_TOKEN')
    expect(COOK_IMPORT_PROMPT_TEMPLATE).not.toContain(secret)
    expect(COOK_SKILL_URL).toContain('github.com/vichare/Cook/blob/main/skills/')

    fireEvent.click(screen.getByRole('button', { name: 'Copy token' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(secret))
    fireEvent.click(screen.getByRole('button', { name: 'I have stored it securely' }))
    expect(screen.queryByText(secret)).not.toBeInTheDocument()
  })

  it('uses the manage preset without embedding the one-time token in its prompt', async () => {
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
    await waitFor(() =>
      expect(writeText).toHaveBeenLastCalledWith(COOK_MANAGE_SKILL_INSTALL_INSTRUCTIONS),
    )
    expect(COOK_MANAGE_SKILL_INSTALL_INSTRUCTIONS).not.toContain(secret)

    fireEvent.click(screen.getByRole('button', { name: 'Copy prompt template' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(COOK_MANAGE_PROMPT_TEMPLATE))
    expect(COOK_MANAGE_PROMPT_TEMPLATE).toContain('COOK_IMPORT_TOKEN')
    expect(COOK_MANAGE_PROMPT_TEMPLATE).not.toContain(secret)
    expect(COOK_MANAGE_PROMPT_TEMPLATE).toContain('pending revision')
  })

  it('requires explicit confirmation and uses short-lived token-free publish guidance', async () => {
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
    await waitFor(() =>
      expect(writeText).toHaveBeenLastCalledWith(COOK_PUBLISH_SKILL_INSTALL_INSTRUCTIONS),
    )
    expect(COOK_PUBLISH_SKILL_INSTALL_INSTRUCTIONS).toContain('recipes:publish')
    expect(COOK_PUBLISH_SKILL_INSTALL_INSTRUCTIONS).not.toContain(secret)

    fireEvent.click(screen.getByRole('button', { name: 'Copy prompt template' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith(COOK_PUBLISH_PROMPT_TEMPLATE))
    expect(COOK_PUBLISH_PROMPT_TEMPLATE).toContain('created, updated, skipped, and failed')
    expect(COOK_PUBLISH_PROMPT_TEMPLATE).toContain('recipe ID or URL')
    expect(COOK_PUBLISH_PROMPT_TEMPLATE).not.toContain(secret)
  })
})
