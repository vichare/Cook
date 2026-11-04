import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import {
  RouterProvider,
  createMemoryHistory,
  createRoute,
  createRootRoute,
  createRouter,
} from '@tanstack/react-router'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  AccountSecuritySettings,
  EmailOtpLogin,
  ExpirationEditor,
  GuestLanguageSelector,
  HomePage,
  LanguageSettings,
  PantryConfirmation,
  PantryPage,
  PrimaryNavigation,
  ProfileSetup,
  RecipeCoverFields,
  RecipeConflictNotice,
  RecipeDetailContent,
  RecipeEditorForm,
  RecipeIngredientList,
  RecipeMetadataFields,
  RecipeProvenanceFields,
  RecipesPage,
  RecommendationsPage,
  ShoppingListPage,
  persistRecipeEdit,
  moderationMutationInput,
  moderationEvidenceImageUrl,
  pantryStatusForIngredient,
  recipeSchema,
} from './router'
import type { RecipeForm } from './router'
import { ApiError } from './api'
import { LocaleProvider, useI18n, type Locale } from './i18n'
import type { ModerationQueueItem, PantryItem, Recipe, Recommendation } from './types'
import {
  encodeRecommendationControls,
  parseRecommendationControls,
  recommendationControlsFromSettings,
} from './recommendation-url'

const storedLocale = { value: null as string | null }
const testStorage = {
  getItem: () => storedLocale.value,
  setItem: (_key: string, value: string) => {
    storedLocale.value = value
  },
}

function Providers({ children, locale = 'en' }: { children: React.ReactNode; locale?: Locale }) {
  return (
    <LocaleProvider initialLocale={locale} storage={testStorage}>
      <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>
    </LocaleProvider>
  )
}

describe('moderation mutations', () => {
  it('uses the documented review reason independently of enforcement reasons', () => {
    const item: ModerationQueueItem = {
      recipeId: 'recipe',
      title: 'Soup',
      authorHandle: 'author',
      reviewState: 'unreviewed',
      moderationState: 'clear',
      recipeVersion: 4,
      moderationVersion: 7,
      caseId: 'case',
      caseVersion: 3,
      reportCount: 1,
      activeAppeal: false,
      updatedAt: '2026-01-01T00:00:00Z',
    }

    expect(moderationMutationInput(item, 'review', 'policy_violation')).toEqual({
      expectedRecipeVersion: 4,
      expectedModerationVersion: 7,
      expectedCaseVersion: 3,
      reasonCode: 'no_action_needed',
    })
  })

  it('uses the case-bound evidence image endpoint', () => {
    expect(moderationEvidenceImageUrl('case/id')).toBe(
      '/api/admin/moderation/cases/case%2Fid/evidence-image',
    )
  })
})

const {
  deleteRecipeCover,
  deleteShoppingItem,
  ingredientSuggestionsApi,
  addRecipeIngredientToShopping,
  purchaseShoppingItem,
  pantryApi,
  recommendationSettings,
  recommendationsApi,
  recipesApi,
  recipeApi,
  requestEmailLink,
  requestEmailOtp,
  saveLocale,
  saveProfile,
  saveRecipe,
  savePantry,
  securitySettings,
  shoppingItems,
  updateRecipeCoverMetadata,
  uploadRecipeCover,
  verifyEmailLink,
  verifyEmailOtp,
} = vi.hoisted(() => ({
  saveProfile: vi.fn().mockImplementation((nickname: string | null) =>
    Promise.resolve({
      id: 'user',
      email: 'private@example.test',
      displayName: 'Google Name',
      nickname,
      onboardingCompleted: true,
      locale: 'en',
    }),
  ),
  saveLocale: vi.fn().mockImplementation((locale: 'en' | 'zh-CN') =>
    Promise.resolve({
      id: 'user',
      email: 'private@example.test',
      displayName: 'Google Name',
      nickname: null,
      onboardingCompleted: true,
      locale,
    }),
  ),
  requestEmailOtp: vi.fn().mockResolvedValue({
    challengeId: 'challenge',
    expiresAt: '2026-11-04T00:10:00Z',
    resendAllowedAt: '2026-11-04T00:00:00Z',
  }),
  verifyEmailOtp: vi.fn().mockResolvedValue({
    id: 'user',
    email: 'private@example.test',
    displayName: 'Private',
    nickname: null,
    onboardingCompleted: false,
    locale: 'en' as const,
  }),
  securitySettings: vi.fn().mockResolvedValue({
    linkedEmails: [],
    googleEmail: null,
    googleSignInStatus: 'unavailable',
    passkeys: [],
  }),
  requestEmailLink: vi.fn().mockResolvedValue({
    challengeId: 'link-challenge',
    expiresAt: '2026-11-04T00:10:00Z',
    resendAllowedAt: '2026-11-04T00:00:00Z',
  }),
  verifyEmailLink: vi.fn().mockResolvedValue({
    linkedEmails: ['linked@example.test'],
    googleEmail: null,
    googleSignInStatus: 'unsupported',
    passkeys: [],
  }),
  shoppingItems: vi.fn().mockResolvedValue([
    {
      id: 'auto:pantry-1',
      name: 'Milk',
      canonicalIngredientId: null,
      automatic: true,
      pantryItemId: 'pantry-1',
      sources: [{ kind: 'automatic', label: '', recipeId: null }],
      version: 1,
      purchaseOptions: {
        resolution: 'reuse',
        candidates: [{
          id: 'pantry-1', name: 'Milk', status: 'out', quantityNote: '', expiresOn: null,
          canonicalIngredientId: null, autoAddToShoppingListWhenOut: true, version: 1,
        }],
      },
    },
    {
      id: 'manual-1',
      name: 'Rice',
      canonicalIngredientId: null,
      automatic: false,
      pantryItemId: null,
      sources: [{ kind: 'manual', label: '', recipeId: null }],
      version: 1,
      purchaseOptions: { resolution: 'create', candidates: [] },
    },
  ]),
  purchaseShoppingItem: vi.fn().mockResolvedValue({
    pantry: {
      id: 'pantry-1', name: 'Milk', status: 'have', quantityNote: '', confirmedAt: '2026-11-04T00:00:00Z',
      expiresOn: null, canonicalIngredientId: null, autoAddToShoppingListWhenOut: true, version: 2,
    },
    shoppingItems: [],
    purchaseActionId: 'purchase-1',
    undoAvailable: true,
  }),
  pantryApi: vi.fn().mockResolvedValue([
    {
      id: 'pantry-1', name: 'Milk', status: 'out', quantityNote: '', confirmedAt: null,
      expiresOn: null, canonicalIngredientId: null, autoAddToShoppingListWhenOut: true, version: 1,
    },
  ]),
  recommendationSettings: vi.fn().mockResolvedValue({
    includeFavoriteRecipes: true,
    includeOwn: true,
    includeFavoriteAccounts: true,
    includeOtherPublic: true,
    verificationDays: 14,
  }),
  recommendationsApi: vi.fn(),
  recipesApi: vi.fn(),
  addRecipeIngredientToShopping: vi.fn(),
  saveRecipe: vi.fn(),
  recipeApi: vi.fn(),
  uploadRecipeCover: vi.fn(),
  updateRecipeCoverMetadata: vi.fn(),
  deleteRecipeCover: vi.fn(),
  deleteShoppingItem: vi.fn().mockResolvedValue({
    outcome: 'removed',
    requiresDisableAutoRestock: false,
    shoppingItems: [],
    undoAvailable: false,
  }),
  ingredientSuggestionsApi: vi.fn().mockResolvedValue([]),
  savePantry: vi.fn().mockResolvedValue([]),
}))

vi.mock('./api', async () => {
  const actual = await vi.importActual<typeof import('./api')>('./api')
  return {
    ...actual,
    api: {
      ...actual.api,
      savePantry,
      purchaseShoppingItem,
      pantry: pantryApi,
      shoppingItems,
      addShoppingItem: vi.fn().mockResolvedValue([]),
      updateShoppingItem: vi.fn().mockResolvedValue([]),
      deleteShoppingItem,
      ingredientSuggestions: ingredientSuggestionsApi,
      settings: recommendationSettings,
      recommendations: recommendationsApi,
      recipes: recipesApi,
      favorite: vi.fn().mockResolvedValue(undefined),
      addRecipeIngredientToShopping,
      undoShoppingPurchase: vi.fn().mockResolvedValue([]),
      saveProfile,
      saveRecipe,
      recipe: recipeApi,
      uploadRecipeCover,
      updateRecipeCoverMetadata,
      deleteRecipeCover,
      saveLocale,
      requestEmailOtp,
      verifyEmailOtp,
      securitySettings,
      requestEmailLink,
      verifyEmailLink,
      startPasskeyRegistration: vi.fn(),
      finishPasskeyRegistration: vi.fn(),
      revokePasskey: vi.fn(),
    },
  }
})

afterEach(() => {
  cleanup()
  window.history.replaceState({}, '', '/')
  ingredientSuggestionsApi.mockReset().mockResolvedValue([])
})

