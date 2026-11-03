import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { PantryConfirmation } from './router'

vi.mock('./api', async () => {
  const actual = await vi.importActual<typeof import('./api')>('./api')
  return {
    ...actual,
    api: {
      ...actual.api,
      savePantry: vi.fn().mockResolvedValue([]),
    },
  }
})

describe('pantry confirmation', () => {
  it('supports skipping and clears skipped answers', () => {
    render(
      <QueryClientProvider client={new QueryClient()}>
        <PantryConfirmation open onOpenChange={() => undefined} names={['Egg', 'Milk', 'Rice']} />
      </QueryClientProvider>,
    )
    expect(screen.getByRole('heading', { name: 'Quick pantry check' })).toBeVisible()
    expect(screen.getByRole('button', { name: 'Skip for now' })).toBeVisible()
    expect(screen.getByRole('button', { name: 'Save answers' })).toBeDisabled()
    fireEvent.click(screen.getAllByLabelText('Still have it')[0])
    expect(screen.getByRole('button', { name: 'Save answers' })).toBeEnabled()
    fireEvent.click(screen.getByRole('button', { name: 'Skip for now' }))
    expect(screen.getByRole('button', { name: 'Save answers' })).toBeDisabled()
  })
})
