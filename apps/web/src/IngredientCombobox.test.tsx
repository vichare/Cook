import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { IngredientCombobox } from './IngredientCombobox'

const { ingredientSuggestions } = vi.hoisted(() => ({
  ingredientSuggestions: vi.fn().mockResolvedValue([
    {
      canonicalIngredientId: 'tomato-id',
      displayName: 'Tomato',
      matchedText: 'Tomato',
      reason: 'prefix',
    },
  ]),
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
  ingredientSuggestions.mockClear()
})

function renderCombobox(overrides: Partial<React.ComponentProps<typeof IngredientCombobox>> = {}) {
  const onChange = vi.fn()
  const onCommit = vi.fn()
  const onPasteList = vi.fn()
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <IngredientCombobox
        label="Ingredient"
        value={{ name: 'tom', canonicalIngredientId: null }}
        onChange={onChange}
        onCommit={onCommit}
        onPasteList={onPasteList}
        {...overrides}
      />
    </QueryClientProvider>,
  )
  return { onChange, onCommit, onPasteList }
}

describe('ingredient combobox', () => {
  it('announces suggestions and supports keyboard selection', async () => {
    const { onChange, onCommit } = renderCombobox()
    const input = screen.getByRole('combobox', { name: 'Ingredient' })
    fireEvent.focus(input)
    expect(input).toHaveAttribute('aria-expanded', 'true')
    await screen.findByRole('option', { name: /Tomato/ })
    expect(screen.getByText('1 suggestion available.')).toHaveAttribute('aria-live', 'polite')

    fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(input).toHaveAttribute('aria-activedescendant')
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
    renderCombobox()
    const input = screen.getByRole('combobox', { name: 'Ingredient' })
    fireEvent.focus(input)
    await screen.findByRole('option', { name: /Keep “tom” as entered/ })
    fireEvent.keyDown(input, { key: 'Escape' })
    await waitFor(() => expect(input).toHaveAttribute('aria-expanded', 'false'))
  })

  it('splits a pasted multilingual list without silently selecting suggestions', () => {
    const { onPasteList } = renderCombobox()
    fireEvent.paste(screen.getByRole('combobox', { name: 'Ingredient' }), {
      clipboardData: { getData: () => 'Milk，青菜；Rice\nmilk' },
    })
    expect(onPasteList).toHaveBeenCalledWith(['Milk', '青菜', 'Rice'])
  })
})
