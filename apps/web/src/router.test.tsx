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
  LanguageSettings,
  PantryConfirmation,
  PrimaryNavigation,
  ProfileSetup,
  RecipeCoverFields,
  RecipeConflictNotice,
  RecipeEditorForm,
  RecipeIngredientList,
  RecipeMetadataFields,
  RecipeProvenanceFields,
  RecommendationsPage,
  ShoppingListPage,
  persistRecipeEdit,
  pantryStatusForIngredient,
  recipeSchema,
} from './router'
import type { RecipeForm } from './router'
import { LocaleProvider, useI18n, type Locale } from './i18n'
import type { PantryItem, Recipe, Recommendation } from './types'

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

const {
  deleteRecipeCover,
  addRecipeIngredientToShopping,
  purchaseShoppingItem,
  recommendationsApi,
  recipeApi,
  requestEmailLink,
  requestEmailOtp,
  saveLocale,
  saveProfile,
  saveRecipe,
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
  recommendationsApi: vi.fn(),
  addRecipeIngredientToShopping: vi.fn(),
  saveRecipe: vi.fn(),
  recipeApi: vi.fn(),
  uploadRecipeCover: vi.fn(),
  updateRecipeCoverMetadata: vi.fn(),
  deleteRecipeCover: vi.fn(),
}))

vi.mock('./api', async () => {
  const actual = await vi.importActual<typeof import('./api')>('./api')
  return {
    ...actual,
    api: {
      ...actual.api,
      savePantry: vi.fn().mockResolvedValue([]),
      purchaseShoppingItem,
      pantry: vi.fn().mockResolvedValue([
        {
          id: 'pantry-1',
          name: 'Milk',
          status: 'out',
          quantityNote: '',
          confirmedAt: null,
          expiresOn: null,
          canonicalIngredientId: null,
          autoAddToShoppingListWhenOut: true,
          version: 1,
        },
      ]),
      shoppingItems,
      addShoppingItem: vi.fn().mockResolvedValue([]),
      updateShoppingItem: vi.fn().mockResolvedValue([]),
      deleteShoppingItem: vi.fn().mockResolvedValue([]),
      recommendations: recommendationsApi,
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

afterEach(cleanup)

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
      difficulty: null,
      estimatedCalories: null,
      ingredients: [{ name: 'water', quantityNote: '', canonicalIngredientId: null }],
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
      ingredients: [{ name: 'water', quantityNote: '', canonicalIngredientId: null }],
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
        ingredients: [{ name: 'water', quantityNote: '', canonicalIngredientId: null }],
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
        <input type="hidden" {...form.register('ingredients.0.name')} />
        <input type="hidden" {...form.register('ingredients.0.quantityNote')} />
        <input type="hidden" {...form.register('ingredients.0.canonicalIngredientId')} />
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
          ingredients: [{ name: 'water', quantityNote: '', canonicalIngredientId: null }],
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
  function renderRecommendations(locale: Locale = 'en') {
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
        authorHandle: 'owner',
        authorNickname: null,
        title: 'Saffron rice',
        description: '',
        instructions: '',
        visibility: 'private' as const,
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

  it('renders semantic membership instead of an add action when already active', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation(['Saffron'])])
    renderRecommendations()
    expect(await screen.findByText('Saffron: In shopping list')).toBeVisible()
    expect(screen.queryByRole('button', { name: 'Add Saffron' })).not.toBeInTheDocument()
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

  it('prevents duplicate submits and transitions immediately after a successful add', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    let resolveAdd!: (value: never[]) => void
    addRecipeIngredientToShopping.mockReset().mockReturnValue(
      new Promise<never[]>((resolve) => {
        resolveAdd = resolve
      }),
    )
    renderRecommendations()
    const add = await screen.findByRole('button', { name: 'Add Saffron' })
    fireEvent.click(add)
    fireEvent.click(add)
    await waitFor(() => expect(addRecipeIngredientToShopping).toHaveBeenCalledTimes(1))
    expect(screen.getByRole('button', { name: 'Adding Saffron…' })).toBeDisabled()
    resolveAdd([])
    expect(await screen.findByText('Saffron: In shopping list')).toBeVisible()
    expect(screen.getByRole('status')).toHaveTextContent('Saffron added to the shopping list.')
  })

  it('restores the add action after failure and never claims success', async () => {
    recommendationsApi.mockReset().mockResolvedValue([recommendation()])
    addRecipeIngredientToShopping.mockReset().mockRejectedValue(new Error('network failed'))
    renderRecommendations()
    fireEvent.click(await screen.findByRole('button', { name: 'Add Saffron' }))
    expect(await screen.findByText('network failed')).toBeVisible()
    expect(screen.getByRole('button', { name: 'Add Saffron' })).toBeEnabled()
    expect(screen.queryByText('Saffron: In shopping list')).not.toBeInTheDocument()
    expect(screen.queryByText('Saffron added to the shopping list.')).not.toBeInTheDocument()
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