describe('shopping list', () => {
  it('uses the same Bought flow for automatic and manual items', async () => {
    shoppingItems.mockClear()
    purchaseShoppingItem.mockClear()
    render(
      <Providers>
        <ShoppingListPage />
      </Providers>,
    )
    expect(await screen.findByRole('button', { name: 'Bought Milk' })).toBeVisible()
    expect(screen.getByRole('button', { name: 'Bought Rice' })).toBeVisible()
    expect(screen.queryByText('Completed items')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Bought Milk' }))
    expect(screen.getByRole('dialog', { name: 'Update pantry' })).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Save to pantry and remove from list' }))
    await waitFor(() => expect(purchaseShoppingItem).toHaveBeenCalled())
  })

  it('defaults an uncertain concrete pantry match to creating with the typed name', async () => {
    shoppingItems.mockReset().mockResolvedValue([{
      id: 'manual-scallion',
      name: 'Scallion',
      canonicalIngredientId: null,
      automatic: false,
      pantryItemId: null,
      sources: [{ kind: 'manual', label: '', recipeId: null }],
      version: 2,
      purchaseOptions: {
        resolution: 'choice',
        candidates: [{
          id: 'pantry-green-onion', name: 'Green onion', status: 'unknown', quantityNote: '',
          expiresOn: null, canonicalIngredientId: 'green-onion',
          autoAddToShoppingListWhenOut: false, version: 4,
        }],
      },
    }])
    purchaseShoppingItem.mockReset().mockResolvedValue({
      pantry: {
        id: 'new-scallion', name: 'Scallion', status: 'have', quantityNote: '',
        confirmedAt: '2026-11-04T00:00:00Z', expiresOn: null,
        canonicalIngredientId: null, autoAddToShoppingListWhenOut: false, version: 1,
      },
      shoppingItems: [],
      purchaseActionId: 'purchase-scallion',
      undoAvailable: true,
    })
    render(<Providers><ShoppingListPage /></Providers>)
    fireEvent.click(await screen.findByRole('button', { name: 'Bought Scallion' }))
    expect(screen.getByRole('radio', { name: 'Create with this name: Scallion' })).toBeChecked()
    expect(screen.getByRole('radio', {
      name: /Use existing pantry item: Green onion/,
    })).not.toBeChecked()
    fireEvent.click(screen.getByRole('button', { name: 'Save to pantry and remove from list' }))
    await waitFor(() => expect(purchaseShoppingItem).toHaveBeenCalledWith(
      'manual-scallion',
      expect.objectContaining({
        pantryItemId: null,
        pantryExpectedVersion: null,
        createUsingCurrentName: true,
      }),
      expect.any(String),
    ))
  })

  it('explains automatic-only removal, uses the disable action, and restores focus on cancel', async () => {
    shoppingItems.mockReset().mockResolvedValue([{
      id: 'auto:pantry-1',
      name: 'Milk',
      canonicalIngredientId: null,
      automatic: true,
      pantryItemId: 'pantry-1',
      sources: [{ kind: 'automatic', label: '', recipeId: null }],
      version: 3,
      purchaseOptions: { resolution: 'reuse', candidates: [] },
    }])
    savePantry.mockReset().mockResolvedValue([])
    deleteShoppingItem.mockClear()
    render(<Providers><ShoppingListPage /></Providers>)
    const remove = await screen.findByRole('button', { name: 'Remove Milk from shopping list' })
    fireEvent.click(remove)
    const dialog = screen.getByRole('dialog', { name: 'Remove from shopping list?' })
    expect(dialog).toHaveTextContent('will remain or reappear')
    expect(within(dialog).queryByRole('button', { name: 'Remove saved sources' })).not.toBeInTheDocument()
    expect(deleteShoppingItem).not.toHaveBeenCalled()
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(remove).toHaveFocus())
    fireEvent.click(remove)
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Turn off automatic restock' }))
    await waitFor(() => expect(savePantry).toHaveBeenCalled())
    expect(await screen.findByText(/Automatic restock for Milk was turned off/)).toBeVisible()
  })

  it('removes concrete and choice items directly with accurate live feedback', async () => {
    const choiceItem = {
      id: 'choice-1',
      name: 'Milk or oat milk',
      canonicalIngredientId: null,
      choiceAlternatives: [
        { id: 'milk', name: 'Milk', canonicalIngredientId: 'milk-id' },
        { id: 'oat', name: 'Oat milk', canonicalIngredientId: 'oat-id' },
      ],
      automatic: false,
      pantryItemId: null,
      sources: [{ kind: 'recipe' as const, label: 'Porridge', recipeId: 'recipe-1' }],
      version: 4,
      purchaseOptions: { resolution: 'create' as const, candidates: [] },
    }
    shoppingItems.mockReset().mockResolvedValue([choiceItem])
    deleteShoppingItem.mockReset().mockResolvedValue({
      outcome: 'removed',
      requiresDisableAutoRestock: false,
      shoppingItems: [],
      undoAvailable: false,
    })
    render(<Providers><ShoppingListPage /></Providers>)
    fireEvent.click(await screen.findByRole('button', {
      name: 'Remove Milk or oat milk from shopping list',
    }))
    await waitFor(() => expect(deleteShoppingItem).toHaveBeenCalledWith(
      'choice-1',
      4,
      expect.any(String),
    ))
    expect(screen.queryByRole('dialog', { name: 'Remove from shopping list?' })).not.toBeInTheDocument()
    expect(await screen.findByText('Milk or oat milk was removed from the shopping list.')).toBeVisible()
  })

  it('does not claim merged automatic items disappeared and refreshes after a conflict', async () => {
    const merged = {
      id: 'merged-1',
      name: 'Milk',
      canonicalIngredientId: 'milk-id',
      automatic: true,
      pantryItemId: 'pantry-1',
      sources: [
        { kind: 'automatic' as const, label: '', recipeId: null },
        { kind: 'recipe' as const, label: 'Soup', recipeId: 'recipe-1' },
      ],
      version: 7,
      purchaseOptions: { resolution: 'reuse' as const, candidates: [] },
    }
    shoppingItems.mockReset().mockResolvedValue([merged])
    deleteShoppingItem.mockReset().mockResolvedValue({
      outcome: 'automaticRemains',
      requiresDisableAutoRestock: true,
      shoppingItems: [{
        ...merged,
        sources: [{ kind: 'automatic', label: '', recipeId: null }],
        version: 8,
      }],
      undoAvailable: false,
    })
    render(<Providers><ShoppingListPage /></Providers>)
    fireEvent.click(await screen.findByRole('button', { name: 'Remove Milk from shopping list' }))
    expect(screen.getByRole('dialog')).toHaveTextContent('automatic item active')
    fireEvent.click(screen.getByRole('button', { name: 'Remove saved sources' }))
    expect(await screen.findByRole('status')).toHaveTextContent('automatic item remains')
    cleanup()

    const { ApiError } = await import('./api')
    shoppingItems.mockReset().mockResolvedValue([merged])
    deleteShoppingItem.mockReset().mockRejectedValue(new ApiError('stale', 409, 'conflict'))
    render(<Providers><ShoppingListPage /></Providers>)
    fireEvent.click(await screen.findByRole('button', { name: 'Remove Milk from shopping list' }))
    fireEvent.click(screen.getByRole('button', { name: 'Remove saved sources' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('refreshed the latest version')
    await waitFor(() => expect(shoppingItems.mock.calls.length).toBeGreaterThan(1))
  })

  describe('recipe concurrency', () => {
    const originalRecipe: Recipe = {
      id: 'recipe-1',
      version: 3,
      isOwner: true,
      authorHandle: 'author',
      authorNickname: null,
      title: 'Original title',
      description: '',
      instructions: 'Stir.',
      visibility: 'private',
      publicationIntent: 'private',
      moderationState: 'clear',
      moderationReviewState: 'unreviewed',
      moderationVersion: 1,
      difficulty: null,
      estimatedCalories: null,
      ingredients: [{ name: 'water', quantityNote: '', canonicalIngredientId: null }],
      ingredientModel: 'groups-v1',
      ingredientGroups: [{
        id: 'group-1',
        optional: false,
        note: '',
        sortOrder: 0,
        alternatives: [{
          id: 'alternative-1',
          name: 'water',
          quantityNote: '',
          canonicalIngredientId: null,
          sortOrder: 0,
        }],
      }],
      isFavorite: false,
      isFavoriteAuthor: false,
      coverImage: {
        id: 'cover-1',
        url: '/api/recipe-images/cover-1',
        width: 800,
        height: 600,
        altText: 'Original cover',
        sourceUrl: null,
        sourceLicense: null,
        sourceAttribution: null,
        version: 1,
      },
      source: null,
    }
    const editedForm: RecipeForm = {
      title: 'Edited title',
      description: '',
      instructions: 'Stir.',
      visibility: 'private',
      difficulty: null,
      estimatedCalories: null,
      sourceUrl: '',
      sourceTitle: '',
      sourceAuthor: '',
      sourceLicense: '',
      sourceAttribution: '',
      ingredientGroups: [{
        id: 'group-1',
        optional: false,
        note: '',
        alternatives: [{ id: 'alternative-1', name: 'water', quantityNote: '', canonicalIngredientId: null }],
      }],
    }
    const coverMetadata = {
      altText: 'Updated cover',
      sourceUrl: '',
      sourceLicense: '',
      sourceAttribution: '',
    }

    afterEach(() => {
      recipeApi.mockReset()
      saveRecipe.mockReset()
      uploadRecipeCover.mockReset()
      updateRecipeCoverMetadata.mockReset()
      deleteRecipeCover.mockReset()
    })

    it('does not PUT an unchanged grouped recipe after form parsing rebuilds field order', async () => {
      const unchangedForm: RecipeForm = {
        ...editedForm,
        title: originalRecipe.title,
      }
      recipeApi.mockResolvedValueOnce(originalRecipe)
      await expect(persistRecipeEdit({
        value: unchangedForm,
        recipe: originalRecipe,
        acknowledgedVersion: originalRecipe.version,
        coverFile: null,
        removeCover: false,
        coverMetadata: {
          altText: originalRecipe.coverImage!.altText,
          sourceUrl: '',
          sourceLicense: '',
          sourceAttribution: '',
        },
        coverMutationKey: 'unchanged-grouped-key',
        acknowledgeVersion: vi.fn(),
      })).resolves.toMatchObject({ version: originalRecipe.version })
      expect(saveRecipe).not.toHaveBeenCalled()
      expect(updateRecipeCoverMetadata).not.toHaveBeenCalled()
    })

    it('performs a cover-only mutation without an unchanged grouped body PUT', async () => {
      const unchangedForm: RecipeForm = {
        ...editedForm,
        title: originalRecipe.title,
      }
      recipeApi.mockResolvedValueOnce(originalRecipe)
      updateRecipeCoverMetadata.mockResolvedValueOnce({ version: 4 })
      await expect(persistRecipeEdit({
        value: unchangedForm,
        recipe: originalRecipe,
        acknowledgedVersion: originalRecipe.version,
        coverFile: null,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'cover-only-key',
        acknowledgeVersion: vi.fn(),
      })).resolves.toMatchObject({ version: 4 })
      expect(saveRecipe).not.toHaveBeenCalled()
      expect(updateRecipeCoverMetadata).toHaveBeenCalledWith(
        originalRecipe.id,
        coverMetadata,
        originalRecipe.version,
        'cover-only-key',
      )
    })

    it('normalizes historical non-continuous orders without treating them as content changes', async () => {
      const historicalRecipe: Recipe = {
        ...originalRecipe,
        ingredientGroups: [{
          id: 'group-1',
          optional: false,
          note: '',
          sortOrder: 14,
          alternatives: [{
            id: 'alternative-1',
            name: 'water',
            quantityNote: '',
            canonicalIngredientId: null,
            sortOrder: 9,
          }],
        }],
      }
      const unchangedForm: RecipeForm = {
        ...editedForm,
        title: historicalRecipe.title,
      }
      recipeApi.mockResolvedValueOnce(historicalRecipe)
      await expect(persistRecipeEdit({
        value: unchangedForm,
        recipe: historicalRecipe,
        acknowledgedVersion: historicalRecipe.version,
        coverFile: null,
        removeCover: false,
        coverMetadata: {
          altText: historicalRecipe.coverImage!.altText,
          sourceUrl: '',
          sourceLicense: '',
          sourceAttribution: '',
        },
        coverMutationKey: 'historical-order-key',
        acknowledgeVersion: vi.fn(),
      })).resolves.toMatchObject({ version: historicalRecipe.version })
      expect(saveRecipe).not.toHaveBeenCalled()
    })

    it('keeps unsaved input and offers comparison guidance after a stale conflict', async () => {
      const { ApiError } = await import('./api')
      render(
          <Providers>
            <input aria-label="Unsaved title" defaultValue="Unsaved local title" />
            <RecipeConflictNotice
              error={new ApiError('stale', 409, 'conflict')}
              recipeId="recipe-1"
            />
          </Providers>,
      )
      const title = screen.getByRole('textbox', { name: 'Unsaved title' })
      expect(screen.getByText(/changed in another tab or AI client/i)).toBeVisible()
      expect(title).toHaveValue('Unsaved local title')
      expect(screen.getByRole('link', { name: 'Open latest version in another tab' })).toHaveAttribute(
        'target',
        '_blank',
      )
    })

    it('saves trimmed recipe provenance and treats an external provenance edit as a conflict', async () => {
      const withSource: RecipeForm = {
        ...editedForm,
        sourceUrl: ' https://example.test/source ',
        sourceTitle: ' Source title ',
        sourceAuthor: ' Source author ',
        sourceLicense: ' CC-BY-4.0 ',
        sourceAttribution: ' Source author — Source title ',
      }
      const saved = {
        ...originalRecipe,
        title: withSource.title,
        version: 4,
        source: {
          url: 'https://example.test/source',
          title: 'Source title',
          author: 'Source author',
          license: 'CC-BY-4.0',
          attribution: 'Source author — Source title',
          warnings: [],
          confidence: null,
        },
      }
      recipeApi.mockResolvedValueOnce(originalRecipe)
      saveRecipe.mockResolvedValueOnce(saved)
      await persistRecipeEdit({
        value: withSource,
        recipe: originalRecipe,
        acknowledgedVersion: 3,
        coverFile: null,
        removeCover: false,
        coverMetadata: {
          altText: originalRecipe.coverImage!.altText,
          sourceUrl: '',
          sourceLicense: '',
          sourceAttribution: '',
        },
        coverMutationKey: 'provenance-save-key',
        acknowledgeVersion: vi.fn(),
      })
      expect(saveRecipe).toHaveBeenCalledWith(
        expect.objectContaining({
          sourceUrl: 'https://example.test/source',
          sourceTitle: 'Source title',
          sourceAuthor: 'Source author',
          sourceLicense: 'CC-BY-4.0',
          sourceAttribution: 'Source author — Source title',
        }),
        'recipe-1',
        3,
      )

      recipeApi.mockResolvedValueOnce({
        ...saved,
        version: 5,
        source: { ...saved.source!, license: 'External license change' },
      })
      await expect(persistRecipeEdit({
        value: withSource,
        recipe: originalRecipe,
        acknowledgedVersion: 4,
        coverFile: null,
        removeCover: false,
        coverMetadata: {
          altText: originalRecipe.coverImage!.altText,
          sourceUrl: '',
          sourceLicense: '',
          sourceAttribution: '',
        },
        coverMutationKey: 'provenance-conflict-key',
        acknowledgeVersion: vi.fn(),
      })).rejects.toEqual(expect.objectContaining({ status: 409, code: 'conflict' }))
    })

    it('serializes edited groups and alternatives with continuous visible-order sort values', async () => {
      const groupedRecipe: Recipe = {
        ...originalRecipe,
        ingredientGroups: [
          {
            id: 'egg',
            optional: false,
            note: '',
            sortOrder: 8,
            alternatives: [
              {
                id: 'egg-primary',
                name: 'Egg',
                quantityNote: '2',
                canonicalIngredientId: null,
                sortOrder: 5,
              },
              {
                id: 'egg-substitute',
                name: 'Duck egg',
                quantityNote: '2',
                canonicalIngredientId: null,
                sortOrder: 9,
              },
            ],
          },
          {
            id: 'water',
            optional: false,
            note: '',
            sortOrder: 20,
            alternatives: [{
              id: 'water-primary',
              name: 'Water',
              quantityNote: '200 ml',
              canonicalIngredientId: null,
              sortOrder: 7,
            }],
          },
        ],
      }
      const saved = { ...groupedRecipe, version: 4 }
      recipeApi.mockResolvedValueOnce(groupedRecipe)
      saveRecipe.mockResolvedValueOnce(saved)
      render(
        <Providers>
          <RecipeEditorForm recipe={groupedRecipe} onSaved={vi.fn()} />
        </Providers>,
      )

      fireEvent.change(screen.getByLabelText('Group 1, alternative 2'), {
        target: { value: 'Quail egg' },
      })
      fireEvent.click(screen.getByRole('button', { name: 'Save recipe' }))

      await waitFor(() => expect(saveRecipe).toHaveBeenCalledTimes(1))
      expect(saveRecipe).toHaveBeenCalledWith(
        expect.objectContaining({
          ingredientModel: 'groups-v1',
          ingredientGroups: [
            expect.objectContaining({
              id: 'egg',
              sortOrder: 0,
              alternatives: [
                expect.objectContaining({ id: 'egg-primary', sortOrder: 0 }),
                expect.objectContaining({
                  id: 'egg-substitute',
                  name: 'Quail egg',
                  sortOrder: 1,
                }),
              ],
            }),
            expect.objectContaining({
              id: 'water',
              sortOrder: 1,
              alternatives: [
                expect.objectContaining({ id: 'water-primary', sortOrder: 0 }),
              ],
            }),
          ],
        }),
        groupedRecipe.id,
        groupedRecipe.version,
      )
    })

    it('retries a failed cover upload using the acknowledged body version', async () => {
      const editedRecipe = { ...originalRecipe, ...editedForm, version: 4 }
      const file = new File(['cover'], 'cover.png', { type: 'image/png' })
      let acknowledgedVersion = originalRecipe.version
      recipeApi.mockResolvedValueOnce(originalRecipe)
      saveRecipe.mockResolvedValueOnce(editedRecipe)
      uploadRecipeCover.mockRejectedValueOnce(new Error('upload failed'))

      await expect(persistRecipeEdit({
        value: editedForm,
        recipe: originalRecipe,
        acknowledgedVersion,
        coverFile: file,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'upload-retry-key',
        acknowledgeVersion: (version) => {
          acknowledgedVersion = version
        },
      })).rejects.toThrow('upload failed')
      expect(acknowledgedVersion).toBe(4)

      recipeApi.mockResolvedValueOnce(editedRecipe)
      uploadRecipeCover.mockResolvedValueOnce({ version: 5 })
      await expect(persistRecipeEdit({
        value: editedForm,
        recipe: originalRecipe,
        acknowledgedVersion,
        coverFile: file,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'upload-retry-key',
        acknowledgeVersion: (version) => {
          acknowledgedVersion = version
        },
      })).resolves.toMatchObject({ version: 5 })
      expect(saveRecipe).toHaveBeenCalledTimes(1)
      expect(uploadRecipeCover).toHaveBeenLastCalledWith(
        'recipe-1',
        file,
        coverMetadata,
        4,
        'upload-retry-key',
      )
    })

    it('retries failed cover metadata using the acknowledged body version', async () => {
      const editedRecipe = { ...originalRecipe, ...editedForm, version: 4 }
      let acknowledgedVersion = originalRecipe.version
      recipeApi.mockResolvedValueOnce(originalRecipe)
      saveRecipe.mockResolvedValueOnce(editedRecipe)
      updateRecipeCoverMetadata.mockRejectedValueOnce(new Error('metadata failed'))

      await expect(persistRecipeEdit({
        value: editedForm,
        recipe: originalRecipe,
        acknowledgedVersion,
        coverFile: null,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'metadata-retry-key',
        acknowledgeVersion: (version) => {
          acknowledgedVersion = version
        },
      })).rejects.toThrow('metadata failed')
      expect(acknowledgedVersion).toBe(4)

      recipeApi.mockResolvedValueOnce(editedRecipe)
      updateRecipeCoverMetadata.mockResolvedValueOnce({ version: 5 })
      await expect(persistRecipeEdit({
        value: editedForm,
        recipe: originalRecipe,
        acknowledgedVersion,
        coverFile: null,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'metadata-retry-key',
        acknowledgeVersion: (version) => {
          acknowledgedVersion = version
        },
      })).resolves.toMatchObject({ version: 5 })
      expect(saveRecipe).toHaveBeenCalledTimes(1)
      expect(updateRecipeCoverMetadata).toHaveBeenLastCalledWith(
        'recipe-1',
        coverMetadata,
        4,
        'metadata-retry-key',
      )
    })

    it('replays a lost successful upload response with the same mutation key', async () => {
      const file = new File(['cover'], 'cover.png', { type: 'image/png' })
      const committed = {
        ...originalRecipe,
        ...editedForm,
        version: 5,
        coverImage: {
          ...originalRecipe.coverImage!,
          id: 'committed-cover',
          version: 2,
          altText: coverMetadata.altText,
        },
      }
      uploadRecipeCover.mockResolvedValueOnce({ version: 5 })
      recipeApi.mockResolvedValueOnce(committed)

      await expect(persistRecipeEdit({
        value: editedForm,
        recipe: originalRecipe,
        acknowledgedVersion: 4,
        coverFile: file,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'lost-upload-key',
        acknowledgeVersion: vi.fn(),
      })).resolves.toMatchObject({ version: 5 })
      expect(uploadRecipeCover).toHaveBeenCalledWith(
        'recipe-1',
        file,
        coverMetadata,
        4,
        'lost-upload-key',
      )
    })

    it('rejects an old replay result after a later external body update', async () => {
      const file = new File(['cover'], 'cover.png', { type: 'image/png' })
      recipeApi.mockResolvedValueOnce({
        ...originalRecipe,
        version: 6,
        title: 'External body update',
        coverImage: {
          ...originalRecipe.coverImage!,
          id: 'committed-cover',
          version: 2,
          altText: coverMetadata.altText,
        },
      })

      await expect(persistRecipeEdit({
        value: editedForm,
        recipe: originalRecipe,
        acknowledgedVersion: 4,
        coverFile: file,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'lost-upload-key',
        acknowledgeVersion: vi.fn(),
      })).rejects.toEqual(expect.objectContaining({
        status: 409,
        code: 'conflict',
      }))
      expect(uploadRecipeCover).not.toHaveBeenCalled()
    })

    it.each([
      ['metadata update', {
        ...originalRecipe.coverImage!,
        id: 'committed-cover',
        version: 3,
        altText: 'External metadata',
      }],
      ['replacement', {
        ...originalRecipe.coverImage!,
        id: 'external-cover',
        version: 1,
        altText: coverMetadata.altText,
      }],
      ['deletion', null],
    ])('rejects an old replay result after a later external cover %s', async (_change, currentCover) => {
      const file = new File(['cover'], 'cover.png', { type: 'image/png' })
      const acknowledgeVersion = vi.fn()
      recipeApi.mockResolvedValueOnce({
        ...originalRecipe,
        ...editedForm,
        version: 6,
        coverImage: currentCover,
      })
      uploadRecipeCover.mockResolvedValueOnce({ version: 5 })

      await expect(persistRecipeEdit({
        value: editedForm,
        recipe: originalRecipe,
        acknowledgedVersion: 4,
        coverFile: file,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'lost-upload-key',
        acknowledgeVersion,
      })).rejects.toEqual(expect.objectContaining({
        status: 409,
        code: 'conflict',
      }))
      expect(acknowledgeVersion).not.toHaveBeenCalled()
    })

    it('does not accept an external same-metadata cover as the attempted upload', async () => {
      const { ApiError } = await import('./api')
      const file = new File(['my-cover'], 'cover.png', { type: 'image/png' })
      recipeApi.mockResolvedValueOnce({
        ...originalRecipe,
        ...editedForm,
        version: 5,
        coverImage: {
          ...originalRecipe.coverImage!,
          id: 'external-cover',
          version: 2,
          altText: coverMetadata.altText,
        },
      })
      uploadRecipeCover.mockRejectedValueOnce(
        new ApiError('recipe changed', 409, 'conflict'),
      )

      await expect(persistRecipeEdit({
        value: editedForm,
        recipe: originalRecipe,
        acknowledgedVersion: 4,
        coverFile: file,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'uncommitted-upload-key',
        acknowledgeVersion: vi.fn(),
      })).rejects.toEqual(expect.objectContaining({
        status: 409,
        code: 'conflict',
      }))
      expect(uploadRecipeCover).toHaveBeenCalledWith(
        'recipe-1',
        file,
        coverMetadata,
        4,
        'uncommitted-upload-key',
      )
    })

    it('retains the conflict UX for genuine external changes', async () => {
      recipeApi.mockResolvedValueOnce({
        ...originalRecipe,
        version: 4,
        title: 'External title',
      })
      await expect(persistRecipeEdit({
        value: editedForm,
        recipe: originalRecipe,
        acknowledgedVersion: 3,
        coverFile: null,
        removeCover: false,
        coverMetadata,
        coverMutationKey: 'external-conflict-key',
        acknowledgeVersion: vi.fn(),
      })).rejects.toEqual(expect.objectContaining({
        status: 409,
        code: 'conflict',
      }))
      expect(saveRecipe).not.toHaveBeenCalled()
    })
  })

  function RecipeMetadataHarness({
    initialDifficulty = null,
    initialCalories = null,
    onSubmit = vi.fn(),
  }: {
    initialDifficulty?: number | null
    initialCalories?: number | null
    onSubmit?: (value: RecipeForm) => void
  }) {
    const { t } = useI18n()
    const form = useForm<RecipeForm>({
      resolver: zodResolver(recipeSchema(t)),
      defaultValues: {
        title: 'Recipe',
        description: '',
        instructions: '',
        visibility: 'private',
        difficulty: initialDifficulty,
        estimatedCalories: initialCalories,
        sourceUrl: '',
        sourceTitle: '',
        sourceAuthor: '',
        sourceLicense: '',
        sourceAttribution: '',
        ingredientGroups: [{
          id: 'group-1',
          optional: false,
          note: '',
          alternatives: [{ id: 'alternative-1', name: 'water', quantityNote: '', canonicalIngredientId: null }],
        }],
      },
    })
    return (
      <form noValidate onSubmit={form.handleSubmit(onSubmit)}>
        <input type="hidden" {...form.register('title')} />
        <input type="hidden" {...form.register('description')} />
        <input type="hidden" {...form.register('instructions')} />
        <input type="hidden" {...form.register('visibility')} />
        <input type="hidden" {...form.register('sourceUrl')} />
        <input type="hidden" {...form.register('sourceTitle')} />
        <input type="hidden" {...form.register('sourceAuthor')} />
        <input type="hidden" {...form.register('sourceLicense')} />
        <input type="hidden" {...form.register('sourceAttribution')} />
        <input type="hidden" {...form.register('ingredientGroups.0.id')} />
        <input type="hidden" {...form.register('ingredientGroups.0.alternatives.0.id')} />
        <input type="hidden" {...form.register('ingredientGroups.0.alternatives.0.name')} />
        <input type="hidden" {...form.register('ingredientGroups.0.alternatives.0.quantityNote')} />
        <input type="hidden" {...form.register('ingredientGroups.0.alternatives.0.canonicalIngredientId')} />
        <RecipeMetadataFields form={form} />
        <button type="submit">Submit metadata</button>
      </form>
    )
  }

  describe('recipe metadata', () => {
    it('uses clearable numeric and plain-language controls', () => {
      render(
        <Providers>
          <RecipeMetadataHarness />
        </Providers>,
      )
      const difficulty = screen.getByLabelText('Difficulty')
      fireEvent.change(difficulty, { target: { value: '4' } })
      expect(difficulty).toHaveValue('4')
      expect(screen.getByRole('option', { name: '4 — Challenging' })).toBeVisible()
      const calories = screen.getByRole('textbox', { name: /Estimated total calories/ })
      fireEvent.change(calories, { target: { value: '850' } })
      expect(calories).toHaveValue('850')
      fireEvent.change(difficulty, { target: { value: '' } })
      expect(difficulty).toHaveValue('')
    })

    it.each([
      ['en' as const, 'Difficulty', /Estimated total calories/],
      ['zh-CN' as const, '难度', /整份预估热量/],
    ])('submits empty and cleared optional values repeatedly in %s', async (locale, difficultyLabel, caloriesLabel) => {
      const onSubmit = vi.fn()
      const { unmount } = render(
        <Providers locale={locale}>
          <RecipeMetadataHarness onSubmit={onSubmit} />
        </Providers>,
      )
      const firstDifficulty = screen.getByLabelText(difficultyLabel)
      const firstCalories = screen.getByRole('textbox', { name: caloriesLabel })
      expect(firstDifficulty).toBeValid()
      expect(firstCalories).toBeValid()
      fireEvent.click(screen.getByRole('button', { name: 'Submit metadata' }))
      await waitFor(() => expect(onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({ difficulty: null, estimatedCalories: null }),
        expect.anything(),
      ))
      unmount()

      render(
        <Providers locale={locale}>
          <RecipeMetadataHarness
            initialDifficulty={4}
            initialCalories={850}
            onSubmit={onSubmit}
          />
        </Providers>,
      )
      const difficulty = screen.getByLabelText(difficultyLabel)
      const calories = screen.getByRole('textbox', { name: caloriesLabel })
      fireEvent.change(difficulty, { target: { value: '' } })
      fireEvent.change(calories, { target: { value: '' } })
      fireEvent.click(screen.getByRole('button', { name: 'Submit metadata' }))
      fireEvent.click(screen.getByRole('button', { name: 'Submit metadata' }))
      await waitFor(() => expect(onSubmit).toHaveBeenLastCalledWith(
        expect.objectContaining({ difficulty: null, estimatedCalories: null }),
        expect.anything(),
      ))
    })

    it('keeps malformed and out-of-range non-empty values visible and reports localized errors', async () => {
      const onSubmit = vi.fn()
      render(
        <Providers locale="zh-CN">
          <RecipeMetadataHarness onSubmit={onSubmit} />
        </Providers>,
      )
      const difficulty = screen.getByLabelText('难度')
      const invalidOption = document.createElement('option')
      invalidOption.value = '6'
      invalidOption.textContent = '6'
      difficulty.append(invalidOption)
      fireEvent.change(difficulty, { target: { value: '6' } })
      const calories = screen.getByRole('textbox', { name: /整份预估热量/ })
      fireEvent.change(calories, { target: { value: '1.5' } })
      fireEvent.click(screen.getByRole('button', { name: 'Submit metadata' }))
      expect(await screen.findByText('请选择 1 到 5 的整数难度，或留空。')).toBeVisible()
      expect(screen.getByText('请输入 1 到 100000 的整份整数估算热量，或留空。')).toBeVisible()
      expect(calories).toHaveValue('1.5')
      expect(onSubmit).not.toHaveBeenCalled()
    })

    it('disables native constraint validation on the full recipe editor', () => {
      render(
        <Providers>
          <RecipeEditorForm onSaved={vi.fn()} />
        </Providers>,
      )
      const form = screen.getByRole('button', { name: 'Save recipe' }).closest('form')
      expect(form).toHaveAttribute('novalidate')
      expect(screen.getByLabelText('Difficulty')).not.toBeRequired()
      expect(screen.getByRole('textbox', { name: /Estimated total calories/ })).not.toBeRequired()
    })

    it('localizes difficulty and whole-recipe calorie guidance', () => {
      render(
        <Providers locale="zh-CN">
          <RecipeMetadataHarness />
        </Providers>,
      )
      expect(screen.getByRole('option', { name: '1 — 非常容易' })).toBeVisible()
      expect(screen.getByText(/整份菜谱的估算值/)).toBeVisible()
    })
  })

  describe('recipe ingredient pantry badges', () => {
    const pantry = (overrides: Partial<PantryItem>): PantryItem => ({
      id: 'pantry-1',
      name: 'Milk',
      status: 'have',
      quantityNote: '',
      confirmedAt: null,
      expiresOn: null,
      canonicalIngredientId: null,
      aliases: [],
      autoAddToShoppingListWhenOut: false,
      version: 1,
      ...overrides,
    })

    it('uses only canonical, confirmed alias, or exact unresolved matches', () => {
      expect(pantryStatusForIngredient(
        { name: 'dairy', quantityNote: '', canonicalIngredientId: 'milk-id' },
        [pantry({ canonicalIngredientId: 'milk-id' })],
      )).toBe('available')
      expect(pantryStatusForIngredient(
        { name: '牛奶', quantityNote: '', canonicalIngredientId: null },
        [pantry({ canonicalIngredientId: 'milk-id', aliases: ['牛奶'] })],
      )).toBe('available')
      expect(pantryStatusForIngredient(
        { name: ' milk ', quantityNote: '', canonicalIngredientId: null },
        [pantry({})],
      )).toBe('available')
      expect(pantryStatusForIngredient(
        { name: 'mil', quantityNote: '', canonicalIngredientId: null },
        [pantry({})],
      )).toBeNull()
      expect(pantryStatusForIngredient(
        { name: 'Milk', quantityNote: '', canonicalIngredientId: 'other-id' },
        [pantry({ canonicalIngredientId: 'milk-id' })],
      )).toBeNull()
    })

    it('maps have, out, unknown, and expired states without a no-match placeholder', () => {
      const ingredient = { name: 'Milk', quantityNote: '', canonicalIngredientId: null }
      expect(pantryStatusForIngredient(ingredient, [pantry({ status: 'have' })])).toBe('available')
      expect(pantryStatusForIngredient(ingredient, [pantry({ status: 'out' })])).toBe('out')
      expect(pantryStatusForIngredient(ingredient, [pantry({ status: 'unknown' })])).toBe('unsure')
      expect(pantryStatusForIngredient(
        ingredient,
        [pantry({ status: 'have', expiresOn: '2020-01-01' })],
      )).toBe('verify')
      expect(pantryStatusForIngredient(
        { name: 'Egg', quantityNote: '', canonicalIngredientId: null },
        [pantry({})],
      )).toBeNull()
    })

    it('renders localized text and an ingredient-specific accessible label', async () => {
      const { unmount } = render(
        <Providers>
          <RecipeIngredientList ingredients={[
            { name: 'Milk', quantityNote: '1 cup', canonicalIngredientId: null },
            { name: 'Egg', quantityNote: '', canonicalIngredientId: null },
          ]} />
        </Providers>,
      )
      expect(await screen.findByLabelText('Milk: pantry status Out')).toHaveTextContent('Out')
      expect(screen.queryByText('Unsure')).not.toBeInTheDocument()
      unmount()
      render(
        <Providers locale="zh-CN">
          <RecipeIngredientList ingredients={[
            { name: 'Milk', quantityNote: '', canonicalIngredientId: null },
          ]} />
        </Providers>,
      )
      expect(await screen.findByLabelText('Milk：库存状态为已用光')).toHaveTextContent('已用光')
    })
  })

  it('exposes optional recipe provenance separately from cover provenance in both locales', () => {
    function Harness() {
      const form = useForm<RecipeForm>({
        defaultValues: {
          title: '',
          description: '',
          instructions: '',
          visibility: 'private',
          difficulty: null,
          estimatedCalories: null,
          sourceUrl: '',
          sourceTitle: '',
          sourceAuthor: '',
          sourceLicense: '',
          sourceAttribution: '',
          ingredientGroups: [{
            id: 'group-1',
            optional: false,
            note: '',
            alternatives: [{ id: 'alternative-1', name: 'water', quantityNote: '', canonicalIngredientId: null }],
          }],
        },
      })
      return <RecipeProvenanceFields form={form} />
    }
    const { unmount } = render(<Providers><Harness /></Providers>)
    fireEvent.click(screen.getByText('Recipe source details (optional)'))
    expect(screen.getByLabelText('Recipe source URL')).toHaveAttribute('type', 'url')
    expect(screen.getByText(/not independent proof/)).toBeVisible()
    unmount()
    render(<Providers locale="zh-CN"><Harness /></Providers>)
    fireEvent.click(screen.getByText('菜谱来源信息（可选）'))
    expect(screen.getByLabelText('来源许可证')).toBeVisible()
    expect(screen.getByText(/不是对许可证适用性的独立证明/)).toBeVisible()
  })

  it('renders shopping controls in Simplified Chinese', async () => {
    render(
      <Providers locale="zh-CN">
        <ShoppingListPage />
      </Providers>,
    )
    expect(screen.getByRole('heading', { name: '购物清单' })).toBeVisible()
    expect(await screen.findByRole('button', { name: 'Milk 已买到' })).toBeVisible()
    expect(screen.getByRole('button', { name: '从购物清单删除 Milk' })).toBeVisible()
    expect(screen.getByText('这里只显示仍需购买的食材。买到后可直接更新库存。')).toBeVisible()
  })
})

describe('responsive primary navigation', () => {
  function renderNavigation(locale: Locale = 'en', initialPath = '/pantry') {
    storedLocale.value = null
    Object.defineProperty(window, 'scrollTo', {
      configurable: true,
      value: vi.fn(),
    })
    const root = createRootRoute({
      component: () => (
        <Providers locale={locale}>
          <PrimaryNavigation />
        </Providers>
      ),
    })
    const routes = [
      '/recommendations',
      '/recipes',
      '/pantry',
      '/shopping-list',
      '/imports',
      '/settings',
    ].map((path) => createRoute({
      getParentRoute: () => root,
      path,
      component: () => null,
    }))
    const router = createRouter({
      routeTree: root.addChildren(routes),
      history: createMemoryHistory({ initialEntries: [initialPath] }),
    })
    render(<RouterProvider router={router} />)
    return router
  }

  it('uses a semantic active state and exposes every destination in the mobile menu', async () => {
    renderNavigation()
    const activeDesktop = await screen.findByRole('link', { name: 'Pantry' })
    expect(activeDesktop).toHaveAttribute('aria-current', 'page')
    const trigger = screen.getByRole('button', { name: 'Menu' })
    fireEvent.click(trigger)
    expect(trigger).toHaveAttribute('aria-expanded', 'true')
    const dialog = screen.getByRole('dialog', { name: 'Menu' })
    for (const label of ['Recommend', 'Recipes', 'Pantry', 'Shopping list', 'Imports', 'Settings']) {
      expect(within(dialog).getByRole('link', { name: label })).toBeVisible()
    }
    expect(within(dialog).getByRole('link', { name: 'Pantry' })).toHaveAttribute('aria-current', 'page')
    fireEvent.keyDown(document, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Menu' })).not.toBeInTheDocument())
    expect(trigger).toHaveFocus()
  })

  it('closes after navigation and renders Chinese labels', async () => {
    const router = renderNavigation('zh-CN')
    fireEvent.click(await screen.findByRole('button', { name: '菜单' }))
    const dialog = screen.getByRole('dialog', { name: '菜单' })
    fireEvent.click(within(dialog).getByRole('link', { name: '购物清单' }))
    await waitFor(() => expect(router.state.location.pathname).toBe('/shopping-list'))
    expect(screen.queryByRole('dialog', { name: '菜单' })).not.toBeInTheDocument()
  })
})

describe('recommendation shopping membership', () => {
  function renderRecommendations(locale: Locale = 'en', path = '/recommendations') {
    window.history.replaceState({}, '', path)
    Object.defineProperty(window, 'scrollTo', {
      configurable: true,
      value: vi.fn(),
    })
    const root = createRootRoute({
      component: () => (
        <LocaleProvider initialLocale={locale} storage={null}>
          <QueryClientProvider client={new QueryClient()}>
            <RecommendationsPage />
          </QueryClientProvider>
        </LocaleProvider>
      ),
    })
    const testRouter = createRouter({
      routeTree: root,
      history: createMemoryHistory({ initialEntries: ['/'] }),
    })
    return render(<RouterProvider router={testRouter} />)
  }

  function recommendation(inShoppingList: string[] = []): Recommendation {
    return {
      recipe: {
        id: 'recipe-shop',
        version: 1,
        isOwner: true,
        authorHandle: '',
        authorNickname: null,
        title: 'Saffron rice',
        description: '',
        instructions: '',
        visibility: 'private' as const,
        publicationIntent: 'private' as const,
        moderationState: 'clear' as const,
        moderationReviewState: 'unreviewed' as const,
        moderationVersion: 1,
        difficulty: null,
        estimatedCalories: null,
        ingredients: [{
          name: 'Saffron',
          quantityNote: '',
          canonicalIngredientId: 'canonical-saffron',
        }],
        isFavorite: false,
        isFavoriteAuthor: false,
        coverImage: null,
        source: null,
      },
      status: 'SHOP' as const,
      available: [],
      verify: [],
      missing: ['Saffron'],
      inShoppingList,
      reasons: [],
      reasonDetails: [{ code: 'confirmedIngredients' as const, confirmed: 0, total: 1 }],
      score: 250,
      scoreBreakdown: {
        statusBase: 250,
        source: 100,
        availability: 0,
        desired: 0,
        certainty: 0,
        simplicity: 8,
      },
      primarySource: 'OWN' as const,
      matchedSources: ['OWN' as const],
      desiredMatches: [],
      ingredientReasons: [{
        ingredient: 'Saffron',
        code: 'MISSING' as const,
        expiresOn: null,
      }],
      appliedControls: {
        preferred: [],
        required: [],
        excluded: [],
        pantryOnly: false,
        includeOwn: true,
        includeFavoriteRecipes: true,
        includeFavoriteAccounts: true,
        includeOtherPublic: true,
      },
    }
  }

  it('loads default controls once, writes a versioned URL, and includes visible source controls', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    renderRecommendations()

    expect(await screen.findByText('Saffron rice')).toBeVisible()
    expect(recommendationsApi).toHaveBeenCalledTimes(1)
    expect(recommendationsApi).toHaveBeenCalledWith(
      expect.objectContaining({
        ingredientScope: 'ANY',
        includeFavoriteRecipes: true,
        includeOwn: true,
        includeFavoriteAccounts: true,
        includeOtherPublic: true,
      }),
      expect.any(AbortSignal),
    )
    expect(window.location.search).toContain('rv=1')
    expect(screen.getByText(/shared link includes the ingredient filter words/i)).toBeVisible()
  })

  it('restores a deep link without losing canonical versus unresolved identity', async () => {
    const defaults = recommendationControlsFromSettings({
      includeFavoriteRecipes: true,
      includeOwn: true,
      includeFavoriteAccounts: true,
      includeOtherPublic: true,
      verificationDays: 14,
    })
    const query = encodeRecommendationControls({
      ...defaults,
      preferred: [
        { name: 'Spring onion', canonicalIngredientId: null },
        { name: 'Spring onion', canonicalIngredientId: 'spring-onion-id' },
      ],
      ingredientScope: 'PANTRY_ONLY',
    })
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])

    renderRecommendations('en', `/recommendations?${query}`)

    await screen.findByText('Saffron rice')
    const request = recommendationsApi.mock.calls[0][0]
    expect(request.preferred).toEqual([
      { name: 'Spring onion', canonicalIngredientId: 'spring-onion-id' },
      { name: 'Spring onion', canonicalIngredientId: null },
    ])
    expect(request.ingredientScope).toBe('PANTRY_ONLY')
    expect(screen.getAllByText('Spring onion')).toHaveLength(2)
  })

  it('downgrades only an unavailable URL-origin canonical identity and retries by name', async () => {
    const defaults = recommendationControlsFromSettings({
      includeFavoriteRecipes: true,
      includeOwn: true,
      includeFavoriteAccounts: true,
      includeOtherPublic: true,
      verificationDays: 14,
    })
    const query = encodeRecommendationControls({
      ...defaults,
      preferred: [{ name: 'Private spice', canonicalIngredientId: 'private-spice-id' }],
    })
    recommendationsApi.mockReset()
      .mockRejectedValueOnce(new ApiError(
        'selected canonical ingredient is not available to this account',
        400,
        'canonical_ingredient_unavailable',
        ['private-spice-id'],
      ))
      .mockResolvedValueOnce([recommendation()])

    renderRecommendations('en', `/recommendations?${query}`)

    expect(await screen.findByText('Saffron rice')).toBeVisible()
    expect(recommendationsApi).toHaveBeenCalledTimes(2)
    expect(recommendationsApi.mock.calls[0][0].preferred).toEqual([
      { name: 'Private spice', canonicalIngredientId: 'private-spice-id' },
    ])
    expect(recommendationsApi.mock.calls[1][0].preferred).toEqual([
      { name: 'Private spice', canonicalIngredientId: null },
    ])
    expect(screen.getByText(/canonical ingredient in this shared link is not available/i)).toBeVisible()
    expect(parseRecommendationControls(window.location.search, defaults).controls.preferred).toEqual([
      { name: 'Private spice', canonicalIngredientId: null },
    ])
  })

  it('preserves edits made during debounce when an older canonical request fails', async () => {
    const defaults = recommendationControlsFromSettings({
      includeFavoriteRecipes: true,
      includeOwn: true,
      includeFavoriteAccounts: true,
      includeOtherPublic: true,
      verificationDays: 14,
    })
    const query = encodeRecommendationControls({
      ...defaults,
      preferred: [{ name: 'Private spice', canonicalIngredientId: 'private-spice-id' }],
    })
    let rejectFirst!: (error: Error) => void
    recommendationsApi.mockReset()
      .mockReturnValueOnce(new Promise((_, reject) => { rejectFirst = reject }))
      .mockRejectedValueOnce(new ApiError(
        'selected canonical ingredient is not available to this account',
        400,
        'canonical_ingredient_unavailable',
        ['private-spice-id'],
      ))
      .mockResolvedValueOnce([recommendation()])
    renderRecommendations('en', `/recommendations?${query}`)
    await waitFor(() => expect(recommendationsApi).toHaveBeenCalledTimes(1))

    const input = screen.getByRole('combobox', { name: 'Add ingredient to Hope to use' })
    fireEvent.change(input, { target: { value: 'Pepper' } })
    fireEvent.keyDown(input, { key: 'Enter' })
    rejectFirst(new ApiError(
      'selected canonical ingredient is not available to this account',
      400,
      'canonical_ingredient_unavailable',
      ['private-spice-id'],
    ))

    expect(await screen.findByText('Saffron rice')).toBeVisible()
    expect(recommendationsApi).toHaveBeenCalledTimes(3)
    expect(recommendationsApi.mock.calls[2][0].preferred).toEqual(expect.arrayContaining([
      { name: 'Pepper', canonicalIngredientId: null },
      { name: 'Private spice', canonicalIngredientId: null },
    ]))
    expect(recommendationsApi.mock.calls[2][0].preferred).toHaveLength(2)
    expect(parseRecommendationControls(window.location.search, defaults).controls.preferred)
      .toEqual(expect.arrayContaining([
        { name: 'Pepper', canonicalIngredientId: null },
        { name: 'Private spice', canonicalIngredientId: null },
      ]))
  })

  it('fails closed when URL canonical downgrade would merge hard-filter identities', async () => {
    const defaults = recommendationControlsFromSettings({
      includeFavoriteRecipes: true,
      includeOwn: true,
      includeFavoriteAccounts: true,
      includeOtherPublic: true,
      verificationDays: 14,
    })
    const original = {
      ...defaults,
      required: [{ name: 'Rice', canonicalIngredientId: 'private-rice-id' }],
      excluded: [{ name: 'Rice', canonicalIngredientId: null }],
    }
    recommendationsApi.mockReset().mockRejectedValue(new ApiError(
      'selected canonical ingredient is not available to this account',
      400,
      'canonical_ingredient_unavailable',
      ['private-rice-id'],
    ))
    renderRecommendations('en', `/recommendations?${encodeRecommendationControls(original)}`)

    expect(await screen.findByText(/cannot safely downgrade this shared link/i)).toBeVisible()
    expect(await screen.findByRole('button', { name: 'Retry recommendations' })).toBeVisible()
    expect(recommendationsApi).toHaveBeenCalledTimes(1)
    const current = parseRecommendationControls(window.location.search, defaults).controls
    expect(current.required).toEqual(original.required)
    expect(current.excluded).toEqual(original.excluded)
  })

  it('keeps an explicitly selected unavailable canonical identity fail-closed', async () => {
    ingredientSuggestionsApi.mockResolvedValue([{
      canonicalIngredientId: 'private-spice-id',
      displayName: 'Private spice',
      matchedText: 'Private spice',
      reason: 'exact',
    }])
    recommendationsApi.mockReset()
      .mockResolvedValueOnce([recommendation()])
      .mockRejectedValueOnce(new ApiError(
        'selected canonical ingredient is not available to this account',
        400,
        'canonical_ingredient_unavailable',
        ['private-spice-id'],
      ))
    renderRecommendations()
    await screen.findByText('Saffron rice')
    const input = screen.getByRole('combobox', { name: 'Add ingredient to Hope to use' })
    fireEvent.change(input, { target: { value: 'Private spice' } })
    fireEvent.focus(input)
    fireEvent.click(await screen.findByRole('option', {
      name: 'Private spice, matched existing ingredient',
    }))

    expect(await screen.findByRole('button', { name: 'Retry recommendations' })).toBeVisible()
    expect(recommendationsApi).toHaveBeenCalledTimes(2)
    expect(parseRecommendationControls(window.location.search, recommendationControlsFromSettings({
      includeFavoriteRecipes: true,
      includeOwn: true,
      includeFavoriteAccounts: true,
      includeOtherPublic: true,
      verificationDays: 14,
    })).controls.preferred).toEqual([
      { name: 'Private spice', canonicalIngredientId: 'private-spice-id' },
    ])
  })

  it('does not refresh for uncommitted combobox typing and debounces committed filters', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    renderRecommendations()
    await screen.findByText('Saffron rice')
    const input = screen.getByRole('combobox', { name: 'Add ingredient to Hope to use' })

    fireEvent.change(input, { target: { value: 'Rice' } })
    await new Promise((resolve) => window.setTimeout(resolve, 350))
    expect(recommendationsApi).toHaveBeenCalledTimes(1)

    fireEvent.keyDown(input, { key: 'Enter' })
    await waitFor(() => expect(recommendationsApi).toHaveBeenCalledTimes(2))
    expect(recommendationsApi.mock.calls[1][0].preferred).toEqual([
      { name: 'Rice', canonicalIngredientId: null },
    ])
  })

  it('restarts pending A after A to B to A within debounce without changing history', async () => {
    let resolveFirst!: (value: Recommendation[]) => void
    let resolveSecond!: (value: Recommendation[]) => void
    recommendationsApi.mockReset()
      .mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve }))
      .mockReturnValueOnce(new Promise((resolve) => { resolveSecond = resolve }))
    renderRecommendations()
    await waitFor(() => expect(recommendationsApi).toHaveBeenCalledTimes(1))
    const initialUrl = window.location.href
    const initialHistoryLength = window.history.length

    fireEvent.click(screen.getByRole('radio', { name: /Pantry only/ }))
    fireEvent.click(screen.getByRole('radio', { name: /Any ingredients/ }))
    await waitFor(() => expect(recommendationsApi).toHaveBeenCalledTimes(2))
    expect(window.location.href).toBe(initialUrl)
    expect(window.history.length).toBe(initialHistoryLength)

    const restarted = recommendation()
    restarted.recipe = { ...restarted.recipe, title: 'Restarted A result' }
    resolveSecond([restarted])
    expect(await screen.findByText('Restarted A result')).toBeVisible()

    const stale = recommendation()
    stale.recipe = { ...stale.recipe, title: 'Aborted first A result' }
    resolveFirst([stale])
    await new Promise((resolve) => window.setTimeout(resolve, 0))
    expect(screen.queryByText('Aborted first A result')).not.toBeInTheDocument()
  })

  it('restores successful A after A to B to A within debounce without another request', async () => {
    const cached = recommendation()
    cached.recipe = { ...cached.recipe, title: 'Cached A result' }
    recommendationsApi.mockReset().mockResolvedValue([cached])
    renderRecommendations()
    expect(await screen.findByText('Cached A result')).toBeVisible()
    const initialUrl = window.location.href
    const initialHistoryLength = window.history.length

    fireEvent.click(screen.getByRole('radio', { name: /Pantry only/ }))
    fireEvent.click(screen.getByRole('radio', { name: /Any ingredients/ }))
    await new Promise((resolve) => window.setTimeout(resolve, 350))

    expect(recommendationsApi).toHaveBeenCalledTimes(1)
    expect(screen.getByText('Cached A result')).toBeVisible()
    expect(screen.queryByText('Updating recommendations…')).not.toBeInTheDocument()
    expect(window.location.href).toBe(initialUrl)
    expect(window.history.length).toBe(initialHistoryLength)
  })

  it('ignores a late response after filters change while a request is pending', async () => {
    let resolveFirst!: (value: Recommendation[]) => void
    let resolveSecond!: (value: Recommendation[]) => void
    recommendationsApi.mockReset()
      .mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve }))
      .mockReturnValueOnce(new Promise((resolve) => { resolveSecond = resolve }))
    renderRecommendations()
    await waitFor(() => expect(recommendationsApi).toHaveBeenCalledTimes(1))

    fireEvent.click(screen.getByRole('radio', { name: /Pantry only/ }))
    await waitFor(() => expect(recommendationsApi).toHaveBeenCalledTimes(2))
    const newest = recommendation()
    newest.recipe = { ...newest.recipe, title: 'Newest filters result' }
    resolveSecond([newest])
    expect(await screen.findByText('Newest filters result')).toBeVisible()

    const stale = recommendation()
    stale.recipe = { ...stale.recipe, title: 'Late stale result' }
    resolveFirst([stale])
    await new Promise((resolve) => window.setTimeout(resolve, 0))
    expect(screen.queryByText('Late stale result')).not.toBeInTheDocument()
    expect(screen.getByText('Newest filters result')).toBeVisible()
  })

  it('keeps the last success on failure and retries current filters', async () => {
    const recovered = recommendation()
    recovered.recipe = { ...recovered.recipe, title: 'Recovered result' }
    recommendationsApi.mockReset()
      .mockResolvedValueOnce([recommendation()])
      .mockRejectedValueOnce(new Error('network failed'))
      .mockResolvedValueOnce([recovered])
    renderRecommendations()
    expect(await screen.findByText('Saffron rice')).toBeVisible()

    fireEvent.click(screen.getByRole('checkbox', { name: /Other users’ public recipes/ }))
    expect(await screen.findByRole('button', { name: 'Retry recommendations' })).toBeVisible()
    expect(screen.getByText('Saffron rice')).toBeVisible()
    expect(window.location.search).toContain('rv=1')

    fireEvent.click(screen.getByRole('button', { name: 'Retry recommendations' }))
    expect(await screen.findByText('Recovered result')).toBeVisible()
    expect(recommendationsApi.mock.calls[2][0].includeOtherPublic).toBe(false)
  })

  it('restores controls and requests on popstate without creating a state loop', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    renderRecommendations()
    await screen.findByText('Saffron rice')

    const defaults = recommendationControlsFromSettings({
      includeFavoriteRecipes: true,
      includeOwn: true,
      includeFavoriteAccounts: true,
      includeOtherPublic: true,
      verificationDays: 14,
    })
    const query = encodeRecommendationControls({ ...defaults, ingredientScope: 'PANTRY_OR_LIST' })
    window.history.replaceState({}, '', `/recommendations?${query}`)
    window.dispatchEvent(new PopStateEvent('popstate'))

    await waitFor(() => expect(screen.getByRole('radio', { name: /Pantry or shopping list/ })).toBeChecked())
    await waitFor(() => expect(recommendationsApi).toHaveBeenCalledTimes(2))
    expect(recommendationsApi.mock.calls[1][0].ingredientScope).toBe('PANTRY_OR_LIST')
  })

  it('invalidates an older request synchronously when popstate restores another URL', async () => {
    let resolveOld!: (value: Recommendation[]) => void
    const newest = recommendation()
    newest.recipe = { ...newest.recipe, title: 'Popstate result' }
    recommendationsApi.mockReset()
      .mockReturnValueOnce(new Promise((resolve) => { resolveOld = resolve }))
      .mockResolvedValueOnce([newest])
    renderRecommendations()
    await waitFor(() => expect(recommendationsApi).toHaveBeenCalledTimes(1))

    const defaults = recommendationControlsFromSettings({
      includeFavoriteRecipes: true,
      includeOwn: true,
      includeFavoriteAccounts: true,
      includeOtherPublic: true,
      verificationDays: 14,
    })
    window.history.replaceState({}, '', `/recommendations?${encodeRecommendationControls({
      ...defaults,
      ingredientScope: 'PANTRY_ONLY',
    })}`)
    window.dispatchEvent(new PopStateEvent('popstate'))
    expect(await screen.findByText('Popstate result')).toBeVisible()

    const stale = recommendation()
    stale.recipe = { ...stale.recipe, title: 'Old URL result' }
    resolveOld([stale])
    await new Promise((resolve) => window.setTimeout(resolve, 0))
    expect(screen.queryByText('Old URL result')).not.toBeInTheDocument()
  })

  it('fails safe for an unknown URL version and announces the non-blocking warning', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    renderRecommendations('en', '/recommendations?rv=99&p=bad&token=private')

    expect(await screen.findByText(/invalid or unsupported/i)).toBeVisible()
    expect(await screen.findByText('Saffron rice')).toBeVisible()
    expect(recommendationsApi.mock.calls[0][0].preferred).toEqual([])
    expect(window.location.search).not.toContain('token')
  })

  it('renders semantic membership instead of an add action when already active', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation(['Saffron'])])
    renderRecommendations()
    expect(await screen.findByText('Saffron: In shopping list')).toBeVisible()
    expect(screen.queryByRole('button', { name: 'Add Saffron to shopping list' })).not.toBeInTheDocument()
  })

  it('sends only the selected ingredient scope and exposes all radio choices accessibly', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    renderRecommendations()
    const any = await screen.findByRole('radio', { name: /Any ingredients/ })
    const pantry = screen.getByRole('radio', { name: /Pantry only/ })
    const pantryOrList = screen.getByRole('radio', { name: /Pantry or shopping list/ })
    expect(any).toBeChecked()
    expect(pantry).not.toBeChecked()
    expect(pantryOrList).not.toBeChecked()
    expect(any.closest('label')).toHaveClass('choice-card')
    pantryOrList.focus()
    expect(pantryOrList).toHaveFocus()
    fireEvent.click(pantryOrList)
    await waitFor(() => {
      const request = recommendationsApi.mock.calls.at(-1)![0]
      expect(request).toEqual(expect.objectContaining({ ingredientScope: 'PANTRY_OR_LIST' }))
      expect(request).not.toHaveProperty('pantryOnly')
      expect(window.location.search).toContain('scope=pantry-list')
    })
  })

  it('preserves backend intent-v2 order while partitioning desired results above readiness', async () => {
    const ready = recommendation()
    ready.recipe = { ...ready.recipe, id: 'ready', title: 'Ready plain meal' }
    ready.status = 'READY'
    ready.desiredMatches = []
    ready.score = 999
    ready.appliedControls.preferred = ['Rice', 'Egg']

    const full = recommendation()
    full.recipe = { ...full.recipe, id: 'desired-full', title: 'Desired shopping meal' }
    full.desiredMatches = ['Rice', 'Egg']
    full.rankingIntent = { version: 'intent-v2', desiredMatched: 2, desiredTotal: 2 }
    full.score = 999
    full.appliedControls.preferred = ['Rice', 'Egg']

    const partial = recommendation()
    partial.recipe = { ...partial.recipe, id: 'desired-partial', title: 'Partial preferred meal' }
    partial.desiredMatches = ['Rice']
    partial.rankingIntent = { version: 'intent-v2', desiredMatched: 1, desiredTotal: 2 }
    partial.score = 999
    partial.appliedControls.preferred = ['Rice', 'Egg']

    recommendationsApi.mockReset().mockResolvedValue([ready, full, partial])
    renderRecommendations()
    const preferredHeading = await screen.findByRole('heading', { name: 'Prioritizes ingredients you hope to use' })
    const otherHeading = screen.getByRole('heading', { name: 'Other recommendations' })
    expect(preferredHeading.compareDocumentPosition(otherHeading) & Node.DOCUMENT_POSITION_FOLLOWING)
      .toBeTruthy()
    expect(screen.getByText('2/2 preferred ingredients')).toBeVisible()
    expect(screen.getByText('1/2 preferred ingredients')).toBeVisible()
    const desiredCard = screen.getByRole('link', { name: 'Desired shopping meal' }).closest('article')
    const readyCard = screen.getByRole('link', { name: 'Ready plain meal' }).closest('article')
    expect(desiredCard).not.toBeNull()
    expect(readyCard).not.toBeNull()
    expect(desiredCard!.compareDocumentPosition(readyCard!) & Node.DOCUMENT_POSITION_FOLLOWING)
      .toBeTruthy()
    fireEvent.click(within(desiredCard as HTMLElement).getByText('Why recommended'))
    expect(within(desiredCard as HTMLElement).getByText('Uses 2 of 2 preferred ingredients.')).toBeVisible()
    expect(within(desiredCard as HTMLElement).queryByText(/Total 999/)).not.toBeInTheDocument()
    expect(screen.queryByText(/Total 999/)).not.toBeInTheDocument()
  })

  it('keeps no-desired results in one backend-ordered section without status or score sorting', async () => {
    const shop = recommendation()
    shop.recipe = { ...shop.recipe, id: 'shop-first', title: 'Shop first from backend' }
    const ready = recommendation()
    ready.recipe = { ...ready.recipe, id: 'ready-second', title: 'Ready second from backend' }
    ready.status = 'READY'
    recommendationsApi.mockReset().mockResolvedValue([shop, ready])
    renderRecommendations()
    expect(await screen.findByRole('heading', { name: 'Recommendations' })).toBeVisible()
    expect(screen.queryByRole('heading', { name: 'Prioritizes ingredients you hope to use' })).not.toBeInTheDocument()
    const shopLink = screen.getByRole('link', { name: 'Shop first from backend' })
    const readyLink = screen.getByRole('link', { name: 'Ready second from backend' })
    expect(shopLink.compareDocumentPosition(readyLink) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })

  it('localizes ingredient scope choices at narrow viewport width', async () => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 320 })
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    renderRecommendations('zh-CN')
    expect(await screen.findByRole('radio', { name: /不限食材来源/ })).toBeChecked()
    expect(screen.getByRole('radio', { name: /仅限库存/ })).toBeVisible()
    expect(screen.getByRole('radio', { name: /库存或购物清单/ })).toBeVisible()
    expect(screen.getByRole('group', { name: '食材范围' })).toBeVisible()
    expect(screen.getByRole('group', { name: '候选菜谱来源' })).toBeVisible()
    expect(screen.getByText(/分享链接会包含食材筛选词/)).toBeVisible()
  })

  it('uses compact preview and ingredient groups with details collapsed by default', async () => {
    const result = recommendation(['Saffron'])
    result.recipe.description = 'A **compact** bowl. <script>unsafe()</script>'
    result.recipe.coverImage = {
      id: 'cover-1',
      url: '/api/recipes/recipe-shop/cover',
      altText: 'Saffron rice in a bowl',
      sourceUrl: null,
      sourceLicense: null,
      sourceAttribution: null,
      version: 1,
      width: 640,
      height: 480,
    }
    result.recipe.ingredients = [
      { name: 'Rice', quantityNote: '', canonicalIngredientId: 'rice' },
      { name: 'Egg', quantityNote: '', canonicalIngredientId: 'egg' },
      { name: 'Saffron', quantityNote: '', canonicalIngredientId: 'canonical-saffron' },
    ]
    result.available = ['Rice']
    result.verify = ['Egg']
    result.reasonDetails = [{ code: 'confirmedIngredients', confirmed: 1, total: 3 }]
    result.ingredientReasons = [
      { ingredient: 'Rice', code: 'AVAILABLE', expiresOn: null },
      { ingredient: 'Egg', code: 'UNKNOWN', expiresOn: null },
      { ingredient: 'Saffron', code: 'MISSING', expiresOn: null },
    ]
    recommendationsApi.mockReset().mockResolvedValue([result])
    renderRecommendations()
    expect(await screen.findByRole('link', { name: 'Saffron rice' })).toHaveAttribute(
      'href',
      '/recipes/recipe-shop',
    )
    expect(screen.queryByRole('link', { name: 'Details' })).not.toBeInTheDocument()
    expect(await screen.findByRole('img', { name: 'Saffron rice in a bowl' })).toHaveAttribute(
      'src',
      '/api/recipes/recipe-shop/cover',
    )
    expect(screen.getByText('compact')).toBeVisible()
    expect(document.querySelector('script')).toBeNull()
    expect(screen.getByText('Primary source: My recipe')).toBeVisible()
    expect(screen.getByText('Available')).toBeVisible()
    expect(screen.getByText('Verify')).toBeVisible()
    expect(screen.getByText('Missing')).toBeVisible()
    expect(screen.getByText('Saffron: In shopping list')).toBeVisible()
    const detailReason = screen.getByText('1 of 3 ingredients confirmed')
    expect(detailReason).not.toBeVisible()
    expect(screen.getByText('Rice: confirmed available')).not.toBeVisible()
    fireEvent.click(screen.getByText('Why recommended'))
    expect(detailReason).toBeVisible()
    expect(screen.getByText('Rice: confirmed available')).toBeVisible()
  })

  it('omits empty ingredient groups and preview placeholders', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    renderRecommendations()
    await screen.findByText('Saffron rice')
    expect(screen.queryByText('Available')).not.toBeInTheDocument()
    expect(screen.queryByText('Verify')).not.toBeInTheDocument()
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
  })

  it('uses Cook user terminology for account ownership while preserving source-author terminology', async () => {
    const result = recommendation()
    result.primarySource = 'FAVORITE_ACCOUNT'
    result.matchedSources = ['FAVORITE_ACCOUNT']
    recommendationsApi.mockReset().mockResolvedValue([result])
    renderRecommendations('zh-CN')
    expect(await screen.findByText('主要来源：已收藏用户')).toBeVisible()
    expect(screen.queryByText('主要来源：已收藏作者')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '将 Saffron 加入购物清单' })).toHaveTextContent('加入购物清单')
  })

  it('prevents duplicate submits and transitions immediately after a successful add', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    let resolveAdd!: (value: never[]) => void
    addRecipeIngredientToShopping.mockReset().mockReturnValue(
      new Promise<never[]>((resolve) => {
        resolveAdd = resolve
      }),
    )
    renderRecommendations()
    const add = await screen.findByRole('button', { name: 'Add Saffron to shopping list' })
    fireEvent.click(add)
    fireEvent.click(add)
    await waitFor(() => expect(addRecipeIngredientToShopping).toHaveBeenCalledTimes(1))
    expect(screen.getByRole('button', { name: 'Adding Saffron to shopping list…' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Adding Saffron to shopping list…' }))
      .toHaveTextContent('Adding to shopping list…')
    resolveAdd([])
    expect(await screen.findByText('Saffron: In shopping list')).toBeVisible()
    expect(screen.getByRole('status')).toHaveTextContent('Saffron added to the shopping list.')
  })

  it('renders one structured group row and lets the user choose the concrete shopping alternative', async () => {
    const result = recommendation()
    result.recipe.ingredientModel = 'groups-v1'
    result.recipe.ingredientGroups = [{
      id: 'group-fat',
      optional: false,
      note: 'Use either',
      sortOrder: 0,
      alternatives: [
        {
          id: 'butter',
          name: 'Butter',
          quantityNote: '1 tbsp',
          canonicalIngredientId: 'butter-id',
          sortOrder: 0,
        },
        {
          id: 'oil',
          name: 'Oil',
          quantityNote: '1 tbsp',
          canonicalIngredientId: 'oil-id',
          sortOrder: 1,
        },
      ],
    }]
    result.missing = ['Butter', 'Oil']
    result.ingredientGroupReasons = [{
      groupId: 'group-fat',
      label: 'Butter / Oil',
      optional: false,
      effective: true,
      code: 'MISSING',
      selectedAlternativeId: null,
      alternatives: [
        { ingredient: 'Butter', code: 'MISSING', expiresOn: null },
        { ingredient: 'Oil', code: 'MISSING', expiresOn: null },
      ],
    }]
    recommendationsApi.mockReset().mockResolvedValue([result])
    addRecipeIngredientToShopping.mockReset().mockResolvedValue([])
    renderRecommendations()
    expect(await screen.findAllByText('Butter or Oil')).toHaveLength(1)
    fireEvent.click(screen.getByText('Why recommended'))
    expect(screen.getByText('0 of 1 ingredient groups are covered by pantry or shopping-list availability.')).toBeVisible()
    fireEvent.change(screen.getByLabelText('Choose an alternative for Butter or Oil'), {
      target: { value: 'oil' },
    })
    fireEvent.click(screen.getByRole('button', { name: 'Add Oil to shopping list' }))
    await waitFor(() => expect(addRecipeIngredientToShopping).toHaveBeenCalledWith(
      'recipe-shop',
      {
        name: 'Oil',
        canonicalIngredientId: 'oil-id',
        groupId: 'group-fat',
        alternativeId: 'oil',
      },
    ))
    expect(await screen.findByText('Butter or Oil: an alternative is in the shopping list')).toBeVisible()
    expect(screen.getByText('1 of 1 ingredient groups are covered by pantry or shopping-list availability.')).toBeVisible()
  })

  it('restores the add action after failure and never claims success', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    addRecipeIngredientToShopping.mockReset().mockRejectedValue(new Error('network failed'))
    renderRecommendations()
    fireEvent.click(await screen.findByRole('button', { name: 'Add Saffron to shopping list' }))
    expect(await screen.findByText('network failed')).toBeVisible()
    expect(screen.getByRole('button', { name: 'Add Saffron to shopping list' })).toBeEnabled()
    expect(screen.queryByText('Saffron: In shopping list')).not.toBeInTheDocument()
    expect(screen.queryByText('Saffron added to the shopping list.')).not.toBeInTheDocument()
  })
})

