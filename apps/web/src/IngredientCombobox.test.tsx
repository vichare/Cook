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
    expect(screen.getByText(/Option 1 of 2: keep as entered/)).toBeInTheDocument()
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(screen.getByText('Option 2 of 2: Tomato, matched existing ingredient.')).toBeInTheDocument()
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

  it('uses simple accessible existing-ingredient copy in Chinese', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: '番茄',
        matchedText: '番茄',
        reason: 'prefix',
      },
    ])
    render(
      <LocaleProvider initialLocale="zh-CN" storage={null}>
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
          <IngredientCombobox
            label="食材"
            value={{ name: '番', canonicalIngredientId: null }}
            onChange={vi.fn()}
          />
        </QueryClientProvider>
      </LocaleProvider>,
    )
    fireEvent.focus(screen.getByRole('combobox', { name: '食材' }))
    await waitFor(() => expect(ingredientSuggestions).toHaveBeenCalled())
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()

    cleanup()
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: '番茄',
        matchedText: '番茄',
        reason: 'prefix',
      },
    ])
    render(
      <LocaleProvider initialLocale="zh-CN" storage={null}>
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
          <IngredientCombobox
            label="食材"
            value={{ name: '番茄酱', canonicalIngredientId: null }}
            onChange={vi.fn()}
          />
        </QueryClientProvider>
      </LocaleProvider>,
    )
    fireEvent.focus(screen.getByRole('combobox', { name: '食材' }))
    expect(await screen.findByRole('option', { name: '番茄，匹配到已有食材' })).toHaveTextContent(
      '番茄匹配到已有食材',
    )
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
    await screen.findByRole('option', { name: 'Tomato, matched existing ingredient' })
    expect(screen.getAllByRole('option')).toHaveLength(1)
    expect(screen.getByRole('option', { name: 'Tomato, matched existing ingredient' })).toHaveTextContent(
      'TomatoMatched existing ingredient',
    )
    expect(screen.queryByText(/→|Exact normalized match/)).not.toBeInTheDocument()
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
    await screen.findByRole('option', { name: 'Plum tomato, matched existing ingredient' })
    expect(screen.getAllByRole('option')).toHaveLength(2)
    expect(ambiguous.onChange).not.toHaveBeenCalled()
    expect(screen.queryByText(/Keep as entered/)).not.toBeInTheDocument()
  })

  it('normalizes width, space, and case for a unique strict exact match', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: 'Tomato',
        matchedText: 'tomato',
        reason: 'exact',
      },
    ])
    const { onChange } = renderCombobox({
      value: { name: '　ＴＯＭＡＴＯ  ', canonicalIngredientId: null },
    })
    fireEvent.focus(screen.getByRole('combobox', { name: 'Ingredient' }))
    await waitFor(() =>
      expect(onChange).toHaveBeenCalledWith({
        name: '　ＴＯＭＡＴＯ  ',
        canonicalIngredientId: 'tomato-id',
      }),
    )
  })

  it('keeps advisory aliases explicit and limits candidates to four after keep', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'tomato-id',
        displayName: 'Tomato',
        matchedText: 'Roma tomato',
        reason: 'alias',
      },
      ...Array.from({ length: 5 }, (_, index) => ({
        canonicalIngredientId: `candidate-${index}`,
        displayName: `Candidate ${index}`,
        matchedText: `Candidate ${index}`,
        reason: 'similar' as const,
      })),
    ])
    const { onChange } = renderCombobox({
      value: { name: 'Roma', canonicalIngredientId: null },
    })
    fireEvent.focus(screen.getByRole('combobox', { name: 'Ingredient' }))
    await screen.findByRole('option', { name: 'Tomato, matched existing ingredient' })
    expect(screen.getAllByRole('option')).toHaveLength(5)
    expect(screen.getAllByRole('option')[0]).toHaveTextContent('Keep as entered')
    expect(screen.getByRole('option', { name: 'Tomato, matched existing ingredient' })).toBeInTheDocument()
    expect(screen.queryByText('Candidate 4')).not.toBeInTheDocument()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('suppresses noisy CJK substring and similar candidates in both locales', async () => {
    ingredientSuggestions.mockResolvedValueOnce([
      {
        canonicalIngredientId: 'oil-id',
        displayName: '油',
        matchedText: '油',
        reason: 'substring',
      },
    ])
    render(
      <LocaleProvider initialLocale="zh-CN" storage={null}>
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
          <IngredientCombobox
            label="食材"
            value={{ name: '酱油', canonicalIngredientId: null }}
            onChange={vi.fn()}
          />
        </QueryClientProvider>
      </LocaleProvider>,
    )
    fireEvent.focus(screen.getByRole('combobox', { name: '食材' }))
    await screen.findByRole('option', { name: /保持原样/ })
    expect(screen.queryByText('油')).not.toBeInTheDocument()
    cleanup()

    ingredientSuggestions.mockResolvedValueOnce([
      {
        canonicalIngredientId: 'chicken-wing-id',
        displayName: '鸡翅',
        matchedText: '鸡翅',
        reason: 'similar',
      },
    ])
    renderCombobox({ value: { name: '鸡腿', canonicalIngredientId: null } })
    fireEvent.focus(screen.getByRole('combobox', { name: 'Ingredient' }))
    await screen.findByRole('option', { name: /Keep as entered/ })
    expect(screen.queryByText('鸡翅')).not.toBeInTheDocument()
  })

  it('does not open a noisy suggestion list for a single Han character', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'oil-id',
        displayName: '油',
        matchedText: '油',
        reason: 'substring',
      },
    ])
    renderCombobox({ value: { name: '油', canonicalIngredientId: null } })
    const input = screen.getByRole('combobox', { name: 'Ingredient' })
    fireEvent.focus(input)
    await waitFor(() => expect(ingredientSuggestions).toHaveBeenCalled())
    expect(input).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(screen.getByText(/type more characters/)).toBeInTheDocument()
  })

  it('shows ambiguous strict matches for a single Han character', async () => {
    ingredientSuggestions.mockResolvedValue([
      {
        canonicalIngredientId: 'oil-id',
        displayName: '油',
        matchedText: '油',
        reason: 'exact',
      },
      {
        canonicalIngredientId: 'confirmed-alias-id',
        displayName: '食用油',
        matchedText: '油',
        reason: 'alias',
      },
    ])
    const { onChange } = renderCombobox({ value: { name: '油', canonicalIngredientId: null } })
    const input = screen.getByRole('combobox', { name: 'Ingredient' })
    fireEvent.focus(input)
    expect(await screen.findAllByRole('option')).toHaveLength(2)
    expect(screen.queryByRole('option', { name: /Keep as entered/ })).not.toBeInTheDocument()
    expect(screen.getByText('2 suggestions available.')).toHaveAttribute('aria-live', 'polite')
    fireEvent.click(screen.getByRole('option', { name: /食用油/ }))
    expect(onChange).toHaveBeenLastCalledWith({
      name: '油',
      canonicalIngredientId: 'confirmed-alias-id',
    })
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
