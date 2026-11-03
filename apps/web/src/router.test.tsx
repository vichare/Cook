import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { useForm } from 'react-hook-form'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  AccountSecuritySettings,
  EmailOtpLogin,
  ExpirationEditor,
  GuestLanguageSelector,
  LanguageSettings,
  PantryConfirmation,
  ProfileSetup,
  RecipeCoverFields,
  RecipeConflictNotice,
  RecipeMetadataFields,
  ShoppingListPage,
  persistRecipeEdit,
} from './router'
import type { RecipeForm } from './router'
import { LocaleProvider, type Locale } from './i18n'
import type { Recipe } from './types'

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
  markAutomaticShoppingPurchased,
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
      completedAt: null,
      pantryItemId: 'pantry-1',
      sources: [{ kind: 'automatic', label: '', recipeId: null }],
    },
    {
      id: 'manual-1',
      name: 'Rice',
      canonicalIngredientId: null,
      automatic: false,
      completedAt: null,
      pantryItemId: null,
      sources: [{ kind: 'manual', label: '', recipeId: null }],
    },
  ]),
  markAutomaticShoppingPurchased: vi.fn().mockResolvedValue([]),
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
        },
      ]),
      shoppingItems,
      addShoppingItem: vi.fn().mockResolvedValue([]),
      updateShoppingItem: vi.fn().mockResolvedValue([]),
      completeShoppingItem: vi.fn().mockResolvedValue([]),
      deleteShoppingItem: vi.fn().mockResolvedValue([]),
      markAutomaticShoppingPurchased,
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
  it('separates automatic and manual items and confirms a pantry purchase', async () => {
    shoppingItems.mockClear()
    markAutomaticShoppingPurchased.mockClear()
    vi.spyOn(window, 'confirm').mockReturnValue(true)
    render(
      <Providers>
        <ShoppingListPage />
      </Providers>,
    )
    expect(await screen.findByRole('heading', { name: 'Automatic restock' })).toBeVisible()
    expect(screen.getByRole('heading', { name: 'Manual and recipe items' })).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Mark Milk purchased' }))
    await waitFor(() => expect(markAutomaticShoppingPurchased).toHaveBeenCalledWith('pantry-1'))
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

  function RecipeMetadataHarness() {
    const form = useForm<RecipeForm>({
      defaultValues: {
        title: '',
        description: '',
        instructions: '',
        visibility: 'private',
        difficulty: null,
        estimatedCalories: null,
        ingredients: [{ name: 'water', quantityNote: '', canonicalIngredientId: null }],
      },
    })
    return <RecipeMetadataFields form={form} />
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
      const calories = screen.getByRole('spinbutton', { name: /Estimated total calories/ })
      fireEvent.change(calories, { target: { value: '850' } })
      expect(calories).toHaveValue(850)
      fireEvent.change(difficulty, { target: { value: '' } })
      expect(difficulty).toHaveValue('')
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

  it('renders shopping controls in Simplified Chinese', async () => {
    render(
      <Providers locale="zh-CN">
        <ShoppingListPage />
      </Providers>,
    )
    expect(await screen.findByRole('heading', { name: '自动补货' })).toBeVisible()
    expect(screen.getByRole('button', { name: 'Milk 已买到' })).toBeVisible()
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
