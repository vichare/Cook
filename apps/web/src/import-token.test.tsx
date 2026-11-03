import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LocaleProvider } from './i18n'
import {
  COOK_IMPORT_PROMPT_TEMPLATE,
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
      scope: 'recipes:import',
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
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    })
  })

  it('keeps installation and prompt outputs token-free while retaining a separate one-time copy action', async () => {
    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <ImportTokenSettings />
        </QueryClientProvider>
      </LocaleProvider>,
    )

    fireEvent.click(screen.getByRole('button', { name: 'Create 90-day token' }))
    expect(await screen.findByText(secret)).toBeVisible()

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
})