describe('ingredient groups and homepage navigation', () => {
  it('projects alternatives as one group with localized OR, optional note, and one pantry badge', async () => {
    render(
      <Providers>
        <RecipeIngredientList
          ingredients={[]}
          ingredientModel="groups-v1"
          ingredientGroups={[{
            id: 'group-dairy',
            optional: true,
            note: 'Use what is open',
            sortOrder: 0,
            alternatives: [
              {
                id: 'alt-milk',
                name: 'Milk',
                quantityNote: '1 cup',
                canonicalIngredientId: null,
                sortOrder: 0,
              },
              {
                id: 'alt-oat',
                name: 'Oat milk',
                quantityNote: '1 cup',
                canonicalIngredientId: null,
                sortOrder: 1,
              },
            ],
          }]}
        />
      </Providers>,
    )
    expect(await screen.findByText('or')).toBeVisible()
    expect(screen.getByText('Optional')).toBeVisible()
    expect(screen.getByText('Use what is open')).toBeVisible()
    expect(screen.getAllByLabelText(/pantry status/)).toHaveLength(1)
  })

  it('adds AND groups and OR alternatives and rejects duplicate ingredient identities', () => {
    render(
      <Providers>
        <RecipeEditorForm onSaved={vi.fn()} />
      </Providers>,
    )
    expect(screen.getByRole('group', { name: 'Ingredient group 1' })).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Add alternative' }))
    expect(screen.getByLabelText('Group 1, alternative 2')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Add ingredient' }))
    expect(screen.getByRole('group', { name: 'Ingredient group 2' })).toBeVisible()

    const result = recipeSchema((key) => key).safeParse({
      title: 'Duplicate',
      description: '',
      instructions: '',
      visibility: 'private',
      difficulty: null,
      estimatedCalories: null,
      sourceUrl: '',
      sourceTitle: '',
      sourceAuthor: '',
      sourceLicense: '',
      sourceAttribution: '',
      ingredientGroups: [
        {
          id: 'g1',
          optional: false,
          note: '',
          alternatives: [
            { id: 'a1', name: ' Milk ', quantityNote: '', canonicalIngredientId: null },
            { id: 'a2', name: 'milk', quantityNote: '', canonicalIngredientId: null },
          ],
        },
      ],
    })
    expect(result.success).toBe(false)
  })

  it('makes each homepage metric a whole-card link with owner-only recipe count and filters', async () => {
    recipesApi.mockReset().mockResolvedValue([
      {
        id: 'owned', version: 1, isOwner: true, authorHandle: '', authorNickname: null,
        title: 'Mine', description: '', instructions: '', visibility: 'private',
        difficulty: null, estimatedCalories: null, ingredients: [], isFavorite: false,
        isFavoriteAuthor: false, coverImage: null, source: null,
      },
      {
        id: 'visible', version: 1, isOwner: false, authorHandle: '', authorNickname: null,
        title: 'Shared', description: '', instructions: '', visibility: 'public',
        difficulty: null, estimatedCalories: null, ingredients: [], isFavorite: false,
        isFavoriteAuthor: false, coverImage: null, source: null,
      },
    ])
    const root = createRootRoute()
    const home = createRoute({ getParentRoute: () => root, path: '/', component: HomePage })
    const recipes = createRoute({ getParentRoute: () => root, path: '/recipes', component: () => null })
    const pantry = createRoute({ getParentRoute: () => root, path: '/pantry', component: () => null })
    const recommendations = createRoute({ getParentRoute: () => root, path: '/recommendations', component: () => null })
    const newRecipe = createRoute({ getParentRoute: () => root, path: '/recipes/new', component: () => null })
    const testRouter = createRouter({
      routeTree: root.addChildren([home, recipes, pantry, recommendations, newRecipe]),
      history: createMemoryHistory({ initialEntries: ['/'] }),
    })

    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <RouterProvider router={testRouter} />
        </QueryClientProvider>
      </LocaleProvider>,
    )
    expect(await screen.findByRole('link', { name: 'My recipes: 1' }))
      .toHaveAttribute('href', '/recipes?scope=mine')
    expect(screen.getByRole('link', { name: 'Pantry items: 1' })).toHaveAttribute('href', '/pantry')
    expect(screen.getByRole('link', { name: 'Confirmed available: 0' }))
      .toHaveAttribute('href', '/pantry?status=have')
  })

  it('applies and clears the owner-only recipes filter', async () => {
    recipesApi.mockReset().mockResolvedValue([
      {
        id: 'owned', version: 1, isOwner: true, authorHandle: '', authorNickname: null,
        title: 'Owned recipe', description: '', instructions: '', visibility: 'private',
        difficulty: null, estimatedCalories: null, ingredients: [], isFavorite: false,
        isFavoriteAuthor: false, coverImage: null, source: null,
      },
      {
        id: 'shared', version: 1, isOwner: false, authorHandle: '', authorNickname: null,
        title: 'Shared recipe', description: '', instructions: '', visibility: 'public',
        difficulty: null, estimatedCalories: null, ingredients: [], isFavorite: false,
        isFavoriteAuthor: false, coverImage: null, source: null,
      },
    ])
    const root = createRootRoute({ component: RecipesPage })
    const testRouter = createRouter({
      routeTree: root,
      history: createMemoryHistory({ initialEntries: ['/?scope=mine'] }),
    })

    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <RouterProvider router={testRouter} />
        </QueryClientProvider>
      </LocaleProvider>,
    )
    expect(await screen.findByRole('link', { name: 'Owned recipe' })).toBeVisible()
    expect(screen.queryByRole('link', { name: 'Shared recipe' })).not.toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Clear filter' })).toHaveAttribute('href', '/recipes')
  })

  it('applies and clears the pantry status filter', async () => {
    pantryApi.mockReset().mockResolvedValue([
      {
        id: 'have', name: 'Egg', status: 'have', quantityNote: '', confirmedAt: null,
        expiresOn: null, canonicalIngredientId: null, autoAddToShoppingListWhenOut: false, version: 1,
      },
      {
        id: 'out', name: 'Milk', status: 'out', quantityNote: '', confirmedAt: null,
        expiresOn: null, canonicalIngredientId: null, autoAddToShoppingListWhenOut: false, version: 1,
      },
    ])
    const root = createRootRoute({ component: PantryPage })
    const testRouter = createRouter({
      routeTree: root,
      history: createMemoryHistory({ initialEntries: ['/?status=have'] }),
    })
    render(
      <LocaleProvider initialLocale="en" storage={null}>
        <QueryClientProvider client={new QueryClient()}>
          <RouterProvider router={testRouter} />
        </QueryClientProvider>
      </LocaleProvider>,
    )
    const table = await screen.findByRole('table', { name: 'Pantry items' })
    expect(within(table).getByText('Egg')).toBeVisible()
    expect(within(table).queryByText('Milk')).not.toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'Clear filter' })).toHaveAttribute('href', '/pantry')
  })

  it('passes the actual bought alternative through the purchase contract', async () => {
    shoppingItems.mockReset().mockResolvedValue([{
      id: 'recipe-choice',
      name: 'Milk or oat milk',
      canonicalIngredientId: null,
      choiceAlternatives: [
        {
          id: 'milk-alt',
          name: 'Milk',
          canonicalIngredientId: 'milk',
          purchaseOptions: {
            resolution: 'reuse',
            candidates: [{
              id: 'pantry-milk', name: 'Milk', status: 'out', quantityNote: '',
              expiresOn: null, canonicalIngredientId: 'milk',
              autoAddToShoppingListWhenOut: false, version: 5,
            }],
          },
        },
        {
          id: 'oat-alt',
          name: 'Oat milk',
          canonicalIngredientId: 'oat-milk',
          purchaseOptions: {
            resolution: 'choice',
            candidates: [{
              id: 'pantry-oat', name: 'Oat carton', status: 'unknown', quantityNote: 'half carton',
              expiresOn: '2026-11-12', canonicalIngredientId: 'oat-milk',
              autoAddToShoppingListWhenOut: true, version: 7,
            }],
          },
        },
      ],
      automatic: false,
      pantryItemId: null,
      sources: [{ kind: 'recipe', label: 'Porridge', recipeId: 'recipe-1' }],
      version: 1,
      purchaseOptions: { resolution: 'selectAlternative', candidates: [] },
    }])
    render(
      <Providers>
        <ShoppingListPage />
      </Providers>,
    )
    fireEvent.click(await screen.findByRole('button', { name: 'Bought Milk or oat milk' }))
    expect(screen.getByRole('button', { name: 'Save to pantry and remove from list' })).toBeDisabled()
    fireEvent.click(screen.getByRole('radio', { name: 'Milk' }))
    expect(screen.getByRole('button', { name: 'Save to pantry and remove from list' })).toBeEnabled()
    fireEvent.click(screen.getByRole('radio', { name: 'Oat milk' }))
    expect(screen.getByRole('radio', { name: /Create with this name: Oat milk/ })).toBeChecked()
    expect(screen.getByRole('button', { name: 'Save to pantry and remove from list' })).toBeEnabled()
    fireEvent.click(screen.getByRole('radio', { name: /Use existing pantry item: Oat carton/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Save to pantry and remove from list' }))
    await waitFor(() => expect(purchaseShoppingItem).toHaveBeenCalledWith(
      'recipe-choice',
      expect.objectContaining({
        selectedAlternativeId: 'oat-alt',
        pantryItemId: 'pantry-oat',
        pantryExpectedVersion: 7,
        createUsingCurrentName: false,
        quantityNote: 'half carton',
        autoAddToShoppingListWhenOut: true,
      }),
      expect.any(String),
    ))
  })
})

describe('recipe card navigation and provenance', () => {
  function recipe(overrides: Partial<Recipe> = {}): Recipe {
    return {
      id: 'recipe-1',
      version: 1,
      isOwner: true,
      authorHandle: 'owner',
      authorNickname: 'Owner',
      title: 'Linked soup',
      description: 'Recipe description',
      instructions: 'Stir the soup.',
      visibility: 'private',
      publicationIntent: 'private',
      moderationState: 'clear',
      moderationReviewState: 'unreviewed',
      moderationVersion: 1,
      difficulty: null,
      estimatedCalories: null,
      ingredients: [{ name: 'water', quantityNote: '1 cup', canonicalIngredientId: null }],
      isFavorite: false,
      isFavoriteAuthor: false,
      coverImage: null,
      source: null,
      ...overrides,
    }
  }

  function renderDetail(value: Recipe, locale: Locale = 'en') {
    const root = createRootRoute({
      component: () => (
        <LocaleProvider initialLocale={locale} storage={null}>
          <QueryClientProvider client={new QueryClient()}>
            <RecipeDetailContent recipe={value} />
          </QueryClientProvider>
        </LocaleProvider>
      ),
    })
    const testRouter = createRouter({
      routeTree: root,
      history: createMemoryHistory({ initialEntries: ['/'] }),
    })
    return render(<RouterProvider router={testRouter} />)
  }

  it('uses recipe titles as the only detail links for private and public recipe cards', async () => {
    recipesApi.mockReset().mockResolvedValue([
      recipe(),
      recipe({
        id: 'recipe-2',
        title: 'Public stew',
        visibility: 'public',
        isOwner: false,
      }),
    ])
    const root = createRootRoute({
      component: () => (
        <LocaleProvider initialLocale="en" storage={null}>
          <QueryClientProvider client={new QueryClient()}>
            <RecipesPage />
          </QueryClientProvider>
        </LocaleProvider>
      ),
    })
    const testRouter = createRouter({
      routeTree: root,
      history: createMemoryHistory({ initialEntries: ['/recipes'] }),
    })
    render(<RouterProvider router={testRouter} />)

    expect(await screen.findByRole('link', { name: 'Linked soup' }))
      .toHaveAttribute('href', '/recipes/recipe-1')
    expect(screen.getByRole('link', { name: 'Public stew' }))
      .toHaveAttribute('href', '/recipes/recipe-2')
    expect(screen.queryByRole('link', { name: 'Open recipe' })).not.toBeInTheDocument()
    expect(document.querySelectorAll('h2 > a.recipe-title-link')).toHaveLength(2)
  })

  it('places recipe provenance after content and keeps cover provenance with the image', async () => {
    const detailed = recipe({
      isOwner: false,
      coverImage: {
        id: 'cover-1',
        url: '/api/recipes/recipe-1/cover',
        altText: 'Soup cover',
        sourceUrl: 'https://images.example.test/soup',
        sourceLicense: 'CC-BY-4.0',
        sourceAttribution: 'Cover photographer',
        version: 1,
        width: 640,
        height: 480,
      },
      source: {
        url: 'https://recipes.example.test/soup',
        title: 'Original soup',
        author: 'Recipe author',
        license: 'CC-BY-4.0',
        attribution: 'Recipe author — Original soup',
        warnings: ['Preparation time was inferred.'],
        confidence: 0.9,
      },
    })
    renderDetail(detailed)

    const article = await screen.findByRole('article')
    const method = screen.getByRole('heading', { name: 'Method' })
    const source = screen.getByRole('heading', { name: 'Source' })
    expect(method.compareDocumentPosition(source) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    const cover = screen.getByRole('img', { name: 'Soup cover' }).closest('figure')
    expect(cover).toHaveTextContent('Cover photographer')
    expect(within(cover as HTMLElement).getByRole('link', { name: 'Open image source' }))
      .toHaveAttribute('href', 'https://images.example.test/soup')
    const sourceSection = source.closest('section')
    expect(sourceSection).toHaveTextContent('Recipe author — Original soup')
    expect(within(sourceSection as HTMLElement).getByRole('link', { name: 'Original soup' }))
      .toHaveAttribute('target', '_blank')
    expect(sourceSection).not.toHaveTextContent('Cover photographer')
    cleanup()
    renderDetail(detailed, 'zh-CN')
    expect(await screen.findByRole('heading', { name: '来源' })).toBeVisible()
    expect(screen.getByRole('heading', { name: '做法' })).toBeVisible()
    expect(screen.getByText('分享用户：Owner')).toBeVisible()
    expect(screen.getByText('来源作者')).toBeVisible()
    expect(screen.queryByText('作者：Owner')).not.toBeInTheDocument()
  })

  it('omits empty recipe provenance and does not link non-HTTP source URLs', async () => {
    renderDetail(recipe())
    await screen.findByRole('article')
    expect(screen.queryByRole('heading', { name: 'Source' })).not.toBeInTheDocument()
    cleanup()
    renderDetail(recipe({
      source: {
        url: 'javascript:alert(1)',
        title: null,
        author: null,
        license: null,
        attribution: null,
        warnings: [],
        confidence: null,
      },
    }))
    expect(await screen.findByRole('heading', { name: 'Source' })).toBeVisible()
    expect(screen.queryByRole('link', { name: /javascript/ })).not.toBeInTheDocument()
  })
})

describe('profile setup', () => {
  const user = {
    id: 'user',
    email: 'private@example.test',
    displayName: 'Google Name',
    nickname: null,
    onboardingCompleted: false,
    locale: 'en' as const,
    capabilities: [],
  }

  it('defaults to locale-rendered anonymous state without publishing the Google name', async () => {
    saveProfile.mockClear()
    render(
      <Providers>
        <ProfileSetup user={user} />
      </Providers>,
    )
    expect(screen.getByLabelText('Anonymous user (default)')).toBeChecked()
    fireEvent.click(screen.getByRole('button', { name: 'Save public profile' }))
    await waitFor(() => expect(saveProfile).toHaveBeenCalledWith(null))
  })

  describe('language settings', () => {
    it('switches immediately and persists the account locale', async () => {
      saveLocale.mockClear()
      const user = {
        id: 'user',
        email: 'private@example.test',
        displayName: 'Google Name',
        nickname: null,
        onboardingCompleted: true,
        locale: 'en' as const,
        capabilities: [],
      }
      render(
        <Providers>
          <LanguageSettings user={user} />
        </Providers>,
      )
      fireEvent.change(screen.getByLabelText('Interface language'), { target: { value: 'zh-CN' } })
      expect(await screen.findByText('语言已保存。')).toBeVisible()
      expect(document.documentElement.lang).toBe('zh-CN')
      expect(saveLocale).toHaveBeenCalledWith('zh-CN')
      expect(storedLocale.value).toBe('zh-CN')
    })
  })

  it('copies the Google display name only after explicit selection', async () => {
    saveProfile.mockClear()
    render(
      <Providers>
        <ProfileSetup user={user} />
      </Providers>,
    )
    fireEvent.click(screen.getByLabelText('Use “Google Name” publicly'))
    fireEvent.click(screen.getByRole('button', { name: 'Save public profile' }))
    await waitFor(() => expect(saveProfile).toHaveBeenCalledWith('Google Name'))
  })

  it('renders the semantic anonymous choice in Simplified Chinese without changing its stored value', async () => {
    saveProfile.mockClear()
    render(
      <Providers locale="zh-CN">
        <ProfileSetup user={user} />
      </Providers>,
    )
    expect(screen.getByLabelText('匿名用户（默认）')).toBeChecked()
    fireEvent.click(screen.getByRole('button', { name: '保存公开资料' }))
    await waitFor(() => expect(saveProfile).toHaveBeenCalledWith(null))
    expect(document.documentElement.lang).toBe('zh-CN')
  })
})

describe('pantry confirmation', () => {
  it('supports skipping and clears skipped answers', () => {
    render(
      <Providers>
        <PantryConfirmation open onOpenChange={() => undefined} names={['Egg', 'Milk', 'Rice']} />
      </Providers>,
    )
    expect(screen.getByRole('heading', { name: 'Quick pantry check' })).toBeVisible()
    expect(screen.getByRole('button', { name: 'Skip for now' })).toBeVisible()
    expect(screen.getByRole('button', { name: 'Save answers' })).toBeDisabled()
    fireEvent.click(screen.getAllByLabelText('Still have it')[0])
    expect(screen.getByRole('button', { name: 'Save answers' })).toBeEnabled()
    fireEvent.click(screen.getByRole('button', { name: 'Skip for now' }))
    expect(screen.getByRole('button', { name: 'Save answers' })).toBeDisabled()
  })

  it('notifies the recommendation workbench after saving pantry answers', async () => {
    const onSaved = vi.fn()
    render(
      <Providers>
        <PantryConfirmation
          open
          onOpenChange={() => undefined}
          names={['Egg']}
          onSaved={onSaved}
        />
      </Providers>,
    )
    fireEvent.click(screen.getByLabelText('Still have it'))
    fireEvent.click(screen.getByRole('button', { name: 'Save answers' }))
    await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1))
  })

  describe('recipe cover fields', () => {
    it('exposes keyboard-accessible file, alt, source, license, attribution, and remove controls', () => {
      const onFile = vi.fn()
      const onRemove = vi.fn()
      render(
        <Providers>
          <RecipeCoverFields
            hasImage
            removed={false}
            preview="/api/recipe-images/image-id"
            metadata={{
              altText: 'Soup in a blue bowl',
              sourceUrl: '',
              sourceLicense: '',
              sourceAttribution: '',
            }}
            onMetadataChange={() => undefined}
            onFile={onFile}
            onRemove={onRemove}
          />
        </Providers>,
      )
      const input = screen.getByLabelText('Choose or drop a cover image')
      expect(input).toHaveAttribute('accept', 'image/jpeg,image/png,image/webp')
      const file = new File(['bytes'], 'cover.webp', { type: 'image/webp' })
      fireEvent.change(input, { target: { files: [file] } })
      expect(onFile).toHaveBeenCalledWith(file)
      expect(screen.getByLabelText('Image description (alt text)')).toBeRequired()
      expect(screen.getByLabelText('Source URL (optional)')).toHaveAttribute('type', 'url')
      expect(screen.getByLabelText('License (optional)')).toBeVisible()
      expect(screen.getByLabelText('Attribution (optional)')).toBeVisible()
      expect(screen.getByRole('img', { name: 'Soup in a blue bowl' })).toBeVisible()
      fireEvent.click(screen.getByRole('button', { name: 'Remove cover image' }))
      expect(onRemove).toHaveBeenCalledOnce()
    })
  })

  describe('expiration editor', () => {
    it('offers relative-day shortcuts, custom integer input, and clearing without a date picker', () => {
      const onChange = vi.fn()
      render(
        <Providers>
          <ExpirationEditor value="2026-01-31" onChange={onChange} now={new Date(2026, 0, 31, 12)} />
        </Providers>,
      )
      expect(screen.getByRole('button', { name: 'Today' })).toHaveAttribute('aria-pressed', 'true')
      expect(screen.queryByDisplayValue('2026-01-31')).not.toBeInTheDocument()
      fireEvent.click(screen.getByRole('button', { name: '30 days' }))
      expect(onChange).toHaveBeenLastCalledWith('2026-03-02')
      const input = screen.getByRole('spinbutton', { name: 'Expires this many days from today' })
      fireEvent.change(input, { target: { value: '14' } })
      expect(onChange).toHaveBeenLastCalledWith('2026-02-14')
      fireEvent.click(screen.getByRole('button', { name: 'Increase days until expiration' }))
      expect(onChange).toHaveBeenLastCalledWith('2026-02-15')
      fireEvent.click(screen.getByRole('button', { name: 'No expiration date' }))
      expect(onChange).toHaveBeenLastCalledWith(null)
    })

    it('preserves past dates and rejects non-integer or out-of-range pasted values', () => {
      const onChange = vi.fn()
      render(
        <Providers>
          <ExpirationEditor value="2026-01-30" onChange={onChange} now={new Date(2026, 0, 31, 12)} />
        </Providers>,
      )
      expect(screen.getByRole('spinbutton')).toHaveValue(-1)
      expect(screen.getByText(/Expired 1 day ago/)).toBeVisible()
      expect(screen.getByRole('alert')).toHaveTextContent(
        'This date is in the past. You can still save it',
      )
      const input = screen.getByRole('spinbutton')
      fireEvent.change(input, { target: { value: '1.5' } })
      expect(screen.getByText('Enter a whole number of days.')).toHaveAttribute('role', 'alert')
      fireEvent.change(input, { target: { value: '4000' } })
      expect(screen.getByText('Enter a number from -3650 to 3650.')).toHaveAttribute('role', 'alert')
      expect(onChange).not.toHaveBeenCalled()
    })

    it('localizes the relative confirmation in Simplified Chinese', () => {
      render(
        <Providers locale="zh-CN">
          <ExpirationEditor value="2026-11-11" onChange={() => undefined} now={new Date(2026, 10, 4, 23, 59)} />
        </Providers>,
      )
      expect(screen.getByRole('spinbutton', { name: '从今天起多少天后过期' })).toHaveValue(7)
      expect(screen.getByText(/从今天起 7 天后/)).toBeVisible()
    })
  })
})

