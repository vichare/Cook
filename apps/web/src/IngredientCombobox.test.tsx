import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { IngredientCombobox } from './IngredientCombobox'
import { LocaleProvider } from './i18n'

const { ingredientSuggestions } = vi.hoisted(() => ({
  ingredientSuggestions: vi.fn(),
}))

vi.mock('./api', async () => {
  const actual = await vi.importActual<typeof import('./api')>('./api')
  return {
    ...actual,
    api: {
      ...actual.api,
      ingredientSuggestions,
    },
  }
})

afterEach(() => {
  cleanup()
  ingredientSuggestions.mockReset()
})

function renderCombobox(overrides: Partial<React.ComponentProps<typeof IngredientCombobox>> = {}) {
  const onChange = vi.fn()
  const onCommit = vi.fn()
  const onPasteList = vi.fn()
  render(
    <LocaleProvider initialLocale="en" storage={null}>
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <IngredientCombobox
          label="Ingredient"
          value={{ name: 'tom', canonicalIngredientId: null }}
          onChange={onChange}
          onCommit={onCommit}
          onPasteList={onPasteList}
          {...overrides}
        />
      </QueryClientProvider>
    </LocaleProvider>,
  )
  return { onChange, onCommit, onPasteList }
}

describe('ingredient combobox', () => {
  it('announces suggestions and supports keyboard selection', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: 'Tomato',
        matchedText: 'Tomato',
        reason: 'prefix',
      },
    ])
    const { onChange, onCommit } = renderCombobox()
    const input = screen.getByRole('combobox', { name: 'Ingredient' })
    fireEvent.focus(input)
    expect(input).toHaveAttribute('aria-expanded', 'true')
    expect(input).toHaveAttribute('aria-haspopup', 'listbox')
    expect(input).toHaveAttribute('aria-busy', 'true')
    expect(ingredientSuggestions).not.toHaveBeenCalled()
    await screen.findByRole('option', { name: /Tomato/ })
    expect(ingredientSuggestions).toHaveBeenCalledTimes(1)
    expect(input).toHaveAttribute('aria-busy', 'false')
    expect(screen.getByText('1 suggestion available.')).toHaveAttribute('aria-live', 'polite')

    fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(input).toHaveAttribute('aria-activedescendant')
    expect(screen.getByText(/Option 1 of 2: tom → Tomato/)).toBeInTheDocument()
    expect(onCommit).not.toHaveBeenCalled()
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(onChange).toHaveBeenLastCalledWith({
      name: 'tom',
      canonicalIngredientId: 'tomato-id',
    })
    expect(onCommit).toHaveBeenCalledWith({
      name: 'tom',
      canonicalIngredientId: 'tomato-id',
    })
  })

  it('keeps the original value available and closes with Escape', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: 'Tomato',
        matchedText: 'Tomato',
        reason: 'prefix',
      },
    ])
    renderCombobox()
    const input = screen.getByRole('combobox', { name: 'Ingredient' })
    fireEvent.focus(input)
    await screen.findByRole('option', {
      name: /Keep as entered — don’t link another ingredient; save this name as entered/,
    })
    fireEvent.keyDown(input, { key: 'Escape' })
    await waitFor(() => expect(input).toHaveAttribute('aria-expanded', 'false'))
  })

  it('uses the approved keep-original wording in Chinese', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: '番茄',
        matchedText: '番茄',
        reason: 'similar',
      },
    ])
    render(
      <LocaleProvider initialLocale="zh-CN" storage={null}>
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
          <IngredientCombobox
            label="食材"
            value={{ name: '蕃茄', canonicalIngredientId: null }}
            onChange={vi.fn()}
          />
        </QueryClientProvider>
      </LocaleProvider>,
    )
    fireEvent.focus(screen.getByRole('combobox', { name: '食材' }))
    expect(await screen.findByText('保持原样 - 不关联其他食材，按当前名称保存')).toBeInTheDocument()
    expect(screen.getByText('将“蕃茄”按当前输入保存，不关联到其他食材。')).toBeInTheDocument()
  })

  it('auto-selects and renders only one unique exact canonical match', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: 'Tomato',
        matchedText: 'Tomato',
        reason: 'exact',
      },
      {
        canonicalIngredientId: 'tomato-sauce-id',
        displayName: 'Tomato sauce',
        matchedText: 'Tomato sauce',
        reason: 'prefix',
      },
    ])
    const { onChange } = renderCombobox({
      value: { name: 'Ｔｏｍａｔｏ', canonicalIngredientId: null },
    })
    const input = screen.getByRole('combobox', { name: 'Ingredient' })
    fireEvent.focus(input)
    await screen.findByText(/Ｔｏｍａｔｏ → Tomato/)
    expect(screen.getAllByRole('option')).toHaveLength(1)
    expect(screen.getByRole('option', { name: /Tomato/ })).toBeInTheDocument()
    expect(screen.queryByText(/Keep as entered/)).not.toBeInTheDocument()
    await waitFor(() =>
      expect(onChange).toHaveBeenCalledWith({
        name: 'Ｔｏｍａｔｏ',
        canonicalIngredientId: 'tomato-id',
      }),
    )
  })

  it('auto-selects one exact owner alias but fails safe for ambiguous exact matches', async () => {
    ingredientSuggestions.mockResolvedValueOnce([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: 'Tomato',
        matchedText: 'Roma tomato',
        reason: 'alias',
      },
    ])
    const unique = renderCombobox({
      value: { name: 'Roma tomato', canonicalIngredientId: null },
    })
    fireEvent.focus(screen.getByRole('combobox', { name: 'Ingredient' }))
    await waitFor(() =>
      expect(unique.onChange).toHaveBeenCalledWith({
        name: 'Roma tomato',
        canonicalIngredientId: 'tomato-id',
      }),
    )
    expect(screen.queryByText(/Keep as entered/)).not.toBeInTheDocument()
    cleanup()

    ingredientSuggestions.mockResolvedValueOnce([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: 'Tomato',
        matchedText: 'Tomato',
        reason: 'exact',
      },
      {
        canonicalIngredientId: 'plum-tomato-id',
        displayName: 'Plum tomato',
        matchedText: 'Tomato',
        reason: 'alias',
      },
    ])
    const ambiguous = renderCombobox({
      value: { name: 'Tomato', canonicalIngredientId: null },
    })
    fireEvent.focus(screen.getByRole('combobox', { name: 'Ingredient' }))
    await screen.findByText(/Tomato → Plum tomato/)
    expect(screen.getAllByRole('option')).toHaveLength(3)
    expect(ambiguous.onChange).not.toHaveBeenCalled()
    expect(screen.getByText(/Keep as entered/)).toBeInTheDocument()
  })

  it('splits a pasted multilingual list without silently selecting suggestions', () => {
    ingredientSuggestions.mockResolvedValue([])
    const { onPasteList } = renderCombobox()
    fireEvent.paste(screen.getByRole('combobox', { name: 'Ingredient' }), {
      clipboardData: { getData: () => 'Milk，青菜；Rice\nmilk' },
    })
    expect(onPasteList).toHaveBeenCalledWith(['Milk', '青菜', 'Rice'])
  })

  it('lets the user undo an explicit canonical selection', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: 'Tomato',
        matchedText: 'Tomato',
        reason: 'prefix',
      },
    ])
    const { onChange } = renderCombobox({
      value: { name: 'Tomato', canonicalIngredientId: 'tomato-id' },
    })
    await screen.findByRole('button', { name: 'Undo selected match' })
    fireEvent.click(screen.getByRole('button', { name: 'Undo selected match' }))
    expect(onChange).toHaveBeenCalledWith({ name: 'Tomato', canonicalIngredientId: null })
  })
})