describe('email OTP sign-in', () => {
  it('switches the guest locale without clearing an email already entered', () => {
    render(
      <Providers>
        <GuestLanguageSelector />
        <EmailOtpLogin onLogin={vi.fn()} passkeyEnabled={false} />
      </Providers>,
    )
    const email = screen.getByLabelText('Email address')
    fireEvent.change(email, { target: { value: 'user@example.test' } })
    fireEvent.change(screen.getByLabelText('Interface language'), { target: { value: 'zh-CN' } })
    expect(email).toHaveValue('user@example.test')
    expect(screen.getByLabelText('界面语言')).toHaveValue('zh-CN')
  })

  it('accepts a pasted six-digit code with one-time-code autocomplete', async () => {
    requestEmailOtp.mockClear()
    verifyEmailOtp.mockClear()
    const onLogin = vi.fn()
    render(
      <Providers>
        <EmailOtpLogin onLogin={onLogin} passkeyEnabled={false} />
      </Providers>,
    )
    fireEvent.change(screen.getByLabelText('Email address'), { target: { value: 'user@example.test' } })
    fireEvent.click(screen.getByRole('button', { name: 'Email me a code' }))
    const code = await screen.findByLabelText('Verification code')
    expect(code).toHaveAttribute('autocomplete', 'one-time-code')
    fireEvent.change(code, { target: { value: '12 34a56' } })
    expect(code).toHaveValue('123456')
    fireEvent.click(screen.getByRole('button', { name: 'Verify and sign in' }))
    await waitFor(() => expect(verifyEmailOtp).toHaveBeenCalledWith('challenge', '123456'))
    expect(onLogin).toHaveBeenCalled()
  })
})

describe('account security', () => {
  it('links email explicitly and keeps passkey registration disabled until linked', async () => {
    securitySettings.mockResolvedValueOnce({
      linkedEmails: [],
      googleEmail: null,
      googleSignInStatus: 'unavailable',
      passkeys: [],
    })
    render(
      <Providers>
        <AccountSecuritySettings />
      </Providers>,
    )
    expect(await screen.findByRole('button', { name: 'Register passkey' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Email address'), { target: { value: 'linked@example.test' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send linking code' }))
    const code = await screen.findByLabelText('Verification code')
    fireEvent.change(code, { target: { value: '654321' } })
    fireEvent.click(screen.getByRole('button', { name: 'Verify and link email' }))
    await waitFor(() => expect(verifyEmailLink).toHaveBeenCalledWith('link-challenge', '654321'))
  })

  it('locks Google-first email linking to the verified Google address', async () => {
    securitySettings.mockResolvedValueOnce({
      linkedEmails: [],
      googleEmail: 'owner@gmail.com',
      googleSignInStatus: 'linked',
      passkeys: [],
    })
    render(
      <Providers>
        <AccountSecuritySettings />
      </Providers>,
    )
    const email = await screen.findByLabelText('Email address')
    await waitFor(() => expect(email).toHaveValue('owner@gmail.com'))
    expect(email).toHaveAttribute('readonly')
    expect(screen.getByText(/must exactly match the verified Google email/i)).toBeInTheDocument()
  })

  it('explains that non-Gmail email-first accounts cannot add Google sign-in', async () => {
    securitySettings.mockResolvedValueOnce({
      linkedEmails: ['owner@example.com'],
      googleEmail: null,
      googleSignInStatus: 'unsupported',
      passkeys: [],
    })
    render(
      <Providers>
        <AccountSecuritySettings />
      </Providers>,
    )
    expect(await screen.findByText(/non-Gmail address/i)).toBeInTheDocument()
  })
})
