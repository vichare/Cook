import * as Dialog from '@radix-ui/react-dialog'
import { zodResolver } from '@hookform/resolvers/zod'
import {
  Link,
  Outlet,
  createRootRouteWithContext,
  createRoute,
  createRouter,
  useNavigate,
  useSearch,
} from '@tanstack/react-router'
import { QueryClient, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { useFieldArray, useForm, type UseFormReturn } from 'react-hook-form'
import { z } from 'zod'
import { ApiError, api } from './api'
import { MarkdownEditor, SafeMarkdown } from './MarkdownEditor'
import type {
  ImportDraft,
  IngredientScope,
  IngredientReference,
  ModerationCase,
  ModerationQueueItem,
  PantryItem,
  PantryStatus,
  Recipe,
  Recommendation,
  RecommendationSource,
  RecommendationSettings,
  RecommendationReason,
  ShoppingItem,
  SecuritySettings as SecuritySettingsData,
  User,
} from './types'
import { IngredientCombobox } from './IngredientCombobox'
import {
  asIngredientReference,
  emptyIngredientGroup,
  expirationLabel,
  expirationRelativeDays,
  expirationShortcut,
  ingredientIdentity,
  localDateValue,
  normalizeIngredientGroupOrder,
  normalizeIngredient,
  recipeIngredientGroups,
  splitIngredientList,
} from './ingredients'
import { createPasskey, getPasskey } from './webauthn'
import { useI18n, type Locale, type Translate } from './i18n'
import { useRecommendationWorkbench } from './recommendation-workbench'
import {
  moveRecommendationIngredient,
  prepareRecommendationIngredientBatch,
  type RecommendationIngredientBucket,
  type RecommendationIngredientMoveRequest,
} from './recommendation-ingredients'
import {
  positionRecommendationMoveMenu,
  type RecommendationMoveMenuPosition,
} from './recommendation-move-menu'
import {
  recommendationControlsKey,
  type RecommendationControls,
} from './recommendation-url'

interface RouterContext {
  queryClient: QueryClient
}

interface PendingRecommendationIngredientBatch {
  baseKey: string
  confirmationIndex: number
  confirmations: RecommendationIngredientMoveRequest[]
  controls: RecommendationControls
  locationGeneration: number
}

const rootRoute = createRootRouteWithContext<RouterContext>()({
  component: Root,
  notFoundComponent: NotFoundPage,
})

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: HomePage,
})
const recommendationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/recommendations',
  component: RecommendationsPage,
})
const recipesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/recipes',
  validateSearch: z.object({ scope: z.enum(['mine']).optional() }),
  component: RecipesPage,
})
const newRecipeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/recipes/new',
  component: () => <RecipeEditor />,
})
const recipeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/recipes/$recipeId',
  component: RecipeDetailPage,
})
const authorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/authors/$handle',
  component: AuthorProfilePage,
})
const editRecipeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/recipes/$recipeId/edit',
  component: EditRecipePage,
})
const pantryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/pantry',
  validateSearch: z.object({ status: z.enum(['have', 'out', 'unknown']).optional() }),
  component: PantryPage,
})
const shoppingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/shopping-list',
  component: ShoppingListPage,
})
const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  component: SettingsPage,
})
const notificationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/notifications',
  component: NotificationsPage,
})
const moderatorClaimRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/admin/claim',
  component: ModeratorClaimPage,
})
const moderationQueueRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/admin/moderation',
  component: ModerationQueuePage,
})
const moderationCaseRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/admin/moderation/cases/$caseId',
  component: ModerationCasePage,
})
const importsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/imports',
  component: ImportsPage,
})
const importDraftRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/imports/$draftId',
  component: ImportDraftPage,
})

const routeTree = rootRoute.addChildren([
  indexRoute,
  recommendationsRoute,
  recipesRoute,
  newRecipeRoute,
  recipeRoute,
  authorRoute,
  editRecipeRoute,
  pantryRoute,
  shoppingRoute,
  settingsRoute,
  notificationsRoute,
  moderatorClaimRoute,
  moderationQueueRoute,
  moderationCaseRoute,
  importsRoute,
  importDraftRoute,
])

export const router = createRouter({ routeTree, context: { queryClient: undefined! } })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}

function Root() {
  const queryClient = useQueryClient()
  const { locale, setLocale, t } = useI18n()
  const me = useQuery({ queryKey: ['me'], queryFn: api.me })
  const initializeLocale = useMutation({
    mutationFn: api.saveLocale,
    onSuccess: (user) => queryClient.setQueryData(['me'], user),
  })
  useEffect(() => {
    if (!me.data) return
    if (me.data.locale) {
      if (me.data.locale !== locale) setLocale(me.data.locale)
      return
    }
    if (!initializeLocale.isPending && !initializeLocale.isSuccess && !initializeLocale.error) {
      initializeLocale.mutate(locale)
    }
  }, [initializeLocale, locale, me.data, setLocale])
  if (me.isLoading) return <StatePanel title={t('app.openingKitchen')} detail={t('app.loadingWorkspace')} />
  if (me.error instanceof ApiError && me.error.status === 401) {
    if (window.location.pathname.startsWith('/authors/')) {
      return (
        <div className="app-shell">
          <header className="site-header">
            <Link to="/" className="brand">Cook</Link>
            <GuestLanguageSelector />
          </header>
          <main id="main"><Outlet /></main>
        </div>
      )
    }
    return <Login onLogin={() => void queryClient.invalidateQueries({ queryKey: ['me'] })} />
  }
  if (me.error) return <StatePanel title={t('app.startFailed')} detail={errorMessage(me.error, t)} />
  if (initializeLocale.error) {
    return <StatePanel title={t('app.startFailed')} detail={errorMessage(initializeLocale.error, t)} />
  }

  return (
    <div className="app-shell">
      <header className="site-header">
        <Link to="/" className="brand">
          Cook
        </Link>
        <PrimaryNavigation />
        <button
          className="button ghost desktop-sign-out"
          onClick={() =>
            void api.logout().then(() => queryClient.setQueryData(['me'], undefined)).finally(() => location.reload())
          }
        >
          {t('app.signOut')}
        </button>
      </header>
      <main id="main">
        <Outlet />
      </main>
      {me.data && !me.data.onboardingCompleted && <ProfileSetup user={me.data} />}
    </div>
  )
}

const primaryDestinations = [
  ['/recommendations', 'app.recommend'],
  ['/recipes', 'app.recipes'],
  ['/pantry', 'app.pantry'],
  ['/shopping-list', 'app.shopping'],
  ['/imports', 'app.imports'],
  ['/notifications', 'app.notifications'],
  ['/settings', 'app.settings'],
] as const

export function PrimaryNavigation() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const me = useQuery({ queryKey: ['me'], queryFn: api.me })
  const [open, setOpen] = useState(false)
  useEffect(() => {
    if (!window.matchMedia) return
    const media = window.matchMedia('(min-width: 761px)')
    const closeOnDesktop = (event: MediaQueryListEvent | MediaQueryList) => {
      if (event.matches) setOpen(false)
    }
    closeOnDesktop(media)
    media.addEventListener('change', closeOnDesktop)
    return () => media.removeEventListener('change', closeOnDesktop)
  }, [])
  const signOut = () =>
    void api.logout().then(() => queryClient.setQueryData(['me'], undefined)).finally(() => location.reload())
  return (
    <>
      <nav className="desktop-nav" aria-label={t('app.primaryNavigation')}>
        {primaryDestinations.map(([to, key]) => (
          <Link key={to} to={to}>{t(key)}</Link>
        ))}
        {me.data?.capabilities.includes('content_moderation') && (
          <Link to="/admin/moderation">{t('app.moderation')}</Link>
        )}
      </nav>
      <Dialog.Root open={open} onOpenChange={setOpen}>
        <Dialog.Trigger asChild>
          <button type="button" className="button ghost mobile-menu-trigger">
            {t('app.menu')}
          </button>
        </Dialog.Trigger>
        <Dialog.Portal>
          <Dialog.Overlay className="mobile-nav-overlay" />
          <Dialog.Content className="mobile-nav-dialog">
            <div className="mobile-nav-heading">
              <Dialog.Title>{t('app.menu')}</Dialog.Title>
              <Dialog.Close asChild>
                <button type="button" className="button ghost" aria-label={t('app.closeMenu')}>
                  ×
                </button>
              </Dialog.Close>
            </div>
            <Dialog.Description className="sr-only">
              {t('app.menuDescription')}
            </Dialog.Description>
            <nav aria-label={t('app.primaryNavigation')}>
              {primaryDestinations.map(([to, key]) => (
                <Link key={to} to={to} onClick={() => setOpen(false)}>{t(key)}</Link>
              ))}
              {me.data?.capabilities.includes('content_moderation') && (
                <Link to="/admin/moderation" onClick={() => setOpen(false)}>
                  {t('app.moderation')}
                </Link>
              )}
            </nav>
            <button type="button" className="button ghost mobile-sign-out" onClick={signOut}>
              {t('app.signOut')}
            </button>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </>
  )
}

function NotFoundPage() {
  const { t } = useI18n()
  return <StatePanel title={t('app.pageNotFound')} detail={t('app.pageNotFoundDetail')} />
}

function Login({ onLogin }: { onLogin: () => void }) {
  const { t } = useI18n()
  const [name, setName] = useState('Demo Cook')
  const googleConflict = new URLSearchParams(window.location.search).get('authError') === 'google-account-conflict'
  const config = useQuery({ queryKey: ['auth-config'], queryFn: api.authConfig })
  const login = useMutation({ mutationFn: api.devLogin, onSuccess: onLogin })
  useEffect(() => {
    if (!googleConflict) return
    const url = new URL(window.location.href)
    url.searchParams.delete('authError')
    window.history.replaceState(null, '', `${url.pathname}${url.search}${url.hash}`)
  }, [googleConflict])
  if (config.isLoading) return <StatePanel title={t('login.loading')} detail={t('login.checking')} />
  if (config.error) return <StatePanel title={t('login.unavailable')} detail={errorMessage(config.error, t)} />
  return (
    <main id="main" className="login">
      <section className="hero-card">
        <GuestLanguageSelector />
        <p className="eyebrow">{t('login.eyebrow')}</p>
        <h1>{t('login.title')}</h1>
        <p>{t('login.intro')}</p>
        {googleConflict && <p className="error-text" role="alert">{t('login.googleConflict')}</p>}
        {config.data?.mode === 'development' && (
          <form
            onSubmit={(event) => {
              event.preventDefault()
              login.mutate(name)
            }}
          >
            <label>
              {t('login.devName')}
              <input value={name} onChange={(event) => setName(event.target.value)} required />
            </label>
            <button className="button primary" disabled={login.isPending}>
              {login.isPending ? t('login.signingIn') : t('login.enterDev')}
            </button>
            {login.error && <ErrorText error={login.error} />}
          </form>
        )}
        {config.data?.emailOtp && <EmailOtpLogin onLogin={onLogin} passkeyEnabled={config.data.passkey} />}
        {config.data?.google && (
          <a className="button secondary" href="/api/auth/google/start">
            {t('login.google')}
          </a>
        )}
        <small>
          {t('login.securityNote')}
        </small>
      </section>
    </main>
  )
}

export function GuestLanguageSelector() {
  const { locale, setLocale, t } = useI18n()
  return (
    <label>
      {t('locale.label')}
      <select value={locale} onChange={(event) => setLocale(event.target.value as Locale)}>
        <option value="en">English</option>
        <option value="zh-CN">简体中文</option>
      </select>
    </label>
  )
}

export function EmailOtpLogin({
  onLogin,
  passkeyEnabled,
}: {
  onLogin: () => void
  passkeyEnabled: boolean
}) {
  const { formatNumber, t } = useI18n()
  const [email, setEmail] = useState('')
  const [code, setCode] = useState('')
  const [challenge, setChallenge] = useState<Awaited<ReturnType<typeof api.requestEmailOtp>> | null>(null)
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!challenge) return
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [challenge])
  const request = useMutation({
    mutationFn: () => api.requestEmailOtp(email),
    onSuccess: (value) => {
      setChallenge(value)
      setCode('')
      setNow(Date.now())
    },
  })
  const verify = useMutation({
    mutationFn: () => api.verifyEmailOtp(challenge!.challengeId, code),
    onSuccess: onLogin,
  })
  const passkey = useMutation({
    mutationFn: async () => {
      const options = await api.startPasskeyLogin(email)
      return api.finishPasskeyLogin(await getPasskey(options))
    },
    onSuccess: onLogin,
  })
  const resendSeconds = challenge
    ? Math.max(0, Math.ceil((new Date(challenge.resendAllowedAt).getTime() - now) / 1000))
    : 0
  return (
    <section className="auth-method">
      <h2>{t('otp.title')}</h2>
      <label>
        {t('otp.email')}
        <input
          type="email"
          autoComplete="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
          required
        />
      </label>
      {!challenge ? (
        <div className="auth-actions">
          <button className="button primary" disabled={!email || request.isPending} onClick={() => request.mutate()}>
            {t('otp.sendCode')}
          </button>
          {passkeyEnabled && (
            <button className="button secondary" disabled={!email || passkey.isPending} onClick={() => passkey.mutate()}>
              {t('otp.passkey')}
            </button>
          )}
        </div>
      ) : (
        <>
          <p role="status">{t('otp.genericSent')}</p>
          <label>
            {t('otp.code')}
            <input
              value={code}
              onChange={(event) => setCode(event.target.value.replace(/\D/g, '').slice(0, 6))}
              autoComplete="one-time-code"
              inputMode="numeric"
              pattern="[0-9]{6}"
              maxLength={6}
              required
            />
          </label>
          <div className="auth-actions">
            <button
              className="button primary"
              disabled={code.length !== 6 || verify.isPending}
              onClick={() => verify.mutate()}
            >
              {t('otp.verify')}
            </button>
            <button
              className="button ghost"
              disabled={resendSeconds > 0 || request.isPending}
              onClick={() => request.mutate()}
            >
              {resendSeconds > 0
                ? t('otp.resendIn', { seconds: formatNumber(resendSeconds) })
                : t('otp.resend')}
            </button>
            <button className="button ghost" onClick={() => setChallenge(null)}>
              {t('otp.otherEmail')}
            </button>
          </div>
        </>
      )}
      {(request.error || verify.error || passkey.error) && (
        <ErrorText error={request.error ?? verify.error ?? passkey.error} />
      )}
    </section>
  )
}

export function HomePage() {
  const { formatNumber, t } = useI18n()
  const recipes = useQuery({ queryKey: ['recipes'], queryFn: api.recipes })
  const pantry = useQuery({ queryKey: ['pantry'], queryFn: api.pantry })
  const ownedRecipes = recipes.data?.filter((recipe) => recipe.isOwner).length
  const availableItems = pantry.data?.filter((item) => item.status === 'have').length
  return (
    <Page title={t('home.title')} intro={t('home.intro')}>
      <div className="metric-grid">
        <Metric
          to="/recipes"
          search={{ scope: 'mine' }}
          label={t('home.myRecipes')}
          value={ownedRecipes === undefined ? '—' : formatNumber(ownedRecipes)}
        />
        <Metric
          to="/pantry"
          label={t('home.pantryItems')}
          value={pantry.data ? formatNumber(pantry.data.length) : '—'}
        />
        <Metric
          to="/pantry"
          search={{ status: 'have' }}
          label={t('home.confirmedAvailable')}
          value={availableItems === undefined ? '—' : formatNumber(availableItems)}
        />
      </div>
      <div className="action-grid">
        <Link className="action-card" to="/recommendations">
          <strong>{t('home.findMeal')}</strong>
          <span>{t('home.findMealDetail')}</span>
        </Link>
        <Link className="action-card" to="/pantry">
          <strong>{t('home.updatePantry')}</strong>
          <span>{t('home.updatePantryDetail')}</span>
        </Link>
        <Link className="action-card" to="/recipes/new">
          <strong>{t('home.addRecipe')}</strong>
          <span>{t('home.addRecipeDetail')}</span>
        </Link>
      </div>
    </Page>
  )
}

export function RecommendationsPage() {
  const { formatDate, formatNumber, t } = useI18n()
  const queryClient = useQueryClient()
  const settings = useQuery({ queryKey: ['settings'], queryFn: api.settings })
  const {
    activeControls,
    controls: filters,
    locationGeneration,
    results: recommendations,
    retry,
    setControls: setFilters,
    updateResults,
    urlIssues,
  } = useRecommendationWorkbench(settings.data)
  const shopping = useQuery({ queryKey: ['shopping-list'], queryFn: api.shoppingItems })
  const verifyNames = useMemo(
    () => [...new Set(recommendations.data.flatMap((item) => item.verify))].slice(0, 5),
    [recommendations.data],
  )
  const [confirmOpen, setConfirmOpen] = useState(false)
  const shoppingAddInFlight = useRef(new Set<string>())
  const [pendingShoppingIdentity, setPendingShoppingIdentity] = useState<string | null>(null)
  const [shoppingNotice, setShoppingNotice] = useState<{ id: number; name: string } | null>(null)
  const shoppingNoticeSequence = useRef(0)
  const [moveNotice, setMoveNotice] = useState<{ id: number; message: string } | null>(null)
  const moveNoticeSequence = useRef(0)
  const [moveBatch, setMoveBatch] = useState<PendingRecommendationIngredientBatch | null>(null)
  const lastMoveConfirmation = useRef<RecommendationIngredientMoveRequest | null>(null)
  const moveConfirmationRestoreFocus = useRef<HTMLElement | null>(null)
  const confirmedMove = useRef(false)
  const [restoreMoveConfirmationFocus, setRestoreMoveConfirmationFocus] = useState(false)
  const currentLocationGeneration = useRef(locationGeneration)
  currentLocationGeneration.current = locationGeneration
  const currentControlsKey = useRef('')
  currentControlsKey.current = filters ? recommendationControlsKey(filters) : ''
  const moveButtonRefs = useRef(new Map<string, HTMLButtonElement>())
  const [pendingMoveFocus, setPendingMoveFocus] = useState<{
    bucket: RecommendationIngredientBucket
    identity: string
  } | null>(null)
  useEffect(() => {
    if (!pendingMoveFocus) return
    const target = moveButtonRefs.current.get(
      `${pendingMoveFocus.bucket}:${pendingMoveFocus.identity}`,
    )
    if (!target) return
    target.focus()
    setPendingMoveFocus(null)
  }, [filters, pendingMoveFocus])
  useEffect(() => {
    if (!restoreMoveConfirmationFocus || moveBatch) return
    moveConfirmationRestoreFocus.current?.focus()
    setRestoreMoveConfirmationFocus(false)
  }, [moveBatch, restoreMoveConfirmationFocus])
  useEffect(() => {
    if (!moveBatch || !filters) return
    if (
      moveBatch.locationGeneration === locationGeneration
      && moveBatch.baseKey === recommendationControlsKey(filters)
    ) return
    confirmedMove.current = false
    setMoveBatch(null)
    setRestoreMoveConfirmationFocus(true)
    setMoveNotice({
      id: ++moveNoticeSequence.current,
      message: t('recommend.moveCanceledNavigation'),
    })
  }, [filters, locationGeneration, moveBatch, t])
  const addShopping = useMutation({
    mutationFn: ({
      recipeId,
      ingredient,
      identity,
    }: {
      recipeId: string
      ingredient: IngredientReference & {
        groupId?: string | null
        alternativeId?: string | null
      }
      identity: string
    }) => api.addRecipeIngredientToShopping(recipeId, ingredient).then((items) => ({
      items,
      identity,
      recipeId,
      name: ingredient.name,
    })),
    onSuccess: ({ items, recipeId, name }) => {
      queryClient.setQueryData(['shopping-list'], items)
      queryClient.setQueriesData<Recommendation[]>({ queryKey: ['recommendations'] }, (current) =>
        current?.map((recommendation) => {
          return recommendation.recipe.id !== recipeId ? recommendation : {
            ...recommendation,
            inShoppingList: [...new Set([...recommendation.inShoppingList, name])],
          }
        }),
      )
      updateResults((current) =>
        current.map((recommendation) => {
          return recommendation.recipe.id !== recipeId ? recommendation : {
            ...recommendation,
            inShoppingList: [...new Set([...recommendation.inShoppingList, name])],
          }
        }),
      )
      setShoppingNotice({ id: ++shoppingNoticeSequence.current, name })
    },
    onSettled: (_data, _error, variables) => {
      shoppingAddInFlight.current.delete(variables.identity)
      setPendingShoppingIdentity((current) => current === variables.identity ? null : current)
    },
  })
  const renderRecommendationCard = (result: Recommendation) => {
    const desiredCoverage = recommendationDesiredCoverage(result, activeControls?.preferred.length ?? 0)
    const shoppingCoverage = recommendationShoppingCoverage(result, shopping.data ?? [])
    return (
      <article className="recommendation-card" key={result.recipe.id}>
        <div className="card-heading">
          <div>
            <span className={`status ${result.status.toLowerCase()}`}>{t(`status.${result.status}`)}</span>
            <h2>
              <Link
                className="recipe-title-link"
                to="/recipes/$recipeId"
                params={{ recipeId: result.recipe.id }}
              >
                {result.recipe.title}
              </Link>
            </h2>
          </div>
        </div>
        {(result.recipe.coverImage || result.recipe.description) && (
          <div className="recommendation-preview">
            {result.recipe.coverImage && (
              <img
                src={result.recipe.coverImage.url}
                alt={result.recipe.coverImage.altText ?? ''}
              />
            )}
            {result.recipe.description && (
              <div className="recommendation-description">
                <SafeMarkdown value={result.recipe.description} />
              </div>
            )}
          </div>
        )}
        <div className="recommendation-intent-summary">
          <span className="recommendation-source-chip">{t('recommend.sourceSummary', {
            source: recommendationSourceLabel(result.primarySource, t),
          })}</span>
          {desiredCoverage.matched > 0 && (
            <span className="recommendation-source-chip">
              {t('recommend.desiredCoverage', desiredCoverage)}
            </span>
          )}
        </div>
        <RecommendationIngredientGroups
          recommendation={result}
          shoppingItems={shopping.data ?? []}
          pendingIdentity={pendingShoppingIdentity}
          onAdd={(ingredient, groupId, alternativeId) => {
            const identity = groupId
              ? `g:${groupId}:${alternativeId ?? 'any'}`
              : ingredientIdentity(ingredient)
            if (shoppingAddInFlight.current.has(identity)) return
            shoppingAddInFlight.current.add(identity)
            setPendingShoppingIdentity(identity)
            addShopping.mutate({
              recipeId: result.recipe.id,
              ingredient: groupId ? { ...ingredient, groupId, alternativeId } : ingredient,
              identity,
            })
          }}
        />
        <details>
          <summary>{t('recommend.scoreDetails')}</summary>
          <RecipeMetadata
            difficulty={result.recipe.difficulty}
            estimatedCalories={result.recipe.estimatedCalories}
            formatNumber={formatNumber}
            t={t}
          />
          {desiredCoverage.total > 0 && (
            <p>{t('recommend.reason.desiredCoverage', desiredCoverage)}</p>
          )}
          {shoppingCoverage.total > 0 && (
            <p>{t('recommend.reason.shoppingCoverage', shoppingCoverage)}</p>
          )}
          <p>
            {result.reasonDetails
              .map((reason) => recommendationReasonLabel(reason, t, formatDate, formatNumber))
              .join(' · ')}
          </p>
          <ul className="plain-list" aria-label={t('recommend.ingredientReasons')}>
            {result.ingredientReasons.map((reason) => (
              <li key={`${reason.ingredient}-${reason.code}`}>
                {recommendationIngredientReasonLabel(reason, t, formatDate)}
              </li>
            ))}
          </ul>
          <p>{t('recommend.matchedSources', {
            sources: result.matchedSources.map((source) => recommendationSourceLabel(source, t)).join(', '),
          })}</p>
        </details>
      </article>
    )
  }

  if (settings.error) {
    return (
      <Page title={t('recommend.title')} intro={t('recommend.intro')}>
        <StatePanel title={t('common.errorTitle')} detail={errorMessage(settings.error, t)} />
      </Page>
    )
  }
  if (!filters || !activeControls) {
    return (
      <Page title={t('recommend.title')} intro={t('recommend.intro')}>
        <StatePanel title={t('common.loading')} detail={t('common.loadingDetail')} />
      </Page>
    )
  }

  const resultStatus = recommendations.status === 'loading'
    ? t('recommend.loading')
    : recommendations.status === 'success'
      ? t('recommend.loaded', { count: formatNumber(recommendations.data.length) })
      : recommendations.status === 'error'
        ? t('recommend.failed')
        : ''
  const pendingMoveConfirmation = moveBatch
    ? moveBatch.confirmations[moveBatch.confirmationIndex] ?? null
    : null
  if (pendingMoveConfirmation) lastMoveConfirmation.current = pendingMoveConfirmation
  const announceMove = (message: string) => {
    setMoveNotice({ id: ++moveNoticeSequence.current, message })
  }
  const commitControls = (next: RecommendationControls) => {
    const result = setFilters(next)
    if (result.status === 'rejected') announceMove(t('recommend.controlsTooLong'))
    return result
  }
  const commitMove = (request: RecommendationIngredientMoveRequest) => {
    const result = moveRecommendationIngredient(
      filters,
      request.identity,
      request.from,
      request.to,
    )
    if (!result.ingredient || result.outcome === 'source-missing') {
      setPendingMoveFocus({ bucket: request.from, identity: request.identity })
      announceMove(t('recommend.moveFailed', { name: request.ingredient.name }))
      return
    }
    if (result.outcome === 'target-full') {
      setPendingMoveFocus({ bucket: request.from, identity: request.identity })
      announceMove(t('recommend.moveTargetFull', {
        bucket: recommendationBucketLabel(request.to, t),
        name: request.ingredient.name,
      }))
      return
    }
    const commit = commitControls(result.controls)
    if (commit.status !== 'committed') {
      setPendingMoveFocus({ bucket: request.from, identity: request.identity })
      if (commit.status === 'unchanged') {
        announceMove(t('recommend.moveFailed', { name: request.ingredient.name }))
      }
      return
    }
    setPendingMoveFocus({ bucket: request.to, identity: result.focusIdentity })
    announceMove(t(
      result.outcome === 'already-present'
        ? 'recommend.moveAlreadyPresent'
        : 'recommend.moveSuccess',
      {
        name: result.ingredient.name,
        bucket: recommendationBucketLabel(request.to, t),
      },
    ))
  }
  const queueAdditions = (
    bucket: RecommendationIngredientBucket,
    additions: IngredientReference[],
  ) => {
    const prepared = prepareRecommendationIngredientBatch(filters, additions, bucket)
    if (prepared.outcome === 'oversized') {
      announceMove(t('recommend.controlsTooLong'))
      return false
    }
    if (prepared.outcome === 'target-full') {
      announceMove(t('recommend.moveTargetFull', {
        bucket: recommendationBucketLabel(bucket, t),
        name: prepared.ingredient.name,
      }))
      return false
    }
    if (prepared.confirmations.length === 0) {
      if (prepared.controls === filters) return true
      return commitControls(prepared.controls).status !== 'rejected'
    }
    moveConfirmationRestoreFocus.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    setMoveBatch({
      baseKey: recommendationControlsKey(filters),
      confirmationIndex: 0,
      confirmations: prepared.confirmations,
      controls: prepared.controls,
      locationGeneration,
    })
    return true
  }
  const finishMoveConfirmation = (move: boolean) => {
    if (!moveBatch || !pendingMoveConfirmation) return
    if (
      currentLocationGeneration.current !== moveBatch.locationGeneration
      || currentControlsKey.current !== moveBatch.baseKey
    ) {
      confirmedMove.current = false
      setMoveBatch(null)
      setRestoreMoveConfirmationFocus(true)
      announceMove(t('recommend.moveCanceledNavigation'))
      return
    }
    confirmedMove.current = move
    if (!move) {
      setMoveBatch(null)
      setRestoreMoveConfirmationFocus(true)
      announceMove(t('recommend.moveBatchCanceled', {
        name: pendingMoveConfirmation.ingredient.name,
      }))
      return
    }
    const nextIndex = moveBatch.confirmationIndex + 1
    if (nextIndex < moveBatch.confirmations.length) {
      confirmedMove.current = false
      setMoveBatch({ ...moveBatch, confirmationIndex: nextIndex })
      return
    }
    const commit = commitControls(moveBatch.controls)
    if (commit.status !== 'committed') {
      confirmedMove.current = false
      return
    }
    setPendingMoveFocus({
      bucket: pendingMoveConfirmation.to,
      identity: pendingMoveConfirmation.identity,
    })
    setMoveBatch(null)
    announceMove(moveBatch.confirmations.length === 1
      ? t('recommend.moveSuccess', {
          name: pendingMoveConfirmation.ingredient.name,
          bucket: recommendationBucketLabel(pendingMoveConfirmation.to, t),
        })
      : t('recommend.moveBatchSuccess', {
          count: formatNumber(moveBatch.confirmations.length),
          bucket: recommendationBucketLabel(pendingMoveConfirmation.to, t),
        }))
  }

  return (
    <Page title={t('recommend.title')} intro={t('recommend.intro')}>
      {urlIssues.length > 0 && (
        <div className="notice" role="status">
          <span>{t(
            urlIssues.includes('canonicalDowngradeConflict')
              ? 'recommend.canonicalDowngradeConflict'
              : urlIssues.includes('canonicalDowngraded')
                ? 'recommend.canonicalDowngraded'
                : 'recommend.urlWarning',
          )}</span>
        </div>
      )}
      <form
        className="filter-panel"
        onSubmit={(event) => {
          event.preventDefault()
        }}
      >
        <p className="recommendation-share-note">{t('recommend.shareNote')}</p>
        <IngredientFilterInput
          bucket="preferred"
          label={t('recommend.hopeToUse')}
          values={filters.preferred}
          onAdd={(values) => queueAdditions('preferred', values)}
          onRemove={(identity) => {
            commitControls({
              ...filters,
              preferred: filters.preferred.filter((value) => ingredientIdentity(value) !== identity),
            })
          }}
          onMove={(identity, to) => commitMove({
            from: 'preferred',
            to,
            identity,
            ingredient: filters.preferred.find((value) =>
              ingredientIdentity(value) === identity
            )!,
          })}
          registerMoveButton={(identity, element) => {
            const key = `preferred:${identity}`
            if (element) moveButtonRefs.current.set(key, element)
            else moveButtonRefs.current.delete(key)
          }}
        />
        <IngredientFilterInput
          bucket="required"
          label={t('recommend.mustUse')}
          values={filters.required}
          onAdd={(values) => queueAdditions('required', values)}
          onRemove={(identity) => {
            commitControls({
              ...filters,
              required: filters.required.filter((value) => ingredientIdentity(value) !== identity),
            })
          }}
          onMove={(identity, to) => commitMove({
            from: 'required',
            to,
            identity,
            ingredient: filters.required.find((value) =>
              ingredientIdentity(value) === identity
            )!,
          })}
          registerMoveButton={(identity, element) => {
            const key = `required:${identity}`
            if (element) moveButtonRefs.current.set(key, element)
            else moveButtonRefs.current.delete(key)
          }}
        />
        <IngredientFilterInput
          bucket="excluded"
          label={t('recommend.doNotUse')}
          values={filters.excluded}
          onAdd={(values) => queueAdditions('excluded', values)}
          onRemove={(identity) => {
            commitControls({
              ...filters,
              excluded: filters.excluded.filter((value) => ingredientIdentity(value) !== identity),
            })
          }}
          onMove={(identity, to) => commitMove({
            from: 'excluded',
            to,
            identity,
            ingredient: filters.excluded.find((value) =>
              ingredientIdentity(value) === identity
            )!,
          })}
          registerMoveButton={(identity, element) => {
            const key = `excluded:${identity}`
            if (element) moveButtonRefs.current.set(key, element)
            else moveButtonRefs.current.delete(key)
          }}
        />
        <fieldset className="recommendation-scope">
          <legend>{t('recommend.scopeLegend')}</legend>
          <p className="field-note">{t('recommend.scopeHelp')}</p>
          {(['ANY', 'PANTRY_ONLY', 'PANTRY_OR_LIST'] as const).map((scope) => (
            <label className="choice-card" key={scope}>
              <input
                type="radio"
                name="ingredient-scope"
                value={scope}
                checked={filters.ingredientScope === scope}
                onChange={() => {
                  commitControls({ ...filters, ingredientScope: scope })
                }}
              />
              <span>
                <strong>{t(`recommend.scope.${scope}`)}</strong>
                <small>{t(`recommend.scope.${scope}.detail`)}</small>
              </span>
            </label>
          ))}
        </fieldset>
        <fieldset className="recommendation-sources">
          <legend>{t('recommend.sourceLegend')}</legend>
          <p className="field-note">{t('recommend.sourceHelp')}</p>
          {([
            ['includeFavoriteRecipes', 'sources.favoriteRecipes'],
            ['includeOwn', 'sources.own'],
            ['includeFavoriteAccounts', 'sources.favoriteAccounts'],
            ['includeOtherPublic', 'sources.otherPublic'],
          ] as const).map(([field, label]) => (
            <label className="check-row" key={field}>
              <input
                type="checkbox"
                checked={filters.sources[field]}
                onChange={(event) => {
                  commitControls({
                    ...filters,
                    sources: { ...filters.sources, [field]: event.target.checked },
                  })
                }}
              />
              {t(label)}
            </label>
          ))}
        </fieldset>
      </form>
      <p className="sr-only" aria-live="polite" aria-atomic="true">{resultStatus}</p>
      <p key={moveNotice?.id} className="sr-only" aria-live="polite" aria-atomic="true">
        {moveNotice?.message ?? ''}
      </p>
      {recommendations.status === 'loading' && recommendations.data.length > 0 && (
        <div className="status-banner">
          <span>{t('recommend.loading')}</span>
        </div>
      )}
      {recommendations.status === 'error' && (
        <div className="status-banner">
          <span>{t('recommend.failed')}</span>
          <button type="button" className="button secondary" onClick={retry}>
            {t('recommend.retry')}
          </button>
        </div>
      )}
      {verifyNames.length > 0 && (
        <div className="notice">
          <span>{t('recommend.checkCount', { count: formatNumber(verifyNames.length) })}</span>
          <button className="button secondary" onClick={() => setConfirmOpen(true)}>
            {t('recommend.checkPantry')}
          </button>
        </div>
      )}
      {recommendations.status === 'loading' && recommendations.data.length === 0 ? (
        <StatePanel title={t('common.loading')} detail={t('recommend.loading')} />
      ) : recommendations.status === 'error' && recommendations.data.length === 0 ? null
        : recommendations.status === 'success' && recommendations.data.length === 0 ? (
          <StatePanel title={t('common.emptyTitle')} detail={t('recommend.noMatches')} />
        ) : (
        <div className="recommendation-results">
          {(() => {
            const results = recommendations.data
            const hasDesiredIntent = activeControls.preferred.length > 0
              || results.some((result) => result.appliedControls.preferred.length > 0)
            const usesPreferred = hasDesiredIntent
              ? results.filter((result) => result.desiredMatches.length > 0)
              : []
            const other = hasDesiredIntent
              ? results.filter((result) => result.desiredMatches.length === 0)
              : results
            return (
              <>
                {usesPreferred.length > 0 && (
                  <section aria-labelledby="recommend-preferred">
                    <h2 id="recommend-preferred">{t('recommend.usesPreferred')}</h2>
                    <div className="card-list">{usesPreferred.map(renderRecommendationCard)}</div>
                  </section>
                )}
                {other.length > 0 && (
                  <section aria-labelledby="recommend-other">
                    <h2 id="recommend-other">
                      {t(hasDesiredIntent ? 'recommend.otherRecommendations' : 'recommend.results')}
                    </h2>
                    <div className="card-list">{other.map(renderRecommendationCard)}</div>
                  </section>
                )}
              </>
            )
          })()}
        </div>
      )}
      {shoppingNotice && (
        <p key={shoppingNotice.id} role="status" aria-live="polite">
          {t('shopping.addedMissing', { name: shoppingNotice.name })}
        </p>
      )}
      {addShopping.error && <ErrorText error={addShopping.error} />}
      <PantryConfirmation
        open={confirmOpen}
        onOpenChange={setConfirmOpen}
        names={verifyNames}
        onSaved={retry}
      />
      <Dialog.Root
        open={pendingMoveConfirmation !== null}
        onOpenChange={(open) => {
          if (!open && pendingMoveConfirmation) finishMoveConfirmation(false)
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="dialog-overlay" />
          {lastMoveConfirmation.current && (
            <Dialog.Content
              className="confirmation-sheet recommendation-move-confirmation"
              onCloseAutoFocus={(event) => {
                event.preventDefault()
                if (!confirmedMove.current) moveConfirmationRestoreFocus.current?.focus()
                confirmedMove.current = false
              }}
            >
              <Dialog.Title>{t('recommend.moveConfirmTitle')}</Dialog.Title>
              <Dialog.Description>
                {t('recommend.moveConfirmDescription', {
                  name: lastMoveConfirmation.current.ingredient.name,
                  from: recommendationBucketLabel(lastMoveConfirmation.current.from, t),
                  to: recommendationBucketLabel(lastMoveConfirmation.current.to, t),
                })}
              </Dialog.Description>
              <div className="dialog-actions">
                <button
                  type="button"
                  className="button ghost"
                  onClick={() => finishMoveConfirmation(false)}
                >
                  {t('common.cancel')}
                </button>
                <button
                  type="button"
                  className="button primary"
                  onClick={() => finishMoveConfirmation(true)}
                >
                  {t('recommend.confirmMove')}
                </button>
              </div>
            </Dialog.Content>
          )}
        </Dialog.Portal>
      </Dialog.Root>
    </Page>
  )
}

function IngredientFilterInput({
  bucket,
  label,
  values,
  onAdd,
  onRemove,
  onMove,
  registerMoveButton,
}: {
  bucket: RecommendationIngredientBucket
  label: string
  values: IngredientReference[]
  onAdd: (values: IngredientReference[]) => boolean
  onRemove: (identity: string) => void
  onMove: (identity: string, to: RecommendationIngredientBucket) => void
  registerMoveButton: (identity: string, element: HTMLButtonElement | null) => void
}) {
  const { t } = useI18n()
  const [draft, setDraft] = useState<IngredientReference>({ name: '', canonicalIngredientId: null })
  function add(next: IngredientReference) {
    const additions = splitIngredientList(next.name).map((name, index) => ({
      name,
      canonicalIngredientId: index === 0 ? next.canonicalIngredientId : null,
    }))
    if (onAdd(additions)) setDraft({ name: '', canonicalIngredientId: null })
  }
  return (
    <fieldset className="ingredient-filter">
      <legend>{label}</legend>
      <IngredientCombobox
        label={t('recommend.addToFilter', { label })}
        value={draft}
        onChange={setDraft}
        onCommit={add}
        onPasteList={(names) => {
          if (onAdd(names.map((name) => ({ name, canonicalIngredientId: null })))) {
            setDraft({ name: '', canonicalIngredientId: null })
          }
        }}
        placeholder={t('recommend.filterPlaceholder')}
      />
      {values.length > 0 && (
        <ul className="ingredient-chips" aria-label={t('recommend.filterList', { label })}>
          {values.map((value, index) => (
            <li key={`${ingredientIdentity(value)}-${index}`}>
              <span>{value.name}</span>
              <RecommendationIngredientMoveMenu
                bucket={bucket}
                ingredient={value}
                onMove={onMove}
                registerTrigger={registerMoveButton}
              />
              <button
                type="button"
                className="button ghost"
                aria-label={t('recommend.removeFromFilter', { name: value.name, label })}
                onClick={() => onRemove(ingredientIdentity(value))}
              >
                {t('common.remove')}
              </button>
            </li>
          ))}
        </ul>
      )}
    </fieldset>
  )
}

function RecommendationIngredientMoveMenu({
  bucket,
  ingredient,
  onMove,
  registerTrigger,
}: {
  bucket: RecommendationIngredientBucket
  ingredient: IngredientReference
  onMove: (identity: string, to: RecommendationIngredientBucket) => void
  registerTrigger: (identity: string, element: HTMLButtonElement | null) => void
}) {
  const { t } = useI18n()
  const menuId = useId()
  const containerRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement | null>(null)
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([])
  const [open, setOpen] = useState(false)
  const [activeIndex, setActiveIndex] = useState(0)
  const [position, setPosition] = useState<RecommendationMoveMenuPosition | null>(null)
  const identity = ingredientIdentity(ingredient)
  const targets = (['preferred', 'required', 'excluded'] as RecommendationIngredientBucket[])
    .filter((candidate) => candidate !== bucket)

  useEffect(() => {
    if (open) itemRefs.current[activeIndex]?.focus()
  }, [activeIndex, open])
  useEffect(() => {
    if (!open) return
    const updatePosition = () => {
      if (!triggerRef.current) return
      const placement = positionRecommendationMoveMenu(
        triggerRef.current.getBoundingClientRect(),
        recommendationMoveMenuViewport(),
      )
      if (placement.status === 'reference-hidden') {
        setOpen(false)
        triggerRef.current.focus({ preventScroll: true })
        return
      }
      setPosition(placement.position)
    }
    updatePosition()
    window.addEventListener('resize', updatePosition)
    window.addEventListener('scroll', updatePosition, true)
    window.visualViewport?.addEventListener('resize', updatePosition)
    window.visualViewport?.addEventListener('scroll', updatePosition)
    return () => {
      window.removeEventListener('resize', updatePosition)
      window.removeEventListener('scroll', updatePosition, true)
      window.visualViewport?.removeEventListener('resize', updatePosition)
      window.visualViewport?.removeEventListener('scroll', updatePosition)
    }
  }, [open])

  const closeAndRestore = () => {
    setOpen(false)
    triggerRef.current?.focus()
  }
  const openAt = (index: number) => {
    if (triggerRef.current) {
      const placement = positionRecommendationMoveMenu(
        triggerRef.current.getBoundingClientRect(),
        recommendationMoveMenuViewport(),
      )
      if (placement.status === 'reference-hidden') {
        triggerRef.current.focus({ preventScroll: true })
        return
      }
      setPosition(placement.position)
    }
    setActiveIndex(index)
    setOpen(true)
  }

  return (
    <div
      className="ingredient-move-menu"
      ref={containerRef}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false)
      }}
    >
      <button
        type="button"
        className="button ghost ingredient-move-trigger"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={t('recommend.moveIngredient', { name: ingredient.name })}
        ref={(element) => {
          triggerRef.current = element
          registerTrigger(identity, element)
        }}
        onClick={() => open ? closeAndRestore() : openAt(0)}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown') {
            event.preventDefault()
            openAt(0)
          } else if (event.key === 'ArrowUp') {
            event.preventDefault()
            openAt(targets.length - 1)
          } else if (event.key === 'Escape' && open) {
            event.preventDefault()
            closeAndRestore()
          }
        }}
      >
        {t('recommend.move')}
      </button>
      {open && (
        <div
          id={menuId}
          role="menu"
          className="ingredient-move-menu-items"
          data-placement={position?.placement}
          aria-label={t('recommend.moveMenu', { name: ingredient.name })}
          style={position ? {
            left: position.left,
            maxHeight: position.maxHeight,
            top: position.top,
            width: position.width,
          } : undefined}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
              event.preventDefault()
              const delta = event.key === 'ArrowDown' ? 1 : -1
              setActiveIndex((current) => (current + delta + targets.length) % targets.length)
            } else if (event.key === 'Home') {
              event.preventDefault()
              setActiveIndex(0)
            } else if (event.key === 'End') {
              event.preventDefault()
              setActiveIndex(targets.length - 1)
            } else if (event.key === 'Escape') {
              event.preventDefault()
              closeAndRestore()
            } else if (event.key === 'Tab') {
              setOpen(false)
            }
          }}
        >
          {targets.map((target, index) => (
            <button
              type="button"
              role="menuitem"
              className="ingredient-move-menu-item"
              key={target}
              ref={(element) => {
                itemRefs.current[index] = element
              }}
              onClick={() => {
                setOpen(false)
                onMove(identity, target)
              }}
            >
              {t('recommend.moveToBucket', { bucket: recommendationBucketLabel(target, t) })}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

function recommendationBucketLabel(
  bucket: RecommendationIngredientBucket,
  t: Translate,
): string {
  if (bucket === 'preferred') return t('recommend.hopeToUse')
  if (bucket === 'required') return t('recommend.mustUse')
  return t('recommend.doNotUse')
}

function recommendationMoveMenuViewport() {
  const viewport = window.visualViewport
  return {
    height: viewport?.height ?? window.innerHeight,
    offsetLeft: viewport?.offsetLeft ?? 0,
    offsetTop: viewport?.offsetTop ?? 0,
    width: viewport?.width ?? window.innerWidth,
  }
}

export function RecipesPage() {
  const { formatNumber, t } = useI18n()
  const search = useSearch({ strict: false }) as { scope?: 'mine' }
  const recipes = useQuery({ queryKey: ['recipes'], queryFn: api.recipes })
  const queryClient = useQueryClient()
  const favorite = useMutation({
    mutationFn: ({ id, next }: { id: string; next: boolean }) => api.favorite(id, next),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['recipes'] }),
  })
  return (
    <Page
      title={t('recipes.title')}
      intro={t('recipes.intro')}
      action={
        <Link className="button primary" to="/recipes/new">
          {t('recipes.new')}
        </Link>
      }
    >
      {search.scope === 'mine' && (
        <div className="active-filter" role="status">
          <span>{t('recipes.filterMine')}</span>
          <Link className="button ghost" to="/recipes" search={{}}>{t('common.clearFilter')}</Link>
        </div>
      )}
      <QueryState query={recipes} empty={t('recipes.empty')}>
        <div className="card-list">
          {recipes.data?.filter((recipe) => search.scope !== 'mine' || recipe.isOwner).map((recipe) => (
            <article className="recipe-card" key={recipe.id}>
              {recipe.coverImage && (
                <img
                  className="recipe-cover"
                  src={recipe.coverImage.url}
                  alt={recipe.coverImage.altText}
                  width={recipe.coverImage.width}
                  height={recipe.coverImage.height}
                />
              )}
              <div className="card-heading">
                <div>
                  <span className="eyebrow">
                    {recipe.visibility === 'public' ? t('common.public') : t('common.private')}
                  </span>
                  <h2>
                    <Link
                      className="recipe-title-link"
                      to="/recipes/$recipeId"
                      params={{ recipeId: recipe.id }}
                    >
                      {recipe.title}
                    </Link>
                  </h2>
                </div>
                <button
                  className="button ghost"
                  aria-pressed={recipe.isFavorite}
                  onClick={() => favorite.mutate({ id: recipe.id, next: !recipe.isFavorite })}
                >
                  {recipe.isFavorite ? t('recipes.saved') : t('recipes.save')}
                </button>
              </div>
              <p>{recipe.description || t('recipes.noDescription')}</p>
              <RecipeMetadata
                difficulty={recipe.difficulty}
                estimatedCalories={recipe.estimatedCalories}
                formatNumber={formatNumber}
                t={t}
              />
              <AuthorByline recipe={recipe} />
            </article>
          ))}
        </div>
      </QueryState>
    </Page>
  )
}

function AuthorByline({ recipe }: { recipe: Recipe }) {
  const { t } = useI18n()
  const name = recipe.authorNickname ?? t('common.anonymousUser')
  return (
    <p className="byline">
      {recipe.authorHandle ? (
        <Link to="/authors/$handle" params={{ handle: recipe.authorHandle }}>
          {t('common.byAuthor', { author: name })}
        </Link>
      ) : (
        t('common.byAuthor', { author: name })
      )}
    </p>
  )
}

function AuthorProfilePage() {
  const { handle } = authorRoute.useParams()
  const { formatNumber, t } = useI18n()
  const queryClient = useQueryClient()
  const [additionalRecipes, setAdditionalRecipes] = useState<Recipe[]>([])
  const [nextCursor, setNextCursor] = useState<string | null | undefined>(undefined)
  const me = useQuery({ queryKey: ['me'], queryFn: api.me, retry: false })
  const profile = useQuery({
    queryKey: ['author', handle],
    queryFn: () => api.authorProfile(handle),
  })
  useEffect(() => {
    setAdditionalRecipes([])
    setNextCursor(undefined)
  }, [handle])
  const favorites = useQuery({
    queryKey: ['favorite-accounts'],
    queryFn: api.favoriteAccounts,
    enabled: Boolean(me.data),
  })
  const visibleRecipes = useQuery({
    queryKey: ['recipes'],
    queryFn: api.recipes,
    enabled: Boolean(me.data),
  })
  const isFavorite = favorites.data?.some((author) => author.handle === handle) ?? false
  const isSelf = visibleRecipes.data?.some(
    (recipe) => recipe.isOwner && recipe.authorHandle === handle,
  ) ?? false
  const favorite = useMutation({
    mutationFn: (next: boolean) => api.favoriteAccount(handle, next),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['favorite-accounts'] }),
        queryClient.invalidateQueries({ queryKey: ['recipes'] }),
        queryClient.invalidateQueries({ queryKey: ['recommendations'] }),
      ])
    },
  })
  const loadMore = useMutation({
    mutationFn: (cursor: string) => api.authorProfile(handle, cursor),
    onSuccess: (page) => {
      setAdditionalRecipes((current) => [...current, ...page.recipes])
      setNextCursor(page.nextCursor)
    },
  })
  const recipes = [...(profile.data?.recipes ?? []), ...additionalRecipes]
  const pageCursor = nextCursor === undefined ? profile.data?.nextCursor : nextCursor
  return (
    <Page
      title={profile.data?.nickname ?? t('common.anonymousUser')}
      intro={t('author.intro')}
      action={me.data && !isSelf ? (
        <button
          type="button"
          className="button secondary"
          aria-pressed={isFavorite}
          disabled={favorite.isPending}
          onClick={() => favorite.mutate(!isFavorite)}
        >
          {isFavorite ? t('author.unfavorite') : t('author.favorite')}
        </button>
      ) : undefined}
    >
      <QueryState query={profile} empty={t('author.empty')}>
        <div className="card-list">
          {recipes.map((recipe) => (
            <article className="recipe-card" key={recipe.id}>
              <h2>
                {me.data ? (
                  <Link
                    className="recipe-title-link"
                    to="/recipes/$recipeId"
                    params={{ recipeId: recipe.id }}
                  >
                    {recipe.title}
                  </Link>
                ) : recipe.title}
              </h2>
              <p>{recipe.description || t('recipes.noDescription')}</p>
              <RecipeMetadata
                difficulty={recipe.difficulty}
                estimatedCalories={recipe.estimatedCalories}
                formatNumber={formatNumber}
                t={t}
              />
            </article>
          ))}
        </div>
      </QueryState>
      {pageCursor && (
        <button
          type="button"
          className="button secondary"
          disabled={loadMore.isPending}
          onClick={() => loadMore.mutate(pageCursor)}
        >
          {loadMore.isPending ? t('author.loadingMore') : t('author.loadMore')}
        </button>
      )}
      {favorite.error && <ErrorText error={favorite.error} />}
      {loadMore.error && <ErrorText error={loadMore.error} />}
    </Page>
  )
}

function RecipeDetailPage() {
  const { formatNumber, t } = useI18n()
  const { recipeId } = recipeRoute.useParams()
  const recipe = useQuery({ queryKey: ['recipe', recipeId], queryFn: () => api.recipe(recipeId) })
  return (
    <Page title={recipe.data?.title ?? t('recipe.fallbackTitle')} intro={recipe.data?.description ?? ''}>
      <QueryState query={recipe} empty={t('recipe.notFound')}>
        {recipe.data && (
          <RecipeDetailContent recipe={recipe.data} />
        )}
      </QueryState>
    </Page>
  )
}

export function RecipeDetailContent({ recipe }: { recipe: Recipe }) {
  const { formatNumber, t } = useI18n()
  return (
          <article className="detail-card">
            <div className="card-heading">
              <span className="status verify">
                {recipe.visibility === 'public' ? t('common.public') : t('common.private')}
              </span>
              {recipe.isOwner && (
                <Link className="button secondary" to="/recipes/$recipeId/edit" params={{ recipeId: recipe.id }}>
                  {t('common.edit')}
                </Link>
              )}
            </div>
            <AuthorByline recipe={recipe} />
            <RecipeMetadata
              difficulty={recipe.difficulty}
              estimatedCalories={recipe.estimatedCalories}
              formatNumber={formatNumber}
              t={t}
            />
            {recipe.coverImage && (
              <figure className="recipe-cover-detail">
                <img
                  src={recipe.coverImage.url}
                  alt={recipe.coverImage.altText}
                  width={recipe.coverImage.width}
                  height={recipe.coverImage.height}
                />
                {(recipe.coverImage.sourceAttribution || recipe.coverImage.sourceLicense) && (
                  <figcaption>
                    {recipe.coverImage.sourceAttribution}
                    {recipe.coverImage.sourceAttribution && recipe.coverImage.sourceLicense && ' · '}
                    {recipe.coverImage.sourceLicense}
                  </figcaption>
                )}
                {recipe.coverImage.sourceUrl && (
                  <a href={recipe.coverImage.sourceUrl} rel="noreferrer" target="_blank">
                    {t('recipe.coverSourceLink')}
                  </a>
                )}
              </figure>
            )}
            {recipe.description && <SafeMarkdown value={recipe.description} />}
            <h2>{t('recipe.ingredients')}</h2>
            <RecipeIngredientList
              ingredients={recipe.ingredients}
              ingredientModel={recipe.ingredientModel}
              ingredientGroups={recipe.ingredientGroups}
            />
            <h2>{t('recipe.method')}</h2>
            {recipe.instructions ? (
              <SafeMarkdown value={recipe.instructions} />
            ) : (
              <p>{t('recipe.noInstructions')}</p>
            )}
            <RecipeSourceSection source={recipe.source} />
            {recipe.isOwner && recipe.moderationState === 'restricted' && (
              <RestrictedOwnerPanel recipe={recipe} />
            )}
            {!recipe.isOwner && recipe.visibility === 'public' && recipe.moderationState === 'clear' && (
              <ReportRecipeForm recipeId={recipe.id} />
            )}
            {recipe.isOwner && <RevisionReview recipeId={recipe.id} />}
          </article>
  )
}

function ReportRecipeForm({ recipeId }: { recipeId: string }) {
  const { t } = useI18n()
  const [reason, setReason] = useState('spam')
  const [details, setDetails] = useState('')
  const report = useMutation({
    mutationFn: () => api.reportRecipe(recipeId, { reason, details }),
  })
  return (
    <section className="settings-section" aria-labelledby={`report-${recipeId}`}>
      <h2 id={`report-${recipeId}`}>{t('moderation.reportTitle')}</h2>
      <p>{t('moderation.reportHelp')}</p>
      <label>
        {t('moderation.reason')}
        <select value={reason} onChange={(event) => setReason(event.target.value)}>
          {['spam', 'unsafe_or_harmful', 'harassment_or_hate', 'privacy',
            'copyright_or_attribution', 'misleading', 'other'].map((value) => (
            <option key={value} value={value}>{t(`moderation.reason.${value}` as never)}</option>
          ))}
        </select>
      </label>
      <label>
        {t('moderation.details')}
        <textarea
          value={details}
          maxLength={500}
          rows={4}
          onChange={(event) => setDetails(event.target.value)}
        />
      </label>
      <button
        type="button"
        className="button secondary"
        disabled={report.isPending || report.isSuccess}
        onClick={() => report.mutate()}
      >
        {report.isPending ? t('moderation.submitting') : t('moderation.submitReport')}
      </button>
      {report.isSuccess && <p role="status">{t('moderation.reportSubmitted')}</p>}
      {report.error && <ErrorText error={report.error} />}
    </section>
  )
}

function RestrictedOwnerPanel({ recipe }: { recipe: Recipe }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [reason, setReason] = useState('')
  const appeals = useQuery({
    queryKey: ['appeals', recipe.id],
    queryFn: () => api.appeals(recipe.id),
  })
  const create = useMutation({
    mutationFn: () => api.createAppeal(recipe.id, reason),
    onSuccess: async () => {
      setReason('')
      await queryClient.invalidateQueries({ queryKey: ['appeals', recipe.id] })
    },
  })
  const active = appeals.data?.some((appeal) => appeal.status === 'active')
  return (
    <section className="moderation-banner" aria-labelledby={`restricted-${recipe.id}`}>
      <h2 id={`restricted-${recipe.id}`}>{t('moderation.restrictedTitle')}</h2>
      <p>{t('moderation.restrictedHelp')}</p>
      <p><strong>{t('moderation.publicationIntent')}:</strong> {t('common.public')}</p>
      {!active && (
        <>
          <label>
            {t('moderation.appealReason')}
            <textarea
              value={reason}
              maxLength={1000}
              rows={5}
              onChange={(event) => setReason(event.target.value)}
            />
          </label>
          <button
            type="button"
            className="button secondary"
            disabled={!reason.trim() || create.isPending}
            onClick={() => create.mutate()}
          >
            {t('moderation.submitAppeal')}
          </button>
        </>
      )}
      {active && <p role="status">{t('moderation.appealActive')}</p>}
      {create.error && <ErrorText error={create.error} />}
    </section>
  )
}

function RecipeSourceSection({ source }: { source: Recipe['source'] }) {
  const { t } = useI18n()
  if (!source || (!source.url
    && !source.title
    && !source.author
    && !source.license
    && !source.attribution
    && source.warnings.length === 0)) return null
  const sourceUrl = safeHttpUrl(source.url)
  return (
    <section className="source-note recipe-source-section" aria-labelledby="recipe-source-heading">
      <h2 id="recipe-source-heading">{t('recipe.importedSource')}</h2>
      <dl className="source-details-list">
        {(source.url || source.title) && (
          <div>
            <dt>{t('recipe.sourceDetails')}</dt>
            <dd>
              {sourceUrl ? (
                <a href={sourceUrl} rel="noreferrer" target="_blank">
                  {source.title ?? sourceUrl}
                </a>
              ) : (source.title ?? source.url)}
            </dd>
          </div>
        )}
        {source.author && (
          <div>
            <dt>{t('recipe.sourceAuthor')}</dt>
            <dd>{source.author}</dd>
          </div>
        )}
        {source.license && (
          <div>
            <dt>{t('recipe.sourceLicense')}</dt>
            <dd>{source.license}</dd>
          </div>
        )}
        {source.attribution && (
          <div>
            <dt>{t('recipe.sourceAttribution')}</dt>
            <dd>{source.attribution}</dd>
          </div>
        )}
      </dl>
      {(source.license || source.attribution) && (
        <small>{t('recipe.sourceLicenseUnverified')}</small>
      )}
      {source.warnings.length > 0 && (
        <ul className="plain-list">
          {source.warnings.map((warning) => <li key={warning}>{warning}</li>)}
        </ul>
      )}
    </section>
  )
}

function safeHttpUrl(value: string | null): string | null {
  if (!value) return null
  try {
    const url = new URL(value)
    return (url.protocol === 'http:' || url.protocol === 'https:') && Boolean(url.hostname)
      ? url.toString()
      : null
  } catch {
    return null
  }
}

function FavoriteAccountsSettings() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const favorites = useQuery({ queryKey: ['favorite-accounts'], queryFn: api.favoriteAccounts })
  const remove = useMutation({
    mutationFn: (handle: string) => api.favoriteAccount(handle, false),
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['favorite-accounts'] }),
        queryClient.invalidateQueries({ queryKey: ['recommendations'] }),
      ])
    },
  })
  return (
    <section className="settings-section">
      <h2>{t('author.favoriteAccounts')}</h2>
      <QueryState query={favorites} empty={t('author.noFavorites')}>
        <ul className="plain-list">
          {favorites.data?.map((author) => (
            <li key={author.handle} className="card-heading">
              <Link to="/authors/$handle" params={{ handle: author.handle }}>
                {author.nickname ?? t('common.anonymousUser')}
              </Link>
              <button
                type="button"
                className="button ghost"
                disabled={remove.isPending}
                onClick={() => remove.mutate(author.handle)}
              >
                {t('author.unfavorite')}
              </button>
            </li>
          ))}
        </ul>
      </QueryState>
      {remove.error && <ErrorText error={remove.error} />}
    </section>
  )
}

function RevisionReview({ recipeId }: { recipeId: string }) {
  const { formatNumber, t } = useI18n()
  const queryClient = useQueryClient()
  const revisions = useQuery({
    queryKey: ['recipe-revisions', recipeId],
    queryFn: () => api.recipeRevisions(recipeId),
  })
  const accept = useMutation({
    mutationFn: api.acceptRecipeRevision,
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['recipe', recipeId] }),
        queryClient.invalidateQueries({ queryKey: ['recipes'] }),
        queryClient.invalidateQueries({ queryKey: ['recipe-revisions', recipeId] }),
      ])
    },
  })
  const reject = useMutation({
    mutationFn: api.rejectRecipeRevision,
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ['recipe-revisions', recipeId] }),
  })
  if (revisions.isLoading) return <p role="status">{t('common.loading')}</p>
  if (revisions.error) return <ErrorText error={revisions.error} />
  if (!revisions.data?.length) return null
  return (
    <section className="revision-review" aria-labelledby={`recipe-revisions-${recipeId}`}>
      <h2 id={`recipe-revisions-${recipeId}`}>{t('recipe.pendingRevision')}</h2>
      <p>{t('recipe.pendingRevisionDetail')}</p>
      {revisions.data.map((revision) => (
        <article className="source-note" key={revision.id}>
          <h3>{revision.title}</h3>
          <p>{revision.description}</p>
          <RecipeMetadata
            difficulty={revision.difficulty}
            estimatedCalories={revision.estimatedCalories}
            formatNumber={formatNumber}
            t={t}
          />
          {(revision.sourceUrl
            || revision.sourceTitle
            || revision.sourceAuthor
            || revision.sourceLicense
            || revision.sourceAttribution) && (
            <div className="source-note">
              <strong>{t('recipe.sourceDetails')}</strong>
              {revision.sourceUrl ? (
                <a href={revision.sourceUrl} rel="noreferrer">
                  {revision.sourceTitle ?? revision.sourceUrl}
                </a>
              ) : revision.sourceTitle ? <span>{revision.sourceTitle}</span> : null}
              {revision.sourceAuthor && (
                <span>{t('recipe.sourceBy', { author: revision.sourceAuthor })}</span>
              )}
              {revision.sourceAttribution && <p>{revision.sourceAttribution}</p>}
              {revision.sourceLicense && (
                <p>{t('recipe.sourceLicenseValue', { license: revision.sourceLicense })}</p>
              )}
              <small>{t('recipe.sourceLicenseUnverified')}</small>
            </div>
          )}
          <p>{t('recipe.revisionImageAction', {
            action: t(
              revision.imageAction === 'replace'
                ? 'recipe.imageAction.replace'
                : revision.imageAction === 'delete'
                  ? 'recipe.imageAction.delete'
                  : 'recipe.imageAction.keep',
            ),
          })}</p>
          {revision.imageUrl && (
            <figure className="recipe-cover-detail">
              <img src={revision.imageUrl} alt={revision.imageAltText ?? ''} />
              {(revision.imageSourceAttribution || revision.imageSourceLicense) && (
                <figcaption>
                  {revision.imageSourceAttribution}
                  {revision.imageSourceAttribution && revision.imageSourceLicense && ' · '}
                  {revision.imageSourceLicense}
                </figcaption>
              )}
              {revision.imageSourceUrl && (
                <a href={revision.imageSourceUrl} rel="noreferrer" target="_blank">
                  {t('recipe.coverSourceLink')}
                </a>
              )}
            </figure>
          )}
          <details>
            <summary>{t('recipe.reviewRevision')}</summary>
            <h4>{t('recipe.ingredients')}</h4>
            <RecipeIngredientList
              ingredients={revision.ingredients}
              ingredientModel={revision.ingredientModel}
              ingredientGroups={revision.ingredientGroups}
            />
            <h4>{t('recipe.method')}</h4>
            <SafeMarkdown value={revision.instructions} />
          </details>
          <div className="button-row">
            <button
              type="button"
              className="button primary"
              disabled={accept.isPending || reject.isPending}
              onClick={() => accept.mutate(revision.id)}
            >
              {t('recipe.acceptRevision')}
            </button>
            <button
              type="button"
              className="button ghost"
              disabled={accept.isPending || reject.isPending}
              onClick={() => {
                if (window.confirm(t('recipe.rejectRevisionConfirm'))) reject.mutate(revision.id)
              }}
            >
              {t('recipe.rejectRevision')}
            </button>
          </div>
        </article>
      ))}
      {(accept.error || reject.error) && <ErrorText error={accept.error ?? reject.error} />}
    </section>
  )
}

function EditRecipePage() {
  const { t } = useI18n()
  const { recipeId } = editRecipeRoute.useParams()
  const recipe = useQuery({ queryKey: ['recipe', recipeId], queryFn: () => api.recipe(recipeId) })
  return (
    <Page title={t('recipe.editTitle')} intro={t('recipe.editIntro')}>
      <QueryState query={recipe} empty={t('recipe.notFound')}>
        {recipe.data && recipe.data.isOwner ? (
          <RecipeEditor recipe={recipe.data} />
        ) : recipe.data ? (
          <StatePanel title={t('recipe.ownerRequired')} detail={t('recipe.ownerRequiredDetail')} />
        ) : null}
      </QueryState>
    </Page>
  )
}

export function recipeSchema(t: Translate) {
  return z.object({
  title: z.string().trim().min(1, t('recipe.titleRequired')),
  description: z.string(),
  instructions: z.string(),
  visibility: z.enum(['private', 'public']),
  difficulty: z.custom<number | null>(
    (value) => value === null
      || (typeof value === 'number' && Number.isInteger(value) && value >= 1 && value <= 5),
    t('recipe.difficultyInvalid'),
  ),
  estimatedCalories: z.custom<number | null>(
    (value) => value === null
      || (typeof value === 'number' && Number.isInteger(value) && value >= 1 && value <= 100000),
    t('recipe.estimatedCaloriesInvalid'),
  ),
  sourceUrl: z.string().max(2048).refine((value) => {
    if (!value.trim()) return true
    try {
      const url = new URL(value)
      return (url.protocol === 'http:' || url.protocol === 'https:') && Boolean(url.hostname)
    } catch {
      return false
    }
  }, t('recipe.sourceUrlInvalid')),
  sourceTitle: z.string().max(200),
  sourceAuthor: z.string().max(200),
  sourceLicense: z.string().max(200),
  sourceAttribution: z.string().max(1000),
  ingredientGroups: z.array(z.object({
    id: z.string().min(1),
    optional: z.boolean(),
    note: z.string().max(240, t('recipe.groupNoteTooLong')),
    alternatives: z.array(z.object({
      id: z.string().min(1),
      name: z.string().trim().min(1, t('recipe.ingredientRequired')).max(120),
      quantityNote: z.string().max(240),
      canonicalIngredientId: z.string().nullable(),
    })).min(1).max(8, t('recipe.alternativeLimit')),
  })).min(1).max(100, t('recipe.groupLimit')).superRefine((groups, context) => {
    if (groups.reduce((total, group) => total + group.alternatives.length, 0) > 100) {
      context.addIssue({
        code: z.ZodIssueCode.custom,
        message: t('recipe.totalAlternativeLimit'),
      })
    }
    groups.forEach((group, groupIndex) => {
      const seen = new Set<string>()
      group.alternatives.forEach((alternative, alternativeIndex) => {
        const identity = ingredientIdentity(alternative)
        if (seen.has(identity)) {
          context.addIssue({
            code: z.ZodIssueCode.custom,
            path: [groupIndex, 'alternatives', alternativeIndex, 'name'],
            message: t('recipe.duplicateIngredient'),
          })
        } else {
          seen.add(identity)
        }
      })
    })
  }),
  })
}
export type RecipeForm = z.infer<ReturnType<typeof recipeSchema>>

function difficultyLabel(value: number, t: Translate): string {
  const key = `recipe.difficulty${value}` as
    | 'recipe.difficulty1'
    | 'recipe.difficulty2'
    | 'recipe.difficulty3'
    | 'recipe.difficulty4'
    | 'recipe.difficulty5'
  return t(key, { value })
}

function RecipeMetadata({
  difficulty,
  estimatedCalories,
  formatNumber,
  t,
}: {
  difficulty: number | null
  estimatedCalories: number | null
  formatNumber: (value: number) => string
  t: Translate
}) {
  if (difficulty === null && estimatedCalories === null) return null
  return (
    <dl className="recipe-metadata">
      {difficulty !== null && (
        <div>
          <dt>{t('recipe.difficulty')}</dt>
          <dd>{difficultyLabel(difficulty, t)}</dd>
        </div>
      )}
      {estimatedCalories !== null && (
        <div>
          <dt>{t('recipe.estimatedCalories')}</dt>
          <dd>{t('recipe.estimatedCaloriesValue', { value: formatNumber(estimatedCalories) })}</dd>
        </div>
      )}
    </dl>
  )
}

export function pantryStatusForIngredient(
  ingredient: IngredientReference & { quantityNote?: string },
  pantry: PantryItem[],
): 'available' | 'out' | 'unsure' | 'verify' | null {
  const normalized = normalizeIngredient(ingredient.name)
  const item = pantry.find((candidate) => {
    if (ingredient.canonicalIngredientId) {
      return candidate.canonicalIngredientId === ingredient.canonicalIngredientId
    }
    return normalizeIngredient(candidate.name) === normalized
      || candidate.aliases?.some((alias) => normalizeIngredient(alias) === normalized) === true
  })
  if (!item) return null
  if (item.status === 'out') return 'out'
  if (item.status === 'unknown') return 'unsure'
  if (item.expiresOn && item.expiresOn <= localDateValue(new Date())) return 'verify'
  return 'available'
}

export function pantryStatusForGroup(
  alternatives: IngredientReference[],
  pantry: PantryItem[],
): 'available' | 'out' | 'unsure' | 'verify' | null {
  const statuses = alternatives.map((alternative) => pantryStatusForIngredient(alternative, pantry))
  if (statuses.includes('available')) return 'available'
  if (statuses.includes('verify')) return 'verify'
  if (statuses.includes('unsure')) return 'unsure'
  if (statuses.includes('out')) return 'out'
  return null
}

export function RecipeIngredientList({
  ingredients,
  ingredientModel,
  ingredientGroups,
}: Pick<Recipe, 'ingredients' | 'ingredientModel' | 'ingredientGroups'>) {
  const { t } = useI18n()
  const pantry = useQuery({ queryKey: ['pantry'], queryFn: api.pantry })
  const groups = recipeIngredientGroups({ ingredients, ingredientModel, ingredientGroups })
  return (
    <ul className="recipe-ingredient-list">
      {groups.map((group) => {
        const status = pantry.data
          ? pantryStatusForGroup(group.alternatives, pantry.data)
          : null
        const statusKey = status
          ? `recipe.pantryStatus.${status}` as
            | 'recipe.pantryStatus.available'
            | 'recipe.pantryStatus.out'
            | 'recipe.pantryStatus.unsure'
            | 'recipe.pantryStatus.verify'
          : null
        return (
          <li key={group.id} className="ingredient-group-display">
            <div className="ingredient-group-alternatives">
              {group.alternatives.map((alternative, index) => (
                <span key={alternative.id}>
                  {index > 0 && <span className="ingredient-or">{t('recipe.or')}</span>}
                  <span>{alternative.name}{alternative.quantityNote && <> — {alternative.quantityNote}</>}</span>
                </span>
              ))}
              {group.optional && <span className="optional-badge">{t('recipe.optional')}</span>}
              {group.note && <small>{group.note}</small>}
            </div>
            {statusKey && (
              <span
                className={`pantry-status-badge pantry-status-${status}`}
                aria-label={t('recipe.pantryStatusLabel', {
                  name: group.alternatives.map((alternative) => alternative.name).join(` ${t('recipe.or')} `),
                  status: t(statusKey),
                })}
              >
                {t(statusKey)}
              </span>
            )}
          </li>
        )
      })}
    </ul>
  )
}

export function RecipeMetadataFields({ form }: { form: UseFormReturn<RecipeForm> }) {
  const { t } = useI18n()
  return (
    <fieldset>
      <legend>{t('recipe.optionalDetails')}</legend>
      <label>
        {t('recipe.difficulty')}
        <select
          {...form.register('difficulty', {
            setValueAs: (value) => (value == null || value === '' ? null : Number(value)),
          })}
          aria-invalid={Boolean(form.formState.errors.difficulty)}
          aria-describedby={form.formState.errors.difficulty ? 'recipe-difficulty-error' : undefined}
        >
          <option value="">{t('recipe.notSet')}</option>
          {[1, 2, 3, 4, 5].map((value) => (
            <option value={value} key={value}>
              {difficultyLabel(value, t)}
            </option>
          ))}
        </select>
        <FieldError id="recipe-difficulty-error" message={form.formState.errors.difficulty?.message} />
      </label>
      <label>
        {t('recipe.estimatedCalories')}
        <input
          type="text"
          inputMode="numeric"
          {...form.register('estimatedCalories', {
            setValueAs: (value) => {
              if (value == null) return null
              const normalized = String(value).trim()
              if (!normalized) return null
              return /^\d+$/.test(normalized) ? Number(normalized) : Number.NaN
            },
          })}
          aria-invalid={Boolean(form.formState.errors.estimatedCalories)}
          aria-describedby={
            form.formState.errors.estimatedCalories
              ? 'recipe-estimated-calories-error recipe-estimated-calories-help'
              : 'recipe-estimated-calories-help'
          }
        />
        <small id="recipe-estimated-calories-help">{t('recipe.estimatedCaloriesHelp')}</small>
        <FieldError
          id="recipe-estimated-calories-error"
          message={form.formState.errors.estimatedCalories?.message}
        />
      </label>
    </fieldset>
  )
}

export function RecipeProvenanceFields({ form }: { form: UseFormReturn<RecipeForm> }) {
  const { t } = useI18n()
  return (
    <details className="source-note">
      <summary>{t('recipe.sourceDetails')}</summary>
      <p>{t('recipe.sourceDetailsHelp')}</p>
      <label>
        {t('recipe.sourceUrl')}
        <input
          type="url"
          maxLength={2048}
          {...form.register('sourceUrl')}
          aria-invalid={Boolean(form.formState.errors.sourceUrl)}
          aria-describedby={form.formState.errors.sourceUrl ? 'recipe-source-url-error' : undefined}
        />
        <FieldError id="recipe-source-url-error" message={form.formState.errors.sourceUrl?.message} />
      </label>
      <label>
        {t('recipe.sourceTitle')}
        <input maxLength={200} {...form.register('sourceTitle')} />
      </label>
      <label>
        {t('recipe.sourceAuthor')}
        <input maxLength={200} {...form.register('sourceAuthor')} />
      </label>
      <label>
        {t('recipe.sourceLicense')}
        <input maxLength={200} {...form.register('sourceLicense')} />
      </label>
      <label>
        {t('recipe.sourceAttribution')}
        <textarea rows={3} maxLength={1000} {...form.register('sourceAttribution')} />
      </label>
      <small>{t('recipe.sourceLicenseUnverified')}</small>
    </details>
  )
}

export function RecipeEditor({ recipe }: { recipe?: Recipe }) {
  const navigate = useNavigate()
  return (
    <RecipeEditorForm
      recipe={recipe}
      onSaved={(id) => navigate({ to: '/recipes/$recipeId', params: { recipeId: id } })}
    />
  )
}

function recipeMatchesForm(recipe: Recipe, value: RecipeForm): boolean {
  return recipe.title === value.title
    && recipe.description === value.description
    && recipe.instructions === value.instructions
    && recipe.visibility === value.visibility
    && recipe.difficulty === value.difficulty
    && recipe.estimatedCalories === value.estimatedCalories
    && (recipe.source?.url ?? '') === value.sourceUrl.trim()
    && (recipe.source?.title ?? '') === value.sourceTitle.trim()
    && (recipe.source?.author ?? '') === value.sourceAuthor.trim()
    && (recipe.source?.license ?? '') === value.sourceLicense.trim()
    && (recipe.source?.attribution ?? '') === value.sourceAttribution.trim()
    && JSON.stringify(normalizeIngredientGroupOrder(recipeIngredientGroups(recipe)))
      === JSON.stringify(normalizeIngredientGroupOrder(value.ingredientGroups))
}

function coverMetadataMatches(recipe: Recipe, metadata: CoverMetadata): boolean {
  const cover = recipe.coverImage
  return cover !== null
    && cover.altText === metadata.altText
    && (cover.sourceUrl ?? '') === metadata.sourceUrl
    && (cover.sourceLicense ?? '') === metadata.sourceLicense
    && (cover.sourceAttribution ?? '') === metadata.sourceAttribution
}

function coverIsUnchanged(current: Recipe, original: Recipe): boolean {
  return current.coverImage?.id === original.coverImage?.id
    && current.coverImage?.version === original.coverImage?.version
}

function newCoverMutationKey(): string {
  return `${Math.floor(Date.now() / 1000)}.${crypto.randomUUID()}`
}

async function persistCoverMutation({
  saved,
  original,
  coverFile,
  removeCover,
  coverMetadata,
  expectedVersion,
  reconcileVersion,
  idempotencyKey,
  acknowledgeVersion,
}: {
  saved: Recipe
  original?: Recipe
  coverFile: File | null
  removeCover: boolean
  coverMetadata: CoverMetadata
  expectedVersion: number
  reconcileVersion?: number
  idempotencyKey: string
  acknowledgeVersion: (version: number) => void
}): Promise<Recipe> {
  if (coverFile) {
    const result = await api.uploadRecipeCover(
      saved.id,
      coverFile,
      coverMetadata,
      expectedVersion,
      idempotencyKey,
    )
    if (reconcileVersion !== undefined && result.version !== reconcileVersion) {
      throw new ApiError('Recipe changed after the cover operation completed', 409, 'conflict')
    }
    acknowledgeVersion(result.version)
    return { ...saved, version: result.version }
  }
  if (removeCover && original?.coverImage) {
    const result = await api.deleteRecipeCover(saved.id, expectedVersion, idempotencyKey)
    if (reconcileVersion !== undefined && result.version !== reconcileVersion) {
      throw new ApiError('Recipe changed after the cover operation completed', 409, 'conflict')
    }
    acknowledgeVersion(result.version)
    return { ...saved, version: result.version, coverImage: null }
  }
  if (original?.coverImage && !coverMetadataMatches(original, coverMetadata)) {
    const result = await api.updateRecipeCoverMetadata(
      saved.id,
      coverMetadata,
      expectedVersion,
      idempotencyKey,
    )
    if (reconcileVersion !== undefined && result.version !== reconcileVersion) {
      throw new ApiError('Recipe changed after the cover operation completed', 409, 'conflict')
    }
    acknowledgeVersion(result.version)
    return { ...saved, version: result.version }
  }
  return saved
}

export async function persistRecipeEdit({
  value,
  recipe,
  acknowledgedVersion,
  coverFile,
  removeCover,
  coverMetadata,
  coverMutationKey,
  acknowledgeVersion,
}: {
  value: RecipeForm
  recipe?: Recipe
  acknowledgedVersion?: number
  coverFile: File | null
  removeCover: boolean
  coverMetadata: CoverMetadata
  coverMutationKey: string
  acknowledgeVersion: (version: number) => void
}): Promise<Recipe> {
  let saved: Recipe
  const payload = {
    ...value,
    ingredientModel: 'groups-v1' as const,
    ingredientGroups: normalizeIngredientGroupOrder(value.ingredientGroups),
    sourceUrl: value.sourceUrl.trim() || null,
    sourceTitle: value.sourceTitle.trim() || null,
    sourceAuthor: value.sourceAuthor.trim() || null,
    sourceLicense: value.sourceLicense.trim() || null,
    sourceAttribution: value.sourceAttribution.trim() || null,
  }
  if (!recipe) {
    saved = await api.saveRecipe(payload)
    acknowledgeVersion(saved.version)
  } else {
    const current = await api.recipe(recipe.id)
    if (current.version !== acknowledgedVersion) {
      if (!recipeMatchesForm(current, value)) {
        throw new ApiError('Recipe changed while this editor was open', 409, 'conflict')
      }
      if (!coverIsUnchanged(current, recipe)) {
        const hasCoverMutation = Boolean(
          coverFile
          || (removeCover && recipe.coverImage)
          || (recipe.coverImage && !coverMetadataMatches(recipe, coverMetadata)),
        )
        if (acknowledgedVersion === undefined || !hasCoverMutation) {
          throw new ApiError('Recipe cover changed while this editor was open', 409, 'conflict')
        }
        return persistCoverMutation({
          saved: current,
          original: recipe,
          coverFile,
          removeCover,
          coverMetadata,
          expectedVersion: acknowledgedVersion,
          reconcileVersion: current.version,
          idempotencyKey: coverMutationKey,
          acknowledgeVersion,
        })
      }
      acknowledgeVersion(current.version)
    }
    if (recipeMatchesForm(current, value)) {
      saved = current
    } else {
      saved = await api.saveRecipe(payload, recipe.id, current.version)
      acknowledgeVersion(saved.version)
    }
  }
  return persistCoverMutation({
    saved,
    original: recipe,
    coverFile,
    removeCover,
    coverMetadata,
    expectedVersion: saved.version,
    idempotencyKey: coverMutationKey,
    acknowledgeVersion,
  })
}

function IngredientGroupEditor({
  form,
  groupIndex,
  canRemove,
  onRemove,
  onAppendGroups,
}: {
  form: UseFormReturn<RecipeForm>
  groupIndex: number
  canRemove: boolean
  onRemove: () => void
  onAppendGroups: (names: string[]) => void
}) {
  const { t } = useI18n()
  const alternatives = useFieldArray({
    control: form.control,
    name: `ingredientGroups.${groupIndex}.alternatives`,
  })
  return (
    <fieldset className="ingredient-group-card">
      <legend>{t('recipe.ingredientGroupNumber', { number: groupIndex + 1 })}</legend>
      <div className="ingredient-group-options">
        <label className="check-row">
          <input type="checkbox" {...form.register(`ingredientGroups.${groupIndex}.optional`)} />
          {t('recipe.optionalIngredient')}
        </label>
        <label>
          {t('recipe.groupNote')}
          <input
            maxLength={240}
            {...form.register(`ingredientGroups.${groupIndex}.note`)}
            placeholder={t('recipe.groupNotePlaceholder')}
          />
        </label>
      </div>
      <div className="alternative-list">
        {alternatives.fields.map((field, alternativeIndex) => {
          const error = form.formState.errors.ingredientGroups?.[groupIndex]
            ?.alternatives?.[alternativeIndex]?.name
          const errorId = `recipe-group-${groupIndex}-alternative-${alternativeIndex}-error`
          return (
            <div className="ingredient-row" key={field.id}>
              <div>
                <IngredientCombobox
                  label={t('recipe.alternativeNumber', {
                    group: groupIndex + 1,
                    number: alternativeIndex + 1,
                  })}
                  value={{
                    name: form.watch(`ingredientGroups.${groupIndex}.alternatives.${alternativeIndex}.name`),
                    canonicalIngredientId: form.watch(`ingredientGroups.${groupIndex}.alternatives.${alternativeIndex}.canonicalIngredientId`),
                  }}
                  onChange={(value) => {
                    form.setValue(`ingredientGroups.${groupIndex}.alternatives.${alternativeIndex}.name`, value.name, { shouldDirty: true })
                    form.setValue(`ingredientGroups.${groupIndex}.alternatives.${alternativeIndex}.canonicalIngredientId`, value.canonicalIngredientId, { shouldDirty: true })
                  }}
                  onPasteList={(names) => {
                    form.setValue(`ingredientGroups.${groupIndex}.alternatives.${alternativeIndex}.name`, names[0], { shouldDirty: true })
                    form.setValue(`ingredientGroups.${groupIndex}.alternatives.${alternativeIndex}.canonicalIngredientId`, null, { shouldDirty: true })
                    onAppendGroups(names.slice(1))
                  }}
                  placeholder={t('recommend.filterPlaceholder')}
                  invalid={Boolean(error)}
                  describedBy={error ? errorId : undefined}
                />
                <FieldError id={errorId} message={error?.message} />
              </div>
              <label>
                <span className="sr-only">{t('recipe.quantityForAlternative', {
                  group: groupIndex + 1,
                  number: alternativeIndex + 1,
                })}</span>
                <input
                  {...form.register(`ingredientGroups.${groupIndex}.alternatives.${alternativeIndex}.quantityNote`)}
                  maxLength={240}
                  placeholder={t('recipe.quantityNote')}
                />
              </label>
              {alternatives.fields.length > 1 && (
                <button
                  type="button"
                  className="button ghost"
                  onClick={() => alternatives.remove(alternativeIndex)}
                  aria-label={t('recipe.removeAlternative', { number: alternativeIndex + 1 })}
                >
                  {t('common.remove')}
                </button>
              )}
            </div>
          )
        })}
      </div>
      <div className="form-actions">
        <button
          type="button"
          className="button secondary"
          disabled={alternatives.fields.length >= 8}
          onClick={() => alternatives.append({
            id: crypto.randomUUID(),
            name: '',
            quantityNote: '',
            canonicalIngredientId: null,
          })}
        >
          {t('recipe.addAlternative')}
        </button>
        {canRemove && (
          <button type="button" className="button ghost" onClick={onRemove}>
            {t('recipe.removeGroup')}
          </button>
        )}
      </div>
    </fieldset>
  )
}

export function IngredientGroupsEditor({ form }: { form: UseFormReturn<RecipeForm> }) {
  const { t } = useI18n()
  const groups = useFieldArray({ control: form.control, name: 'ingredientGroups' })
  return (
    <fieldset className="ingredient-groups-editor">
      <legend>{t('recipe.ingredients')}</legend>
      <p className="field-note">{t('recipe.ingredientGroupsHelp')}</p>
      {groups.fields.map((field, groupIndex) => (
        <IngredientGroupEditor
          key={field.id}
          form={form}
          groupIndex={groupIndex}
          canRemove={groups.fields.length > 1}
          onRemove={() => groups.remove(groupIndex)}
          onAppendGroups={(names) => names.forEach((name) => groups.append(emptyIngredientGroup(name)))}
        />
      ))}
      <button
        type="button"
        className="button secondary"
        disabled={groups.fields.length >= 100}
        onClick={() => groups.append(emptyIngredientGroup())}
      >
        {t('recipe.addIngredient')}
      </button>
    </fieldset>
  )
}

export function RecipeEditorForm({
  recipe,
  onSaved,
}: {
  recipe?: Recipe
  onSaved: (id: string) => Promise<unknown> | unknown
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const errorSummaryRef = useRef<HTMLDivElement>(null)
  const [coverFile, setCoverFile] = useState<File | null>(null)
  const [removeCover, setRemoveCover] = useState(false)
  const [coverPreview, setCoverPreview] = useState<string | null>(recipe?.coverImage?.url ?? null)
  const [acknowledgedVersion, setAcknowledgedVersion] = useState(recipe?.version)
  const [coverMutationKey, setCoverMutationKey] = useState(newCoverMutationKey)
  const [coverMetadata, setCoverMetadata] = useState({
    altText: recipe?.coverImage?.altText ?? '',
    sourceUrl: recipe?.coverImage?.sourceUrl ?? '',
    sourceLicense: recipe?.coverImage?.sourceLicense ?? '',
    sourceAttribution: recipe?.coverImage?.sourceAttribution ?? '',
  })
  useEffect(() => {
    if (!coverFile) return
    const url = URL.createObjectURL(coverFile)
    setCoverPreview(url)
    return () => URL.revokeObjectURL(url)
  }, [coverFile])
  const form = useForm<RecipeForm>({
    resolver: zodResolver(recipeSchema(t)),
    defaultValues: recipe ? {
      title: recipe.title,
      description: recipe.description,
      instructions: recipe.instructions,
      visibility: recipe.visibility,
      difficulty: recipe.difficulty,
      estimatedCalories: recipe.estimatedCalories,
      sourceUrl: recipe.source?.url ?? '',
      sourceTitle: recipe.source?.title ?? '',
      sourceAuthor: recipe.source?.author ?? '',
      sourceLicense: recipe.source?.license ?? '',
      sourceAttribution: recipe.source?.attribution ?? '',
      ingredientGroups: recipeIngredientGroups(recipe),
    } : {
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
      ingredientGroups: [emptyIngredientGroup()],
    },
  })
  const save = useMutation({
    mutationFn: (value: RecipeForm) => persistRecipeEdit({
      value,
      recipe,
      acknowledgedVersion,
      coverFile,
      removeCover,
      coverMetadata,
      coverMutationKey,
      acknowledgeVersion: setAcknowledgedVersion,
    }),
    onSuccess: async (saved) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['recipes'] }),
        queryClient.invalidateQueries({ queryKey: ['recipe', saved.id] }),
      ])
      await onSaved(saved.id)
    },
  })
  const ingredientErrors = Array.isArray(form.formState.errors.ingredientGroups)
    ? form.formState.errors.ingredientGroups
    : []
  const validationMessages = [
    form.formState.errors.title?.message,
    form.formState.errors.difficulty?.message,
    form.formState.errors.estimatedCalories?.message,
    ...ingredientErrors.flatMap((error, groupIndex) =>
      Array.isArray(error?.alternatives)
        ? error.alternatives.map((alternative: { name?: { message?: string } } | undefined, alternativeIndex: number) =>
          alternative?.name?.message
            ? t('recipe.alternativeError', {
              group: groupIndex + 1,
              number: alternativeIndex + 1,
              message: alternative.name.message,
            })
            : undefined)
        : [error?.message],
    ),
  ].filter((message): message is string => Boolean(message))
  return (
    <form
      className="editor-form"
      noValidate
      onSubmit={form.handleSubmit(
        (value) => save.mutate(value),
        () => window.requestAnimationFrame(() => errorSummaryRef.current?.focus()),
      )}
    >
      {validationMessages.length > 0 && (
        <div className="error-summary" role="alert" tabIndex={-1} ref={errorSummaryRef}>
          <strong>{t('recipe.checkFields')}</strong>
          <ul>
            {validationMessages.map((message) => <li key={message}>{message}</li>)}
          </ul>
        </div>
      )}
      <label>
        {t('recipe.title')}
        <input
          {...form.register('title')}
          maxLength={120}
          aria-invalid={Boolean(form.formState.errors.title)}
          aria-describedby={form.formState.errors.title ? 'recipe-title-error' : undefined}
        />
        <FieldError id="recipe-title-error" message={form.formState.errors.title?.message} />
      </label>
      <label>
        {t('recipe.description')}
        <textarea {...form.register('description')} rows={3} maxLength={2000} />
      </label>
      <MarkdownEditor
        id="recipe-instructions"
        label={t('recipe.instructions')}
        value={form.watch('instructions')}
        onChange={(instructions) => form.setValue('instructions', instructions, { shouldDirty: true })}
        maxLength={20000}
      />
      <label>
        {t('recipe.visibility')}
        <select {...form.register('visibility')}>
          <option value="private">{t('recipe.privateDefault')}</option>
          <option value="public">{t('recipe.public')}</option>
        </select>
      </label>
      <RecipeMetadataFields form={form} />
      <RecipeProvenanceFields form={form} />
      <RecipeCoverFields
        hasImage={Boolean(coverFile || recipe?.coverImage)}
        removed={removeCover}
        preview={coverPreview}
        metadata={coverMetadata}
        onMetadataChange={(metadata) => {
          setCoverMetadata(metadata)
          setCoverMutationKey(newCoverMutationKey())
        }}
        onFile={(file) => {
          setCoverFile(file)
          setCoverMutationKey(newCoverMutationKey())
          if (file) setRemoveCover(false)
        }}
        onRemove={() => {
          setCoverFile(null)
          setCoverPreview(null)
          setRemoveCover(Boolean(recipe?.coverImage))
          setCoverMutationKey(newCoverMutationKey())
        }}
      />
      <IngredientGroupsEditor form={form} />
      <button className="button primary" disabled={save.isPending}>
        {save.isPending ? t('recipe.saving') : t('recipe.saveRecipe')}
      </button>
      {save.error && <ErrorText error={save.error} />}
      {recipe && <RecipeConflictNotice error={save.error} recipeId={recipe.id} />}
    </form>
  )
}

export function RecipeConflictNotice({
  error,
  recipeId,
}: {
  error: unknown
  recipeId: string
}) {
  const { t } = useI18n()
  if (!(error instanceof ApiError) || error.code !== 'conflict') return null
  return (
    <div className="notice" role="alert">
      <p>{t('recipe.staleConflict')}</p>
      <a
        className="button secondary"
        href={`/recipes/${encodeURIComponent(recipeId)}`}
        target="_blank"
        rel="noopener noreferrer"
      >
        {t('recipe.openLatest')}
      </a>
    </div>
  )
}

interface CoverMetadata {
  altText: string
  sourceUrl: string
  sourceLicense: string
  sourceAttribution: string
}

export function RecipeCoverFields({
  hasImage,
  removed,
  preview,
  metadata,
  onMetadataChange,
  onFile,
  onRemove,
}: {
  hasImage: boolean
  removed: boolean
  preview: string | null
  metadata: CoverMetadata
  onMetadataChange: (metadata: CoverMetadata) => void
  onFile: (file: File | null) => void
  onRemove: () => void
}) {
  const { t } = useI18n()
  const active = hasImage && !removed
  return (
    <fieldset>
      <legend>{t('recipe.coverImage')}</legend>
      <p>{t('recipe.coverImageHelp')}</p>
      <label
        className="image-drop-zone"
        onDragOver={(event) => event.preventDefault()}
        onDrop={(event) => {
          event.preventDefault()
          onFile(event.dataTransfer.files[0] ?? null)
        }}
      >
        {t('recipe.chooseCover')}
        <input
          type="file"
          accept="image/jpeg,image/png,image/webp"
          onChange={(event) => onFile(event.target.files?.[0] ?? null)}
        />
      </label>
      {preview && active && (
        <img className="recipe-cover-preview" src={preview} alt={metadata.altText} />
      )}
      {active && (
        <div className="image-metadata-grid">
          <label>
            {t('recipe.coverAlt')}
            <input
              value={metadata.altText}
              maxLength={300}
              required
              onChange={(event) => onMetadataChange({ ...metadata, altText: event.target.value })}
            />
          </label>
          <label>
            {t('recipe.coverSourceUrl')}
            <input
              type="url"
              value={metadata.sourceUrl}
              onChange={(event) => onMetadataChange({ ...metadata, sourceUrl: event.target.value })}
            />
          </label>
          <label>
            {t('recipe.coverLicense')}
            <input
              value={metadata.sourceLicense}
              maxLength={120}
              onChange={(event) => onMetadataChange({ ...metadata, sourceLicense: event.target.value })}
            />
          </label>
          <label>
            {t('recipe.coverAttribution')}
            <input
              value={metadata.sourceAttribution}
              maxLength={500}
              onChange={(event) =>
                onMetadataChange({ ...metadata, sourceAttribution: event.target.value })
              }
            />
          </label>
        </div>
      )}
      {active && (
        <button type="button" className="button ghost" onClick={onRemove}>
          {t('recipe.removeCover')}
        </button>
      )}
      <p className="field-note">{t('recipe.coverRights')}</p>
    </fieldset>
  )
}

export function PantryPage() {
  const { formatDate, t } = useI18n()
  const search = useSearch({ strict: false }) as { status?: PantryStatus }
  const pantry = useQuery({ queryKey: ['pantry'], queryFn: api.pantry })
  const queryClient = useQueryClient()
  const [draft, setDraft] = useState({
    name: '',
    status: 'have' as PantryStatus,
    quantityNote: '',
    expiresOn: null as string | null,
    canonicalIngredientId: null as string | null,
    autoAddToShoppingListWhenOut: false,
  })
  const [editingName, setEditingName] = useState<string | null>(null)
  const pantryFormRef = useRef<HTMLFormElement>(null)
  useEffect(() => {
    if (!pantry.data || !window.location.hash.startsWith('#pantry-')) return
    const id = window.location.hash.slice('#pantry-'.length)
    const item = pantry.data.find((value) => value.id === id)
    if (!item) return
    setDraft({
      name: item.name,
      status: item.status,
      quantityNote: item.quantityNote,
      expiresOn: item.expiresOn,
      canonicalIngredientId: item.canonicalIngredientId,
      autoAddToShoppingListWhenOut: item.autoAddToShoppingListWhenOut,
    })
    setEditingName(item.name)
    queueMicrotask(() => pantryFormRef.current?.querySelector<HTMLElement>('input')?.focus())
  }, [pantry.data])
  const save = useMutation({
    mutationFn: api.savePantry,
    onSuccess: (items) => {
      queryClient.setQueryData(['pantry'], items)
      setDraft({ name: '', status: 'have', quantityNote: '', expiresOn: null, canonicalIngredientId: null, autoAddToShoppingListWhenOut: false })
      setEditingName(null)
    },
  })
  return (
    <Page title={t('pantry.title')} intro={t('pantry.intro')}>
      {search.status && (
        <div className="active-filter" role="status">
          <span>{t('pantry.filterStatus', { status: pantryStatusLabel(search.status, t) })}</span>
          <Link className="button ghost" to="/pantry" search={{}}>{t('common.clearFilter')}</Link>
        </div>
      )}
      <form
        ref={pantryFormRef}
        className="inline-form pantry-form"
        onSubmit={(event) => {
          event.preventDefault()
          save.mutate([draft])
        }}
      >
        <IngredientCombobox
          label={t('pantry.ingredient')}
          value={{ name: draft.name, canonicalIngredientId: draft.canonicalIngredientId }}
          onChange={(value) => setDraft({ ...draft, ...value })}
          onPasteList={(names) =>
            setDraft({ ...draft, name: names.join(', '), canonicalIngredientId: null })
          }
          placeholder={t('pantry.placeholder')}
        />
        <label>
          {t('pantry.status')}
          <select
            value={draft.status}
            onChange={(event) => setDraft({ ...draft, status: event.target.value as PantryStatus })}
          >
            <option value="have">{t('pantry.have')}</option>
            <option value="out">{t('pantry.out')}</option>
            <option value="unknown">{t('pantry.unknown')}</option>
          </select>
        </label>
        <label>
          {t('pantry.quantityNote')}
          <input value={draft.quantityNote} onChange={(event) => setDraft({ ...draft, quantityNote: event.target.value })} />
        </label>
        <ExpirationEditor
          value={draft.expiresOn}
          onChange={(expiresOn) => setDraft({ ...draft, expiresOn })}
        />
        <label className="choice-card">
          <input
            type="checkbox"
            checked={draft.autoAddToShoppingListWhenOut}
            onChange={(event) =>
              setDraft({ ...draft, autoAddToShoppingListWhenOut: event.target.checked })
            }
          />
          <span>
            <strong>{t('pantry.autoShopping')}</strong>
            <small>{t('pantry.autoShoppingDetail')}</small>
          </span>
        </label>
        <div className="form-actions">
          <button className="button primary" disabled={!draft.name.trim() || save.isPending}>
            {editingName
              ? t('pantry.saveChanges', { name: editingName })
              : t('pantry.saveItem')}
          </button>
          {editingName && (
            <button
              type="button"
              className="button ghost"
              onClick={() => {
                setDraft({ name: '', status: 'have', quantityNote: '', expiresOn: null, canonicalIngredientId: null, autoAddToShoppingListWhenOut: false })
                setEditingName(null)
              }}
            >
              {t('pantry.cancelEditing')}
            </button>
          )}
        </div>
      </form>
      {editingName && <p role="status">{t('pantry.editing', { name: editingName })}</p>}
      {save.error && <ErrorText error={save.error} />}
      <QueryState query={pantry} empty={t('pantry.empty')}>
        <div className="pantry-table" role="table" aria-label={t('pantry.itemsLabel')}>
          {pantry.data?.filter((item) => !search.status || item.status === search.status).map((item) => (
            <div className="pantry-row" role="row" key={item.id} id={`pantry-${item.id}`}>
              <strong role="cell">{item.name}</strong>
              <span role="cell" className={`status ${item.status === 'have' ? 'ready' : item.status === 'out' ? 'shop' : 'verify'}`}>
                {pantryStatusLabel(item.status, t)}
              </span>
              <span role="cell">{item.quantityNote || t('pantry.noQuantity')}</span>
              <span role="cell" className={item.expiresOn && item.expiresOn <= localDateValue(new Date()) ? 'expiry-warning' : ''}>
                {expirationLabel(item.expiresOn, { t, formatDate })}
              </span>
              <button
                type="button"
                className="button ghost"
                onClick={() => {
                  setDraft({
                    name: item.name,
                    status: item.status,
                    quantityNote: item.quantityNote,
                    expiresOn: item.expiresOn,
                    canonicalIngredientId: item.canonicalIngredientId,
                    autoAddToShoppingListWhenOut: item.autoAddToShoppingListWhenOut,
                  })
                  setEditingName(item.name)
                }}
              >
                {t('pantry.editItem')}
              </button>
              <button
                className="button ghost"
                onClick={() =>
                  save.mutate([
                    {
                      name: item.name,
                      quantityNote: item.quantityNote,
                      status: nextStatus(item.status),
                      expiresOn: item.expiresOn,
                      canonicalIngredientId: item.canonicalIngredientId,
                      autoAddToShoppingListWhenOut: item.autoAddToShoppingListWhenOut,
                    },
                  ])
                }
              >
                {t('pantry.changeStatus')}
              </button>
            </div>
          ))}
        </div>
      </QueryState>
    </Page>
  )
}

export function ShoppingListPage() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const shopping = useQuery({ queryKey: ['shopping-list'], queryFn: api.shoppingItems })
  const pantry = useQuery({ queryKey: ['pantry'], queryFn: api.pantry })
  const [draft, setDraft] = useState<IngredientReference>({ name: '', canonicalIngredientId: null })
  const [editingId, setEditingId] = useState<string | null>(null)
  const [removeCandidate, setRemoveCandidate] = useState<NonNullable<typeof shopping.data>[number] | null>(null)
  const [shoppingNotice, setShoppingNotice] = useState<string | null>(null)
  const [purchase, setPurchase] = useState<{
    item: NonNullable<typeof shopping.data>[number]
    pantryItemId: string | null
    createUsingCurrentName: boolean
    expiresOn: string | null
    quantityNote: string
    autoAddToShoppingListWhenOut: boolean
    selectedAlternativeId: string | null
    idempotencyKey: string
  } | null>(null)
  const [banner, setBanner] = useState<{
    name: string
    pantryId: string
    actionId: string
    kept: boolean
  } | null>(null)
  const errorSummaryRef = useRef<HTMLDivElement>(null)
  const removeErrorRef = useRef<HTMLDivElement>(null)
  const removeTriggerRef = useRef<HTMLButtonElement | null>(null)
  const removeKeys = useRef(new Map<string, string>())
  useEffect(() => {
    setPurchase((current) => {
      if (!current) return current
      const latest = shopping.data?.find((item) => item.id === current.item.id)
      return latest ? { ...current, item: latest } : current
    })
  }, [shopping.data])
  const listMutation = useMutation({
    mutationFn: (operation: () => ReturnType<typeof api.addShoppingItem>) => operation(),
    onSuccess: (items) => {
      queryClient.setQueryData(['shopping-list'], items)
      void queryClient.invalidateQueries({ queryKey: ['recommendations'] })
      setDraft({ name: '', canonicalIngredientId: null })
      setEditingId(null)
    },
  })
  const removeMutation = useMutation({
    mutationFn: (item: NonNullable<typeof shopping.data>[number]) => {
      const idempotencyKey = removeKeys.current.get(item.id)
        ?? `${Math.floor(Date.now() / 1000)}.${crypto.randomUUID()}`
      removeKeys.current.set(item.id, idempotencyKey)
      return api.deleteShoppingItem(item.id, item.version, idempotencyKey)
        .then((response) => ({ item, response }))
    },
    onSuccess: ({ item, response }) => {
      queryClient.setQueryData(['shopping-list'], response.shoppingItems)
      void queryClient.invalidateQueries({ queryKey: ['recommendations'] })
      removeKeys.current.delete(item.id)
      setShoppingNotice(t(
        response.requiresDisableAutoRestock
          ? 'shopping.removeAutomaticRemains'
          : 'shopping.removeSuccess',
        { name: item.name },
      ))
      setRemoveCandidate(null)
    },
    onError: (error) => {
      if (error instanceof ApiError && error.status === 409) {
        void queryClient.invalidateQueries({ queryKey: ['shopping-list'] })
        void queryClient.invalidateQueries({ queryKey: ['pantry'] })
        void queryClient.invalidateQueries({ queryKey: ['recommendations'] })
      }
      queueMicrotask(() => removeErrorRef.current?.focus())
    },
  })
  const disableAutomatic = useMutation({
    mutationFn: (item: NonNullable<typeof shopping.data>[number]) => {
      const pantryItem = pantry.data?.find((value) => value.id === item.pantryItemId)
      if (!pantryItem) throw new Error(t('shopping.autoPantryMissing'))
      return api.savePantry([{ ...pantryItem, autoAddToShoppingListWhenOut: false }])
        .then((items) => ({ item, items }))
    },
    onSuccess: async ({ item, items }) => {
      queryClient.setQueryData(['pantry'], items)
      await Promise.all([
        shopping.refetch(),
        queryClient.invalidateQueries({ queryKey: ['recommendations'] }),
      ])
      setShoppingNotice(t('shopping.autoDisabled', { name: item.name }))
      setRemoveCandidate(null)
    },
    onError: (error) => {
      if (error instanceof ApiError && error.status === 409) {
        void queryClient.invalidateQueries({ queryKey: ['shopping-list'] })
        void queryClient.invalidateQueries({ queryKey: ['pantry'] })
        void queryClient.invalidateQueries({ queryKey: ['recommendations'] })
      }
      queueMicrotask(() => removeErrorRef.current?.focus())
    },
  })
  const purchaseMutation = useMutation({
    mutationFn: ({ disposition }: { disposition: 'remove' | 'keep' }) => {
      if (!purchase) throw new Error('No purchase is open')
      const options = purchase.item.choiceAlternatives
        ?.find((alternative) => alternative.id === purchase.selectedAlternativeId)
        ?.purchaseOptions ?? purchase.item.purchaseOptions
      const candidate = options.candidates.find(
        (value) => value.id === purchase.pantryItemId,
      )
      return api.purchaseShoppingItem(
        purchase.item.id,
        {
          disposition,
          shoppingExpectedVersion: purchase.item.version,
          pantryItemId: purchase.pantryItemId,
          createUsingCurrentName: purchase.createUsingCurrentName,
          pantryExpectedVersion: candidate?.version ?? null,
          expiresOn: purchase.expiresOn,
          quantityNote: purchase.quantityNote,
          autoAddToShoppingListWhenOut: purchase.autoAddToShoppingListWhenOut,
          selectedAlternativeId: purchase.selectedAlternativeId,
        },
        purchase.idempotencyKey,
      )
    },
    onSuccess: (result, variables) => {
      queryClient.setQueryData(['shopping-list'], result.shoppingItems)
      queryClient.setQueryData<PantryItem[]>(['pantry'], (current = []) => [
        ...current.filter((item) => item.id !== result.pantry.id),
        result.pantry,
      ])
      setBanner({
        name: result.pantry.name,
        pantryId: result.pantry.id,
        actionId: result.purchaseActionId,
        kept: variables.disposition === 'keep',
      })
      void queryClient.invalidateQueries({ queryKey: ['recommendations'] })
      setPurchase(null)
    },
    onError: (error) => {
      if (error instanceof ApiError && error.status === 409) {
        void queryClient.invalidateQueries({ queryKey: ['shopping-list'] })
        void queryClient.invalidateQueries({ queryKey: ['pantry'] })
      }
      queueMicrotask(() => errorSummaryRef.current?.focus())
    },
  })
  const undoMutation = useMutation({
    mutationFn: api.undoShoppingPurchase,
    onSuccess: (items) => {
      queryClient.setQueryData(['shopping-list'], items)
      void queryClient.invalidateQueries({ queryKey: ['pantry'] })
      void queryClient.invalidateQueries({ queryKey: ['recommendations'] })
      setBanner(null)
    },
    onError: () => {
      void queryClient.invalidateQueries({ queryKey: ['shopping-list'] })
      void queryClient.invalidateQueries({ queryKey: ['pantry'] })
      void queryClient.invalidateQueries({ queryKey: ['recommendations'] })
    },
  })
  const items = shopping.data ?? []
  function sourceLabel(kind: string, label: string) {
    if (kind === 'automatic') return t('shopping.sourceAutomatic')
    if (kind === 'manual') return t('shopping.sourceManual')
    return t('shopping.sourceRecipe', { recipe: label })
  }
  function openPurchase(item: (typeof items)[number]) {
    const candidate = item.purchaseOptions.candidates.find((value) => value.id === item.pantryItemId)
      ?? item.purchaseOptions.candidates[0]
    const reuse = item.purchaseOptions.resolution === 'reuse'
    const needsAlternative = item.purchaseOptions.resolution === 'selectAlternative'
    setPurchase({
      item,
      pantryItemId: !needsAlternative && reuse ? candidate?.id ?? null : null,
      createUsingCurrentName: !needsAlternative
        && (item.purchaseOptions.resolution === 'create' || item.purchaseOptions.resolution === 'choice'),
      expiresOn: !needsAlternative && candidate?.status !== 'out' ? candidate?.expiresOn ?? null : null,
      quantityNote: !needsAlternative ? candidate?.quantityNote ?? '' : '',
      autoAddToShoppingListWhenOut: !needsAlternative
        ? candidate?.autoAddToShoppingListWhenOut ?? false
        : false,
      selectedAlternativeId: null,
      idempotencyKey: `${Math.floor(Date.now() / 1000)}.${crypto.randomUUID()}`,
    })
    purchaseMutation.reset()
  }
  const activePurchaseOptions = purchase
    ? purchase.item.choiceAlternatives
      ?.find((alternative) => alternative.id === purchase.selectedAlternativeId)
      ?.purchaseOptions ?? purchase.item.purchaseOptions
    : null
  const purchaseIngredientName = purchase?.item.choiceAlternatives
    ?.find((alternative) => alternative.id === purchase.selectedAlternativeId)
    ?.name ?? purchase?.item.name ?? ''
  const purchaseNeedsAlternative = purchase?.item.purchaseOptions.resolution === 'selectAlternative'
    && !purchase.selectedAlternativeId
  const purchaseNeedsPantryChoice = activePurchaseOptions?.resolution === 'choice'
    && !purchase?.pantryItemId
    && !purchase?.createUsingCurrentName
  return (
    <Page title={t('shopping.title')} intro={t('shopping.intro')}>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault()
          if (!draft.name.trim()) return
          listMutation.mutate(() => editingId ? api.updateShoppingItem(editingId, draft) : api.addShoppingItem(draft))
        }}
      >
        <IngredientCombobox
          label={t('shopping.item')}
          value={draft}
          onChange={setDraft}
          placeholder={t('shopping.placeholder')}
        />
        <button className="button primary" disabled={!draft.name.trim() || listMutation.isPending}>
          {editingId ? t('shopping.saveEdit') : t('shopping.add')}
        </button>
        {editingId && (
          <button type="button" className="button ghost" onClick={() => {
            setEditingId(null)
            setDraft({ name: '', canonicalIngredientId: null })
          }}>{t('common.cancel')}</button>
        )}
      </form>
      {banner && (
        <div className="status-banner" role="status" aria-live="polite">
          <span>{t(banner.kept ? 'shopping.purchaseKept' : 'shopping.purchaseRemoved', { name: banner.name })}</span>
          <div className="form-actions">
            <a className="button secondary" href={`/pantry#pantry-${banner.pantryId}`}>{t('shopping.editPantry')}</a>
            <button
              type="button"
              className="button secondary"
              disabled={undoMutation.isPending}
              onClick={() => undoMutation.mutate(banner.actionId)}
            >{t('shopping.undoPurchase')}</button>
            <button type="button" className="button ghost" aria-label={t('common.dismiss')} onClick={() => setBanner(null)}>×</button>
          </div>
        </div>
      )}
      {undoMutation.error && (
        <div className="error-summary" role="alert">
          {undoMutation.error instanceof ApiError && undoMutation.error.status === 409
            ? t('shopping.undoConflict')
            : <ErrorText error={undoMutation.error} />}
        </div>
      )}
      {shoppingNotice && <p role="status" aria-live="polite">{shoppingNotice}</p>}
      {!removeCandidate && (listMutation.error || removeMutation.error || disableAutomatic.error) && (
        <div className="error-summary" role="alert" tabIndex={-1} ref={removeErrorRef}>
          {(removeMutation.error instanceof ApiError && removeMutation.error.status === 409)
            || (disableAutomatic.error instanceof ApiError && disableAutomatic.error.status === 409)
            ? t('shopping.removeConflict')
            : <ErrorText error={listMutation.error ?? removeMutation.error ?? disableAutomatic.error} />}
        </div>
      )}
      <QueryState query={shopping} empty={t('shopping.empty')}>
        <div className="card-list" aria-label={t('shopping.itemsLabel')}>
          {items.map((item) => {
            const canEdit = item.sources.some((source) => source.kind === 'manual')
            const hasPersistentSource = item.sources.some((source) => source.kind !== 'automatic')
            return (
              <article className="recipe-card" key={item.id}>
                <strong>{item.name}</strong>
                <span>{item.sources.map((source) => sourceLabel(source.kind, source.label)).join(' · ')}</span>
                {item.automatic && (
                  <p className="field-help">
                    {t(
                      hasPersistentSource
                        ? 'shopping.automaticMergedHelp'
                        : 'shopping.automaticOnlyHelp',
                    )}
                  </p>
                )}
                <div className="form-actions">
                  <button
                    type="button"
                    className="button primary"
                    aria-label={t('shopping.boughtLabel', { name: item.name })}
                    onClick={() => openPurchase(item)}
                  >{t('shopping.bought')}</button>
                  {item.automatic && item.pantryItemId && (
                    <button
                      type="button"
                      className="button ghost"
                      disabled={disableAutomatic.isPending}
                      onClick={() => disableAutomatic.mutate(item)}
                    >{t('shopping.disableAutomatic')}</button>
                  )}
                  {canEdit && (
                    <button type="button" className="button ghost" onClick={() => {
                      setEditingId(item.id)
                      setDraft({ name: item.name, canonicalIngredientId: item.canonicalIngredientId })
                    }}>{t('shopping.edit')}</button>
                  )}
                  <button
                    type="button"
                    className="button secondary destructive"
                    disabled={removeMutation.isPending || disableAutomatic.isPending}
                    aria-label={t('shopping.removeLabel', { name: item.name })}
                    onClick={(event) => {
                      setShoppingNotice(null)
                      removeMutation.reset()
                      disableAutomatic.reset()
                      if (item.automatic) {
                        removeTriggerRef.current = event.currentTarget
                        setRemoveCandidate(item)
                      } else {
                        removeMutation.mutate(item)
                      }
                    }}
                  >
                    {t('shopping.remove')}
                  </button>
                </div>
              </article>
            )
          })}
        </div>
      </QueryState>
      <Dialog.Root
        open={removeCandidate !== null}
        onOpenChange={(open) => {
          if (!open && !removeMutation.isPending && !disableAutomatic.isPending) {
            setRemoveCandidate(null)
          }
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="dialog-overlay" />
          {removeCandidate && (
            <Dialog.Content
              className="confirmation-sheet remove-shopping-dialog"
              onCloseAutoFocus={(event) => {
                event.preventDefault()
                removeTriggerRef.current?.focus()
              }}
            >
              <Dialog.Title>{t('shopping.removeDialogTitle')}</Dialog.Title>
              <Dialog.Description>
                {t(
                  removeCandidate.sources.some((source) => source.kind !== 'automatic')
                    ? 'shopping.removeMergedDetail'
                    : 'shopping.removeAutomaticOnlyDetail',
                  { name: removeCandidate.name },
                )}
              </Dialog.Description>
              {(removeMutation.error || disableAutomatic.error) && (
                <div className="error-summary" role="alert" tabIndex={-1} ref={removeErrorRef}>
                  {(removeMutation.error instanceof ApiError && removeMutation.error.status === 409)
                    || (disableAutomatic.error instanceof ApiError && disableAutomatic.error.status === 409)
                    ? t('shopping.removeConflict')
                    : <ErrorText error={removeMutation.error ?? disableAutomatic.error} />}
                </div>
              )}
              <div className="dialog-actions">
                {removeCandidate.sources.some((source) => source.kind !== 'automatic') && (
                  <button
                    type="button"
                    className="button secondary destructive"
                    disabled={removeMutation.isPending || disableAutomatic.isPending}
                    onClick={() => removeMutation.mutate(removeCandidate)}
                  >
                    {t('shopping.removePersistent')}
                  </button>
                )}
                <button
                  type="button"
                  className="button secondary"
                  disabled={removeMutation.isPending || disableAutomatic.isPending}
                  onClick={() => disableAutomatic.mutate(removeCandidate)}
                >
                  {t('shopping.disableAutomatic')}
                </button>
                <Dialog.Close asChild>
                  <button
                    type="button"
                    className="button ghost"
                    disabled={removeMutation.isPending || disableAutomatic.isPending}
                  >
                    {t('common.cancel')}
                  </button>
                </Dialog.Close>
              </div>
            </Dialog.Content>
          )}
        </Dialog.Portal>
      </Dialog.Root>
      <Dialog.Root open={purchase !== null} onOpenChange={(open) => {
        if (!open && !purchaseMutation.isPending) setPurchase(null)
      }}>
        <Dialog.Portal>
          <Dialog.Overlay className="dialog-overlay" />
          {purchase && (
            <Dialog.Content className="confirmation-sheet purchase-sheet">
              <div className="purchase-sheet-content">
                <Dialog.Title>{t('shopping.updatePantry')}</Dialog.Title>
                <Dialog.Description>{t('shopping.updatePantryDetail', { name: purchaseIngredientName })}</Dialog.Description>
                {purchaseMutation.error && (
                  <div className="error-summary" role="alert" tabIndex={-1} ref={errorSummaryRef}>
                    {purchaseMutation.error instanceof ApiError && purchaseMutation.error.status === 409
                      ? t('shopping.conflict')
                      : t('shopping.networkError')}
                  </div>
                )}
                {activePurchaseOptions?.resolution === 'choice' && (
                  <fieldset>
                    <legend>{t('shopping.matchRequired')}</legend>
                    <p>{t('shopping.matchGuidance')}</p>
                    {activePurchaseOptions.candidates.map((candidate) => (
                      <label className="choice-card" key={candidate.id}>
                        <input
                          type="radio"
                          name="pantry-match"
                          checked={!purchase.createUsingCurrentName && purchase.pantryItemId === candidate.id}
                          onChange={() => setPurchase({
                            ...purchase,
                            pantryItemId: candidate.id,
                            createUsingCurrentName: false,
                            expiresOn: candidate.status === 'out' ? null : candidate.expiresOn,
                            quantityNote: candidate.quantityNote,
                            autoAddToShoppingListWhenOut: candidate.autoAddToShoppingListWhenOut,
                          })}
                        />
                        <span><strong>{t('shopping.useExisting', { name: candidate.name })}</strong><small>{t('shopping.matchCandidateStatus', { status: t(`pantry.${candidate.status}`) })}</small></span>
                      </label>
                    ))}
                    <label className="choice-card">
                      <input
                        type="radio"
                        name="pantry-match"
                        checked={purchase.createUsingCurrentName}
                        onChange={() => setPurchase({
                          ...purchase,
                          pantryItemId: null,
                          createUsingCurrentName: true,
                          expiresOn: null,
                          quantityNote: '',
                          autoAddToShoppingListWhenOut: false,
                        })}
                      />
                      <span>{t('shopping.createCurrent', { name: purchaseIngredientName })}</span>
                    </label>
                  </fieldset>
                )}
                {activePurchaseOptions?.resolution === 'create' && <p>{t('shopping.noMatch')}</p>}
                {purchase.item.choiceAlternatives && purchase.item.choiceAlternatives.length > 1 && (
                  <fieldset>
                    <legend>{t('shopping.boughtAlternative')}</legend>
                    <p>{t('shopping.boughtAlternativeHelp')}</p>
                    {purchase.item.choiceAlternatives.map((alternative) => (
                      <label className="choice-card" key={ingredientIdentity(alternative)}>
                        <input
                          type="radio"
                          name="purchased-alternative"
                          checked={purchase.selectedAlternativeId === alternative.id}
                          onChange={() => {
                            const options = alternative.purchaseOptions ?? {
                              resolution: 'create' as const,
                              candidates: [],
                            }
                            const candidate = options.resolution === 'reuse'
                              ? options.candidates[0]
                              : undefined
                            setPurchase({
                              ...purchase,
                              selectedAlternativeId: alternative.id,
                              pantryItemId: candidate?.id ?? null,
                              createUsingCurrentName: options.resolution === 'create'
                                || options.resolution === 'choice',
                              expiresOn: candidate?.status === 'out' ? null : candidate?.expiresOn ?? null,
                              quantityNote: candidate?.quantityNote ?? '',
                              autoAddToShoppingListWhenOut: candidate?.autoAddToShoppingListWhenOut ?? false,
                            })
                          }}
                        />
                        <span>{alternative.name}</span>
                      </label>
                    ))}
                  </fieldset>
                )}
                <p><strong>{t('shopping.available')}</strong></p>
                <ExpirationEditor value={purchase.expiresOn} onChange={(expiresOn) => setPurchase({ ...purchase, expiresOn })} />
                <label>{t('shopping.quantityNote')}
                  <input value={purchase.quantityNote} maxLength={240} onChange={(event) => setPurchase({ ...purchase, quantityNote: event.target.value })} />
                </label>
                <label className="check-row">
                  <input type="checkbox" checked={purchase.autoAddToShoppingListWhenOut} onChange={(event) => setPurchase({ ...purchase, autoAddToShoppingListWhenOut: event.target.checked })} />
                  {t('shopping.autoAdd')}
                </label>
              </div>
              <div className="purchase-sheet-actions">
                <button
                  type="button"
                  className="button primary"
                  disabled={purchaseMutation.isPending || purchaseNeedsAlternative || purchaseNeedsPantryChoice}
                  onClick={() => purchaseMutation.mutate({ disposition: 'remove' })}
                >{t('shopping.saveRemove')}</button>
                <button
                  type="button"
                  className="button secondary"
                  disabled={purchaseMutation.isPending || purchaseNeedsAlternative || purchaseNeedsPantryChoice}
                  onClick={() => purchaseMutation.mutate({ disposition: 'keep' })}
                >{t('shopping.saveKeep')}</button>
                <Dialog.Close asChild><button type="button" className="button ghost" disabled={purchaseMutation.isPending}>{t('common.cancel')}</button></Dialog.Close>
              </div>
            </Dialog.Content>
          )}
        </Dialog.Portal>
      </Dialog.Root>
    </Page>
  )
}

export function ExpirationEditor({
  value,
  onChange,
  now = new Date(),
}: {
  value: string | null
  onChange: (value: string | null) => void
  now?: Date
}) {
  const { formatDate, t } = useI18n()
  const relativeDays = value === null ? null : expirationRelativeDays(value, now)
  const [daysText, setDaysText] = useState(() => relativeDays?.toString() ?? '')
  const [daysError, setDaysError] = useState<string | null>(null)
  const minimumDays = -3650
  const maximumDays = 3650
  useEffect(() => {
    setDaysText(relativeDays?.toString() ?? '')
    setDaysError(null)
  }, [relativeDays])
  const options = [
    [t('expiry.none'), null],
    [t('expiry.today'), 0],
    [t('expiry.tomorrow'), 1],
    [t('expiry.threeDays'), 3],
    [t('expiry.sevenDays'), 7],
    [t('expiry.fourteenDays'), 14],
    [t('expiry.thirtyDays'), 30],
  ] as const
  function applyDays(input: string) {
    setDaysText(input)
    if (!/^-?\d+$/.test(input)) {
      setDaysError(input ? t('expiry.integerError') : null)
      return
    }
    const days = Number(input)
    if (!Number.isSafeInteger(days) || days < minimumDays || days > maximumDays) {
      setDaysError(t('expiry.rangeError', { minimum: minimumDays, maximum: maximumDays }))
      return
    }
    setDaysError(null)
    onChange(expirationShortcut(days, now))
  }
  function adjustDays(delta: number) {
    const parsed = /^-?\d+$/.test(daysText) ? Number(daysText) : (relativeDays ?? 0)
    const next = Math.max(minimumDays, Math.min(maximumDays, parsed + delta))
    applyDays(next.toString())
  }
  let confirmation: string | null = null
  if (value !== null && relativeDays !== null) {
    if (relativeDays === 0) {
      confirmation = t('expiry.confirmToday', { date: formatDate(value) })
    } else if (relativeDays === 1) {
      confirmation = t('expiry.confirmTomorrow', { date: formatDate(value) })
    } else if (relativeDays < 0) {
      confirmation = t('expiry.confirmPast', {
        days: Math.abs(relativeDays) === 1
          ? t('common.day')
          : t('common.days', { count: Math.abs(relativeDays) }),
        date: formatDate(value),
      })
    } else {
      confirmation = t('expiry.confirmFuture', {
        days: t('common.days', { count: relativeDays }),
        date: formatDate(value),
      })
    }
  }
  return (
    <fieldset className="expiration-editor">
      <legend>{t('expiry.optional')}</legend>
      <div className="date-shortcuts" role="group" aria-label={t('expiry.shortcuts')}>
        {options.map(([label, days]) => (
          <button
            type="button"
            className={relativeDays === days ? 'button secondary selected' : 'button ghost'}
            aria-pressed={relativeDays === days}
            key={label}
            onClick={() => {
              if (days === null) {
                setDaysText('')
                setDaysError(null)
                onChange(null)
              } else {
                applyDays(days.toString())
              }
            }}
          >
            {label}
          </button>
        ))}
      </div>
      <label htmlFor="expiration-relative-days">{t('expiry.daysPrompt')}</label>
      <div className="relative-days-control">
        <button
          type="button"
          className="button secondary"
          aria-label={t('expiry.decreaseDays')}
          onClick={() => adjustDays(-1)}
        >
          −
        </button>
        <input
          id="expiration-relative-days"
          type="number"
          inputMode="numeric"
          min={minimumDays}
          max={maximumDays}
          step={1}
          value={daysText}
          aria-invalid={Boolean(daysError)}
          aria-describedby={daysError ? 'expiration-days-error' : 'expiration-days-help'}
          onChange={(event) => applyDays(event.target.value)}
        />
        <button
          type="button"
          className="button secondary"
          aria-label={t('expiry.increaseDays')}
          onClick={() => adjustDays(1)}
        >
          +
        </button>
      </div>
      {daysError && <p id="expiration-days-error" className="error" role="alert">{daysError}</p>}
      {confirmation && <output className="field-help" aria-live="polite">{confirmation}</output>}
      <p id="expiration-days-help" className="field-help">{t('expiry.dateOnlyHelp')}</p>
      {relativeDays !== null && relativeDays < 0 && (
        <p className="expiry-warning" role="alert">
          {t('expiry.pastWarning')}
        </p>
      )}
    </fieldset>
  )
}

function SettingsPage() {
  const { t } = useI18n()
  const settings = useQuery({ queryKey: ['settings'], queryFn: api.settings })
  const me = useQuery({ queryKey: ['me'], queryFn: api.me })
  return (
    <Page title={t('settings.title')} intro={t('settings.intro')}>
      {me.data && <LanguageSettings user={me.data} />}
      {me.data && <ProfileSettings user={me.data} />}
      <AccountSecuritySettings />
      <FavoriteAccountsSettings />
      <QueryState query={settings} empty={t('settings.unavailable')}>
        {settings.data && <SettingsForm initial={settings.data} />}
      </QueryState>
      <ImportTokenSettings />
      <section className="settings-section">
        <h2>{t('moderation.claimTitle')}</h2>
        <p>{t('moderation.claimSettingsHelp')}</p>
        <Link className="button secondary" to="/admin/claim">
          {t('moderation.openClaim')}
        </Link>
      </section>
    </Page>
  )
}

function NotificationsPage() {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const notifications = useQuery({ queryKey: ['notifications'], queryFn: api.notifications })
  const read = useMutation({
    mutationFn: api.readNotification,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['notifications'] }),
  })
  return (
    <Page title={t('moderation.notificationsTitle')} intro={t('moderation.notificationsIntro')}>
      <QueryState query={notifications} empty={t('moderation.noNotifications')}>
        <div className="card-list">
          {notifications.data?.map((notification) => (
            <article className="detail-card" key={notification.id}>
              <h2>{t(`moderation.notification.${notification.kind}` as never)}</h2>
              <p>{t('moderation.reasonCode', { reason: notification.reasonCode })}</p>
              <Link to="/recipes/$recipeId" params={{ recipeId: notification.recipeId }}>
                {t('moderation.openRecipe')}
              </Link>
              {!notification.readAt && (
                <button
                  type="button"
                  className="button ghost"
                  disabled={read.isPending}
                  onClick={() => read.mutate(notification.id)}
                >
                  {t('moderation.markRead')}
                </button>
              )}
            </article>
          ))}
        </div>
      </QueryState>
    </Page>
  )
}

function ModeratorClaimPage() {
  const { t } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [code, setCode] = useState('')
  const claim = useMutation({
    mutationFn: async () => {
      const challenge = await api.startModeratorClaim(code.trim())
      const credential = await getPasskey(challenge)
      return api.finishModeratorClaim(code.trim(), credential)
    },
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['me'] })
      await navigate({ to: '/admin/moderation' })
    },
  })
  return (
    <Page title={t('moderation.claimTitle')} intro={t('moderation.claimIntro')}>
      <form
        className="settings-form"
        onSubmit={(event) => {
          event.preventDefault()
          claim.mutate()
        }}
      >
        <label>
          {t('moderation.claimCode')}
          <input
            value={code}
            autoComplete="one-time-code"
            onChange={(event) => setCode(event.target.value)}
          />
        </label>
        <button className="button primary" disabled={!code.trim() || claim.isPending}>
          {claim.isPending ? t('moderation.verifyingPasskey') : t('moderation.claimSubmit')}
        </button>
        {claim.error && <ErrorText error={claim.error} />}
      </form>
    </Page>
  )
}

function AdminReauthButton({ onSuccess }: { onSuccess?: () => void }) {
  const { t } = useI18n()
  const reauth = useMutation({
    mutationFn: async () => {
      const challenge = await api.startAdminReauth()
      const credential = await getPasskey(challenge)
      await api.finishAdminReauth(credential)
    },
    onSuccess,
  })
  return (
    <div>
      <button
        type="button"
        className="button secondary"
        disabled={reauth.isPending}
        onClick={() => reauth.mutate()}
      >
        {reauth.isPending ? t('moderation.verifyingPasskey') : t('moderation.reauth')}
      </button>
      {reauth.isSuccess && <p role="status">{t('moderation.reauthComplete')}</p>}
      {reauth.error && <ErrorText error={reauth.error} />}
    </div>
  )
}

function ModerationQueuePage() {
  const { t } = useI18n()
  const queue = useQuery({ queryKey: ['moderation-queue'], queryFn: () => api.moderationQueue() })
  return (
    <Page title={t('moderation.queueTitle')} intro={t('moderation.queueIntro')}>
      <AdminReauthButton onSuccess={() => void queue.refetch()} />
      <QueryState query={queue} empty={t('moderation.queueEmpty')}>
        <div className="moderation-queue">
          {queue.data?.items.map((item) => <ModerationQueueCard key={item.recipeId} item={item} />)}
        </div>
      </QueryState>
    </Page>
  )
}

function ModerationQueueCard({ item }: { item: ModerationQueueItem }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [reasonCode, setReasonCode] = useState('policy_violation')
  const action = useMutation({
    mutationFn: (kind: 'review' | 'restrict' | 'reinstate') => {
      const value = moderationMutationInput(item, kind, reasonCode)
      const key = `${Math.floor(Date.now() / 1000)}.${crypto.randomUUID()}`
      if (kind === 'review') return api.reviewRecipe(item.recipeId, value, key)
      if (kind === 'restrict') return api.restrictRecipe(item.recipeId, value, key)
      return api.reinstateRecipe(item.recipeId, value, key)
    },
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['moderation-queue'] }),
  })
  return (
    <article className="detail-card moderation-card">
      <div className="card-heading">
        <div>
          <h2>{item.title}</h2>
          <p>{t('moderation.byHandle', { handle: item.authorHandle })}</p>
        </div>
        <span className="status verify">
          {t(`moderation.state.${item.moderationState}` as never)}
        </span>
      </div>
      <p>{t('moderation.queueSummary', {
        review: t(`moderation.reviewState.${item.reviewState}` as never),
        reports: item.reportCount,
        appeal: item.activeAppeal ? t('moderation.yes') : t('moderation.no'),
      })}</p>
      <label>
        {t('moderation.actionReason')}
        <select value={reasonCode} onChange={(event) => setReasonCode(event.target.value)}>
          {['policy_violation', 'safety', 'privacy', 'copyright', 'spam', 'appeal_reviewed', 'other']
            .map((value) => (
              <option key={value} value={value}>
                {t(`moderation.actionReason.${value}` as never)}
              </option>
            ))}
        </select>
      </label>
      <div className="form-actions">
        {item.reviewState === 'unreviewed' && (
          <button type="button" className="button secondary" onClick={() => action.mutate('review')}>
            {t('moderation.markReviewed')}
          </button>
        )}
        {item.moderationState === 'clear' ? (
          <button type="button" className="button destructive" onClick={() => action.mutate('restrict')}>
            {t('moderation.restrict')}
          </button>
        ) : (
          <button type="button" className="button secondary" onClick={() => action.mutate('reinstate')}>
            {t('moderation.reinstate')}
          </button>
        )}
        {item.caseId && (
          <Link className="button ghost" to="/admin/moderation/cases/$caseId" params={{ caseId: item.caseId }}>
            {t('moderation.openCase')}
          </Link>
        )}
      </div>
      {action.error && <ErrorText error={action.error} />}
    </article>
  )
}

export function moderationMutationInput(
  item: ModerationQueueItem,
  kind: 'review' | 'restrict' | 'reinstate',
  actionReasonCode: string,
) {
  return {
    expectedRecipeVersion: item.recipeVersion,
    expectedModerationVersion: item.moderationVersion,
    expectedCaseVersion: item.caseVersion,
    reasonCode: kind === 'review' ? 'no_action_needed' : actionReasonCode,
  }
}

export function moderationEvidenceImageUrl(caseId: string) {
  return `/api/admin/moderation/cases/${encodeURIComponent(caseId)}/evidence-image`
}

function ModerationCasePage() {
  const { caseId } = moderationCaseRoute.useParams()
  const { t } = useI18n()
  const moderationCase = useQuery({
    queryKey: ['moderation-case', caseId],
    queryFn: () => api.moderationCase(caseId),
    retry: false,
  })
  if (moderationCase.error instanceof ApiError
    && moderationCase.error.code === 'admin_passkey_reauth_required') {
    return (
      <Page title={t('moderation.caseTitle')} intro={t('moderation.caseReauth')}>
        <AdminReauthButton onSuccess={() => void moderationCase.refetch()} />
      </Page>
    )
  }
  return (
    <Page title={t('moderation.caseTitle')} intro={t('moderation.caseIntro')}>
      <QueryState query={moderationCase} empty={t('moderation.caseMissing')}>
        {moderationCase.data && <ModerationCaseContent value={moderationCase.data} />}
      </QueryState>
    </Page>
  )
}

function ModerationCaseContent({ value }: { value: ModerationCase }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [reasonCode, setReasonCode] = useState('appeal_reviewed')
  const resolve = useMutation({
    mutationFn: (resolution: 'reinstate' | 'uphold') => {
      if (!value.appeal || value.recipeVersion === null || value.moderationVersion === null) {
        throw new Error('Case state is incomplete')
      }
      return api.resolveAppeal(value.appeal.id, {
        resolution,
        expectedRecipeVersion: value.recipeVersion,
        expectedModerationVersion: value.moderationVersion,
        expectedCaseVersion: value.version,
        expectedAppealVersion: value.appeal.version,
        reasonCode,
      }, `${Math.floor(Date.now() / 1000)}.${crypto.randomUUID()}`)
    },
    onSuccess: () => {
      void Promise.all([
        queryClient.invalidateQueries({ queryKey: ['moderation-case', value.id] }),
        queryClient.invalidateQueries({ queryKey: ['moderation-queue'] }),
      ])
    },
  })
  return (
    <article className="detail-card">
      <p>{t('moderation.caseStatus', { status: value.status })}</p>
      <p>{t('moderation.byHandle', { handle: value.evidence.authorHandle })}</p>
      <h2>{value.evidence.title}</h2>
      {value.evidence.imageId && (
        <figure className="recipe-cover-detail">
          <img
            src={moderationEvidenceImageUrl(value.id)}
            alt={value.evidence.title}
          />
        </figure>
      )}
      <SafeMarkdown value={value.evidence.description} />
      <SafeMarkdown value={value.evidence.instructions} />
      <h2>{t('moderation.reports')}</h2>
      <div className="card-list">
        {value.reports.map((report, index) => (
          <section className="source-note" key={`${report.createdAt}-${index}`}>
            <strong>{report.reason}</strong>
            <p className="pre-line">{report.details || t('common.notProvided')}</p>
          </section>
        ))}
      </div>
      {value.appeal && (
        <section className="moderation-banner">
          <h2>{t('moderation.activeAppeal')}</h2>
          <p className="pre-line">{value.appeal.reason}</p>
          <label>
            {t('moderation.actionReason')}
            <select value={reasonCode} onChange={(event) => setReasonCode(event.target.value)}>
              {['appeal_reviewed', 'policy_violation', 'safety', 'privacy', 'copyright', 'spam', 'other']
                .map((reason) => (
                  <option key={reason} value={reason}>
                    {t(`moderation.actionReason.${reason}` as never)}
                  </option>
                ))}
            </select>
          </label>
          <div className="form-actions">
            <button type="button" className="button secondary" onClick={() => resolve.mutate('reinstate')}>
              {t('moderation.reinstate')}
            </button>
            <button type="button" className="button destructive" onClick={() => resolve.mutate('uphold')}>
              {t('moderation.uphold')}
            </button>
          </div>
          {resolve.error && <ErrorText error={resolve.error} />}
        </section>
      )}
    </article>
  )
}

export function LanguageSettings({ user }: { user: User }) {
  const queryClient = useQueryClient()
  const { locale, setLocale, t } = useI18n()
  const save = useMutation({
    mutationFn: (next: Locale) => api.saveLocale(next),
    onSuccess: (value) => {
      queryClient.setQueryData(['me'], value)
    },
    onError: () => {
      const previous = user.locale ?? locale
      queryClient.setQueryData(['me'], user)
      setLocale(previous)
    },
  })
  return (
    <section className="settings-section">
      <h2>{t('locale.title')}</h2>
      <label>
        {t('locale.label')}
        <select
          value={user.locale ?? locale}
          onChange={(event) => {
            const next = event.target.value as Locale
            queryClient.setQueryData(['me'], { ...user, locale: next })
            setLocale(next)
            save.mutate(next)
          }}
        >
          <option value="en">{t('locale.en')}</option>
          <option value="zh-CN">{t('locale.zhCN')}</option>
        </select>
      </label>
      {save.isSuccess && <p role="status">{t('locale.saved')}</p>}
      {save.error && <ErrorText error={save.error} />}
    </section>
  )
}

export function ProfileSetup({ user }: { user: User }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [choice, setChoice] = useState<'anonymous' | 'suggested' | 'custom'>('anonymous')
  const [custom, setCustom] = useState('')
  const save = useMutation({
    mutationFn: (nickname: string | null) => api.saveProfile(nickname),
    onSuccess: async (value) => {
      queryClient.setQueryData(['me'], value)
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['recipes'] }),
        queryClient.invalidateQueries({ queryKey: ['recipe'] }),
        queryClient.invalidateQueries({ queryKey: ['recommendations'] }),
      ])
    },
  })
  return (
    <Dialog.Root open>
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="confirmation-sheet">
          <Dialog.Title>{t('profile.setupTitle')}</Dialog.Title>
          <Dialog.Description>
            {t('profile.setupDescription')}
          </Dialog.Description>
          <fieldset className="choice-list">
            <label className="radio-pill">
              <input
                type="radio"
                name="profile-choice"
                checked={choice === 'anonymous'}
                onChange={() => setChoice('anonymous')}
              />
              {t('profile.anonymousDefault')}
            </label>
            <label className="radio-pill">
              <input
                type="radio"
                name="profile-choice"
                checked={choice === 'suggested'}
                onChange={() => setChoice('suggested')}
              />
              {t('profile.useSuggested', { name: user.displayName })}
            </label>
            <label className="radio-pill">
              <input
                type="radio"
                name="profile-choice"
                checked={choice === 'custom'}
                onChange={() => setChoice('custom')}
              />
              {t('profile.custom')}
            </label>
            {choice === 'custom' && (
              <input
                aria-label={t('profile.customLabel')}
                value={custom}
                maxLength={40}
                onChange={(event) => setCustom(event.target.value)}
              />
            )}
          </fieldset>
          <div className="dialog-actions">
            <button className="button ghost" onClick={() => save.mutate(null)}>
              {t('profile.skip')}
            </button>
            <button
              className="button primary"
              disabled={save.isPending || (choice === 'custom' && !custom.trim())}
              onClick={() =>
                save.mutate(choice === 'suggested' ? user.displayName : choice === 'custom' ? custom : null)
              }
            >
              {t('profile.save')}
            </button>
          </div>
          {save.error && <ErrorText error={save.error} />}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

export function AccountSecuritySettings() {
  const { formatDate, formatDateTime, formatNumber, t } = useI18n()
  const queryClient = useQueryClient()
  const security = useQuery({ queryKey: ['security-settings'], queryFn: api.securitySettings })
  const [email, setEmail] = useState('')
  const [code, setCode] = useState('')
  const [challenge, setChallenge] = useState<Awaited<ReturnType<typeof api.requestEmailLink>> | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const [passkeyName, setPasskeyName] = useState(() => t('security.defaultPasskeyName'))
  useEffect(() => {
    if (!challenge) return
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [challenge])
  useEffect(() => {
    if (!challenge && security.data?.googleEmail) {
      setEmail(security.data.googleEmail)
    }
  }, [challenge, security.data?.googleEmail])
  const linkRequest = useMutation({
    mutationFn: () => api.requestEmailLink(email),
    onSuccess: (value) => {
      setChallenge(value)
      setCode('')
      setNow(Date.now())
    },
  })
  const linkVerify = useMutation({
    mutationFn: () => api.verifyEmailLink(challenge!.challengeId, code),
    onSuccess: (value: SecuritySettingsData) => {
      queryClient.setQueryData(['security-settings'], value)
      setChallenge(null)
      setEmail('')
      setCode('')
    },
  })
  const register = useMutation({
    mutationFn: async () => {
      const options = await api.startPasskeyRegistration(passkeyName)
      return api.finishPasskeyRegistration(await createPasskey(options))
    },
    onSuccess: async () => {
      setPasskeyName(t('security.defaultPasskeyName'))
      await queryClient.invalidateQueries({ queryKey: ['security-settings'] })
    },
  })
  const revoke = useMutation({
    mutationFn: api.revokePasskey,
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['security-settings'] }),
  })
  const resendSeconds = challenge
    ? Math.max(0, Math.ceil((new Date(challenge.resendAllowedAt).getTime() - now) / 1000))
    : 0
  return (
    <section className="settings-section">
      <h2>{t('security.title')}</h2>
      <p>{t('security.accountNote')}</p>
      <QueryState query={security} empty={t('security.empty')}>
        <>
          <div>
            <h3>{t('security.linkedEmail')}</h3>
            {security.data?.linkedEmails.length ? (
              <ul>{security.data.linkedEmails.map((value) => <li key={value}>{value}</li>)}</ul>
            ) : (
              <p>{t('security.linkBeforePasskey')}</p>
            )}
            {security.data?.googleSignInStatus === 'linked' && security.data.googleEmail && (
              <p>{t('security.googleEmail', { email: maskEmail(security.data.googleEmail) })}</p>
            )}
            {security.data?.googleSignInStatus === 'eligible' && <p>{t('security.googleEligible')}</p>}
            {security.data?.googleSignInStatus === 'unsupported' && <p>{t('security.googleUnsupported')}</p>}
            {security.data?.googleSignInStatus === 'unavailable' && <p>{t('security.googleUnavailable')}</p>}
          </div>
          <div>
            <h3>{t('security.passkeys')}</h3>
            {security.data?.passkeys.length ? (
              <div className="card-list">
                {security.data.passkeys.map((passkey) => (
                  <article className="recipe-card" key={passkey.id}>
                    <strong>{passkey.name}</strong>
                    <span>{t('security.added', { date: formatDate(passkey.createdAt) })}</span>
                    <span>
                      {passkey.lastUsedAt
                        ? t('security.lastUsed', { date: formatDateTime(passkey.lastUsedAt) })
                        : t('security.notUsed')}
                    </span>
                    <button
                      className="button ghost"
                      onClick={() => {
                        if (window.confirm(t('security.revokeConfirm', { name: passkey.name }))) {
                          revoke.mutate(passkey.id)
                        }
                      }}
                    >
                      {t('security.revoke')}
                    </button>
                  </article>
                ))}
              </div>
            ) : (
              <p>{t('security.noPasskeys')}</p>
            )}
          </div>
        </>
      </QueryState>
      <div className="security-grid">
        <div>
          <h3>{t('security.linkEmail')}</h3>
          <label>
            {t('otp.email')}
            <input
              type="email"
              autoComplete="email"
              value={email}
              readOnly={Boolean(security.data?.googleEmail)}
              onChange={(event) => setEmail(event.target.value)}
            />
          </label>
          {security.data?.googleEmail && <p>{t('security.emailMustMatchGoogle')}</p>}
          {challenge && (
            <>
              <p role="status">{t('security.linkSent')}</p>
              <label>
                {t('otp.code')}
                <input
                  autoComplete="one-time-code"
                  inputMode="numeric"
                  pattern="[0-9]{6}"
                  maxLength={6}
                  value={code}
                  onChange={(event) => setCode(event.target.value.replace(/\D/g, '').slice(0, 6))}
                />
              </label>
            </>
          )}
          <div className="auth-actions">
            <button
              className="button secondary"
              disabled={!email || linkRequest.isPending || (challenge !== null && code.length !== 6)}
              onClick={() => (challenge ? linkVerify.mutate() : linkRequest.mutate())}
            >
              {challenge ? t('security.verifyLink') : t('security.sendLink')}
            </button>
            {challenge && (
              <button
                className="button ghost"
                disabled={resendSeconds > 0 || linkRequest.isPending}
                onClick={() => linkRequest.mutate()}
              >
                {resendSeconds > 0
                  ? t('otp.resendIn', { seconds: formatNumber(resendSeconds) })
                  : t('otp.resend')}
              </button>
            )}
          </div>
        </div>
        <div>
          <h3>{t('security.addPasskey')}</h3>
          <label>
            {t('security.passkeyName')}
            <input maxLength={80} value={passkeyName} onChange={(event) => setPasskeyName(event.target.value)} />
          </label>
          <button
            className="button secondary"
            disabled={!passkeyName.trim() || !security.data?.linkedEmails.length || register.isPending}
            onClick={() => register.mutate()}
          >
            {t('security.register')}
          </button>
        </div>
      </div>
      {(linkRequest.error || linkVerify.error || register.error || revoke.error) && (
        <ErrorText error={linkRequest.error ?? linkVerify.error ?? register.error ?? revoke.error} />
      )}
    </section>
  )
}

function maskEmail(value: string) {
  const [local, domain] = value.split('@')
  if (!local || !domain) return value
  return `${local.slice(0, Math.min(2, local.length))}${local.length > 2 ? '***' : '*'}@${domain}`
}

function ProfileSettings({ user }: { user: User }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [nickname, setNickname] = useState(user.nickname ?? '')
  const save = useMutation({
    mutationFn: () => api.saveProfile(nickname.trim() || null),
    onSuccess: async (value) => {
      queryClient.setQueryData(['me'], value)
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['recipes'] }),
        queryClient.invalidateQueries({ queryKey: ['recipe'] }),
        queryClient.invalidateQueries({ queryKey: ['recommendations'] }),
      ])
    },
  })
  return (
    <section className="settings-section">
      <h2>{t('profile.title')}</h2>
      <p>{t('profile.privacy')}</p>
      <form
        className="settings-form"
        onSubmit={(event) => {
          event.preventDefault()
          save.mutate()
        }}
      >
        <label>
          {t('profile.nickname')}
          <input value={nickname} maxLength={40} onChange={(event) => setNickname(event.target.value)} />
        </label>
        <button className="button primary" disabled={save.isPending}>{t('profile.saveProfile')}</button>
        {save.isSuccess && <p role="status">{t('profile.saved')}</p>}
        {save.error && <ErrorText error={save.error} />}
      </form>
    </section>
  )
}

export const COOK_SKILL_URL =
  'https://github.com/vichare/Cook/tree/v0.5.0/skills/cook-recipe-import'
export const COOK_IMPORT_SCHEMA_URL =
  'https://raw.githubusercontent.com/vichare/Cook/v0.5.0/skills/cook-recipe-import/schema/import-draft-v2.schema.json'
export const COOK_AI_RECIPE_SCHEMA_URL =
  'https://raw.githubusercontent.com/vichare/Cook/v0.5.0/skills/cook-recipe-import/schema/ai-recipe-v2.schema.json'
export type TokenPreset = 'import' | 'manage' | 'publish'

function tokenPresetForScopes(scopes: string[]): TokenPreset | null {
  const key = [...scopes].sort().join(',')
  if (key === 'recipes:import') return 'import'
  if (key === 'recipes:images:write,recipes:read,recipes:write') return 'manage'
  if (key === 'recipes:images:write,recipes:publish,recipes:read,recipes:write') return 'publish'
  return null
}

export const importDraftExample = {
  schemaVersion: '2.0',
  sourceUrl: 'https://example.com/recipes/tomato-soup',
  sourceTitle: 'Tomato soup',
  sourceAuthor: 'Example author',
  sourceLicense: 'CC-BY-4.0',
  sourceAttribution: 'Example author — Tomato soup',
  title: 'Tomato soup',
  description: 'A concise description.',
  difficulty: 2,
  estimatedCalories: 900,
  ingredientModel: 'groups-v1',
  ingredientGroups: [
    {
      id: 'tomato-group',
      optional: false,
      note: '',
      sortOrder: 0,
      alternatives: [
        { id: 'tomatoes', name: 'tomatoes', quantityNote: '800 g', sortOrder: 0 },
      ],
    },
    {
      id: 'allium-group',
      optional: false,
      note: 'Use either one.',
      sortOrder: 1,
      alternatives: [
        { id: 'onion', name: 'onion', quantityNote: '1', sortOrder: 0 },
        { id: 'shallots', name: 'shallots', quantityNote: '2', sortOrder: 1 },
      ],
    },
  ],
  steps: ['Chop the ingredients.', 'Simmer until tender, then blend.'],
  warnings: ['Cooking time was inferred from the source.'],
  confidence: 0.9,
}

export const createRecipeExample = {
  title: 'Tomato soup',
  description: 'A concise description.',
  instructions: '1. Chop the ingredients.\n2. Simmer, then blend.',
  visibility: 'private',
  difficulty: 2,
  estimatedCalories: 900,
  sourceUrl: 'https://example.com/recipes/tomato-soup',
  sourceTitle: 'Tomato soup',
  sourceAuthor: 'Example author',
  sourceLicense: 'CC-BY-4.0',
  sourceAttribution: 'Example author — Tomato soup',
  ingredientModel: 'groups-v1',
  ingredientGroups: [
    {
      id: 'tomato-group',
      optional: false,
      note: '',
      sortOrder: 0,
      alternatives: [
        { id: 'tomatoes', name: 'tomatoes', quantityNote: '800 g', sortOrder: 0 },
      ],
    },
    {
      id: 'allium-group',
      optional: false,
      note: 'Use either one.',
      sortOrder: 1,
      alternatives: [
        { id: 'onion', name: 'onion', quantityNote: '1', sortOrder: 0 },
        { id: 'shallots', name: 'shallots', quantityNote: '2', sortOrder: 1 },
      ],
    },
  ],
}

export const updateRecipeExample = {
  ...createRecipeExample,
  title: 'Tomato soup, revised',
  expectedVersion: 3,
}

function requestFormatReference(baseUrl: string) {
  return `Request format / 请求格式

Import a private review draft / 导入私密待审核草稿
POST ${baseUrl}/api/import/drafts
Authorization: Bearer $COOK_IMPORT_TOKEN
Content-Type: application/json

${JSON.stringify(importDraftExample, null, 2)}

Create an owned recipe / 创建自己的菜谱
POST ${baseUrl}/api/ai/recipes
Authorization: Bearer $COOK_IMPORT_TOKEN
Idempotency-Key: stable-source-item-revision-key
Content-Type: application/json

${JSON.stringify(createRecipeExample, null, 2)}

Update an owned recipe / 更新自己的菜谱
PUT ${baseUrl}/api/ai/recipes/{recipeId}
Authorization: Bearer $COOK_IMPORT_TOKEN
Content-Type: application/json

${JSON.stringify(updateRecipeExample, null, 2)}

Cover upload is multipart, not JSON / 封面上传使用 multipart，不是 JSON:
POST ${baseUrl}/api/ai/recipes/{recipeId}/cover
Authorization: Bearer $COOK_IMPORT_TOKEN
If-Match: "recipe-v3"
Content-Type: multipart/form-data
Fields: image=<local JPEG/PNG/WebP>, altText, sourceUrl?, sourceLicense?, sourceAttribution?

Authoritative schemas / 权威 schemas:
${COOK_IMPORT_SCHEMA_URL}
${COOK_AI_RECIPE_SCHEMA_URL}`
}

function deterministicAiPolicy(baseUrl: string) {
  return `At task start, inspect GET ${baseUrl}/api/ai/capabilities once with the bearer token. Inspect it again only before a newly requested capability-sensitive phase when needed, not before every item.

Process one recipe at a time and use a stable Idempotency-Key per source item/revision. Stop the batch on 401/403 or capability mismatch, on 429 (report Retry-After when provided), or after bounded repeated network/5xx failures. For an isolated item 400, provenance, or image failure, record it and continue; never silently skip. On 409, reread and never overwrite or guess a merge. Mark needs_review unless a true idempotent replay is positively identified. Never use last-write-wins.

Final report: created, updated, skipped, failed, and needs_review counts, plus every source item and its recipe ID/URL or non-secret error.

For externally sourced content, preserve sourceUrl, sourceTitle, sourceAuthor, sourceLicense, and sourceAttribution when available. A stated license is metadata, not verified proof. If reliable source/license/attribution is missing, do not directly publish: create a private review draft or private recipe and warn the user.

Difficulty and estimatedCalories are optional and may be omitted or null. If present, difficulty must be an integer from 1 to 5 and estimatedCalories must be an integer from 1 to 100000 representing total kcal for the whole recipe, never per serving.

Upload only local images the user owns or has explicit permission to use. A repository-level license may not cover every image. When uncertain, skip the image and record why. Cook never remotely fetches image URLs.`
}

export function firstTaskForPreset(preset: TokenPreset, locale: 'en' | 'zh-CN') {
  if (locale === 'zh-CN') {
    if (preset === 'publish') return '使用 Cook Skill 检查 token 权限，然后把这个我有权使用的菜谱保存并直接发布；缺少可靠来源/许可证/归属时改为私密并标记待审核。'
    if (preset === 'manage') return '使用 Cook Skill 检查 token 权限，然后把这个我有权使用的菜谱保存为私密菜谱；保留来源信息，不确定的图片请跳过并说明原因。'
    return '使用 Cook Skill 检查 token 权限，然后把这个我有权访问的菜谱 URL 导入为私密待审核草稿。'
  }
  if (preset === 'publish') return 'Use the Cook skill to inspect token capabilities, then save and publish this authorized recipe; if reliable source/license/attribution is missing, keep it private and flag it for review.'
  if (preset === 'manage') return 'Use the Cook skill to inspect token capabilities, then save this authorized recipe as private; preserve provenance and skip any image whose rights are uncertain.'
  return 'Use the Cook skill to inspect token capabilities, then import this authorized recipe URL as a private review draft.'
}

export function resolveCookBaseUrl(
  origin = typeof window === 'undefined' ? 'http://localhost:5173' : window.location.origin,
) {
  const parsed = new URL(origin)
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error('Cook must run from an HTTP or HTTPS origin')
  }
  return parsed.origin
}

export function windowsTemporarySetup(origin = resolveCookBaseUrl()) {
  const baseUrl = resolveCookBaseUrl(origin)
  return `$env:COOK_BASE_URL = '${baseUrl}'
$secureToken = Read-Host 'COOK_IMPORT_TOKEN' -AsSecureString
$env:COOK_IMPORT_TOKEN = [System.Net.NetworkCredential]::new('', $secureToken).Password`
}

export function unixTemporarySetup(origin = resolveCookBaseUrl()) {
  const baseUrl = resolveCookBaseUrl(origin)
  return `export COOK_BASE_URL='${baseUrl}'
printf 'COOK_IMPORT_TOKEN: ' >&2
trap 'stty echo' EXIT HUP INT TERM
stty -echo
IFS= read -r COOK_IMPORT_TOKEN
stty echo
trap - EXIT HUP INT TERM
printf '\\n' >&2
export COOK_IMPORT_TOKEN`
}

export function windowsPersistentBaseUrlSetup(origin = resolveCookBaseUrl()) {
  const baseUrl = resolveCookBaseUrl(origin)
  return `[Environment]::SetEnvironmentVariable('COOK_BASE_URL', '${baseUrl}', 'User')`
}

export function unixPersistentBaseUrlSetup(origin = resolveCookBaseUrl()) {
  const baseUrl = resolveCookBaseUrl(origin)
  return `case "\${SHELL##*/}" in
  zsh) profile="\${ZDOTDIR:-$HOME}/.zshrc" ;;
  bash) profile="$HOME/.bashrc" ;;
  *) profile=''; printf 'Add COOK_BASE_URL to your shell profile manually.\\n' >&2 ;;
esac
if [ -n "$profile" ]; then
  printf "\\nexport COOK_BASE_URL='${baseUrl}'\\n" >> "$profile"
  printf 'Open a new interactive shell, then launch the client from that shell.\\n'
fi`
}

function presetScopeGuidance(preset: TokenPreset) {
  if (preset === 'publish') {
    return `Preset: Manage and publish recipes.
Required scopes: recipes:read, recipes:write, recipes:images:write, recipes:publish.
This high-risk preset may create public recipes, publish owned private recipes, and directly update owned public recipes. It cannot delete recipes or access another account.`
  }
  if (preset === 'manage') {
    return `Preset: Manage private recipes and cover images.
Required scopes: recipes:read, recipes:write, recipes:images:write.
It may manage owned private recipes and propose reviewed revisions for public recipes. It cannot publish or delete recipes.`
  }
  return `Preset: Import drafts only (recommended).
Required scope: recipes:import.
It can only create private review drafts; it cannot read, publish, or directly update recipes.`
}

function presetScopeGuidanceZh(preset: TokenPreset) {
  if (preset === 'publish') {
    return `权限预设：管理并发布菜谱。
所需 scopes：recipes:read、recipes:write、recipes:images:write、recipes:publish。
此高风险预设可创建公开菜谱、发布自己的私密菜谱并直接更新自己的公开菜谱；不能删除菜谱或访问其他账号。`
  }
  if (preset === 'manage') {
    return `权限预设：管理私密菜谱和封面。
所需 scopes：recipes:read、recipes:write、recipes:images:write。
可管理自己的私密菜谱，并对公开菜谱提出待审核 revision；不能发布或删除菜谱。`
  }
  return `权限预设：仅导入草稿（推荐）。
所需 scope：recipes:import。
只能创建私密待审核草稿，不能读取、发布或直接更新菜谱。`
}

export function skillInstructionsForPreset(
  preset: TokenPreset,
  origin = resolveCookBaseUrl(),
) {
  const baseUrl = resolveCookBaseUrl(origin)
  return `English:
GitHub Copilot skill installation:
1. Preview: gh skill preview vichare/Cook cook-recipe-import@v0.5.0
2. Install for cross-session use, pinned to this release:
   gh skill install vichare/Cook cook-recipe-import --agent github-copilot --scope user --pin v0.5.0
3. Restart GitHub Copilot if needed, then verify the skill in Customize → Skills.

Manual fallback: download or copy the entire skills/cook-recipe-import directory, including its schema directory, to ~/.copilot/skills/cook-recipe-import or ~/.agents/skills/cook-recipe-import. Do not copy only SKILL.md.

Human-review/source link (not a paste-to-install URL):
${COOK_SKILL_URL}

COOK_BASE_URL is the trusted Cook site that receives skill API calls. Set it to this exact value:
COOK_BASE_URL=${baseUrl}

GitHub Copilot does not document an arbitrary custom-secret UI for skills. Set COOK_BASE_URL and the separately copied one-time token as OS environment variables before launching the app; restart the app after changing them so it can inherit the values. On Windows PowerShell, use Read-Host -AsSecureString and assign the converted value only to the current process environment so the token is not embedded in command history. On macOS/Linux, use a silent read in the launching shell. The token is intentionally not included here. Never paste it into a prompt, URL, transcript, skill file, reusable command, or log.

${presetScopeGuidance(preset)}

Minimal first use: “${firstTaskForPreset(preset, 'en')}” The import endpoint for this site is ${baseUrl}/api/import/drafts.

简体中文：
GitHub Copilot Skill 安装：
1. 预览：gh skill preview vichare/Cook cook-recipe-import@v0.5.0
2. 安装到用户范围并固定到此 release：
   gh skill install vichare/Cook cook-recipe-import --agent github-copilot --scope user --pin v0.5.0
3. 必要时重启 GitHub Copilot，然后在 Customize → Skills 中确认。

手动备用：下载或复制整个 skills/cook-recipe-import 目录（必须包括 schema 目录）到 ~/.copilot/skills/cook-recipe-import 或 ~/.agents/skills/cook-recipe-import。不要只复制 SKILL.md。

以下链接仅供人工查看来源，不是粘贴即安装的 URL：
${COOK_SKILL_URL}

COOK_BASE_URL 是 Skill 调用 API 时使用的可信 Cook 站点。请设置为以下确定值，无需自行查找或修改：
COOK_BASE_URL=${baseUrl}

GitHub Copilot 目前没有文档化的任意 Skill 自定义 secret UI。启动应用前，请把 COOK_BASE_URL 和单独复制的一次性 token 设置为操作系统环境变量；修改后需重启应用，使其继承新值。Windows PowerShell 可用 Read-Host -AsSecureString，并只把转换后的值赋给当前进程环境，避免 token 出现在命令历史；macOS/Linux 可在启动 shell 中静默读取。可复用说明不会包含真实 token。绝不要把 token 粘贴到 prompt、URL、transcript、Skill 文件、可复用命令或日志中。

${presetScopeGuidanceZh(preset)}

首次使用示例：“${firstTaskForPreset(preset, 'zh-CN')}”此站点的导入端点是 ${baseUrl}/api/import/drafts。

${requestFormatReference(baseUrl)}

${deterministicAiPolicy(baseUrl)}`
}

export function promptForPreset(preset: TokenPreset, origin = resolveCookBaseUrl()) {
  const baseUrl = resolveCookBaseUrl(origin)
  const setup = `Cook base URL / Cook 站点地址:
COOK_BASE_URL=${baseUrl}
Use this exact resolved value; do not ask me to discover or replace a base-URL placeholder. Read the token only from the COOK_IMPORT_TOKEN secret/environment variable. The real token is not included in this prompt.
请直接使用以上已解析的地址，不要要求我查找或替换 base URL 占位符。只从 COOK_IMPORT_TOKEN secret/环境变量读取 token；此 prompt 不包含真实 token。

`
  if (preset === 'publish') {
    return `${setup}English:
Use your browser or transcript tools only for content I am authorized to access. Use ${baseUrl} with the COOK_IMPORT_TOKEN secret/environment variable, and check that the token has recipes:read, recipes:write, and recipes:publish before requesting public visibility; cover changes also require recipes:images:write. With recipes:publish, you may create public recipes, publish my private recipes, and directly update my public recipes, but every update must use the current recipe version and stop on 409 until you reread it. Never delete recipes, access another account, bypass validation, or publish when recipes:publish is absent. Process bulk work one recipe at a time with a distinct Idempotency-Key for each creation. Do not silently skip failures. At the end, report created, updated, skipped, and failed counts with each recipe ID or URL and error. AI output becomes public immediately and may contain errors, copyrighted material, or private information, so preserve source/license/attribution, confirm image rights before uploading local JPEG/PNG/WebP bytes, and never fetch a source image automatically. Never print, quote, log, download, or place the token in a URL or prompt.

简体中文：
仅使用浏览器或 transcript 工具处理我有权访问的内容。使用 ${baseUrl}，并从 secret store/环境变量读取 COOK_IMPORT_TOKEN；请求公开可见性前，必须确认 token 同时具有 recipes:read、recipes:write 和 recipes:publish，修改封面还需要 recipes:images:write。有 recipes:publish 时，可以创建公开菜谱、把我的私密菜谱发布为公开，并直接更新我的公开菜谱；每次更新必须使用当前菜谱 version，遇到 409 必须重新读取后再处理。绝不能删除菜谱、访问其他账号、绕过校验，或在没有 recipes:publish 时发布。批量操作应逐条处理，每个创建使用独立 Idempotency-Key；不得悄悄跳过失败项。结束时输出 created、updated、skipped、failed 数量，以及每项菜谱 ID 或 URL 和错误。AI 输出会立即公开，可能包含错误、受版权保护材料或隐私信息，因此必须保留来源、许可和归属；上传本地 JPEG/PNG/WebP 图片前确认使用权，绝不自动抓取来源图片。绝不要打印、引用、记录、下载 token，也不要把 token 放进 URL 或 prompt。

${requestFormatReference(baseUrl)}

${deterministicAiPolicy(baseUrl)}`
  }
  if (preset === 'manage') {
    return `${setup}English:
Use your browser or transcript tools only for content I am authorized to access. Prefer schema.org Recipe/JSON-LD and legitimately available captions/transcripts. Do not bypass login, paywalls, access controls, robots restrictions, or DRM. Use ${baseUrl} with the COOK_IMPORT_TOKEN secret/environment variable to read my recipes and versions, create private recipes, or update my private recipes with optimistic concurrency. Updates to public recipes must create a pending revision for my review and must never publish directly. Before uploading a local JPEG, PNG, or WebP cover image, ask me to confirm that I own it or have permission, then provide accurate alt text and any source/license/attribution. Never fetch or copy a source site's image automatically. Never print, quote, log, download, or place the token in a URL or prompt.

简体中文：
仅使用浏览器或 transcript 工具处理我有权访问的内容。优先使用 schema.org Recipe/JSON-LD 和合法可用的字幕或 transcript。不得绕过登录、付费墙、访问控制、robots 限制或 DRM。使用 ${baseUrl}，并从 secret store/环境变量读取 COOK_IMPORT_TOKEN，以读取我的菜谱及版本、创建私密菜谱，或通过乐观并发更新我的私密菜谱。公开菜谱的更新必须创建待审核 revision，绝不能直接发布。上传本地 JPEG、PNG 或 WebP 封面前，先请我确认拥有或获准使用，并提供准确的替代文字及来源、许可、归属信息。绝不要自动抓取或复制来源网站图片。绝不要打印、引用、记录、下载 token，也不要把 token 放进 URL 或 prompt。

${requestFormatReference(baseUrl)}

${deterministicAiPolicy(baseUrl)}`
  }
  return `${setup}English:
Use your browser tools to extract a recipe from a webpage I am authorized to access, preferring schema.org Recipe/JSON-LD. For video, use only legitimately available captions/transcripts; if unavailable, ask me for a transcript. Do not bypass login, paywalls, robots restrictions, access controls, or DRM. Rewrite concise structured ingredients and steps, preserve source URL/title/author, and record uncertainty as warnings. Preserve a stated 1–5 difficulty; include estimatedCalories only as an explicitly estimated whole-recipe total, never per serving or medical advice. Validate against the Cook import draft schema, then create a private review draft at ${baseUrl}/api/import/drafts using the recipes:import credential from the COOK_IMPORT_TOKEN secret/environment variable. Never print, quote, log, download, or place that token in a URL or prompt. Do not publish or finalize the recipe; tell me to review the private draft in Cook.

简体中文：
请使用浏览器工具，从我有权访问的网页提取菜谱，优先使用 schema.org Recipe/JSON-LD。视频只能使用合法可用的字幕或 transcript；若无法取得，请要求我提供 transcript。不得绕过登录、付费墙、robots 限制、访问控制或 DRM。请将食材和步骤简洁结构化并改写，保留来源 URL、标题和作者，把不确定内容记录为 warnings。保留来源明确给出的 1–5 难度；estimatedCalories 只能表示明确标注为估算的整份总热量，不能当作每份或医疗建议。按 Cook 导入草稿 schema 验证后，使用 secret store/环境变量中的 COOK_IMPORT_TOKEN（recipes:import 权限）调用 ${baseUrl}/api/import/drafts，创建私密待审核草稿。绝不要打印、引用、记录、下载 token，也不要把 token 放进 URL 或 prompt。不要发布或完成菜谱；请提醒我在 Cook 中人工审核私密草稿。

${requestFormatReference(baseUrl)}

${deterministicAiPolicy(baseUrl)}`
}

export function ImportTokenSettings() {
  const { formatDateTime, locale, t } = useI18n()
  const queryClient = useQueryClient()
  const baseUrl = useMemo(() => resolveCookBaseUrl(), [])
  const tokens = useQuery({ queryKey: ['import-tokens'], queryFn: api.importTokens })
  const [name, setName] = useState(() => t('token.defaultName'))
  const [preset, setPreset] = useState<TokenPreset>('import')
  const [setupPreset, setSetupPreset] = useState<TokenPreset>('import')
  const [expiresInDays, setExpiresInDays] = useState<30 | 90 | null>(90)
  const [secret, setSecret] = useState<string | null>(null)
  const baseUrlAssignment = `COOK_BASE_URL=${baseUrl}`
  const copySequence = useRef(0)
  const [copyNotice, setCopyNotice] = useState<{
    id: number
    kind: 'success' | 'error'
    message: string
  } | null>(null)
  const connection = useMutation({
    mutationFn: () => {
      if (!secret) throw new Error('The one-time token is no longer available')
      return api.aiCapabilities(secret)
    },
  })
  const create = useMutation({
    mutationFn: (input: {
      name: string
      expiresInDays: 30 | 90 | null
      preset: TokenPreset
    }) => api.createImportToken(input.name, input.expiresInDays, input.preset),
    onSuccess: async (created, input) => {
      setSecret(created.secret)
      setSetupPreset(input.preset)
      setCopyNotice(null)
      connection.reset()
      await queryClient.invalidateQueries({ queryKey: ['import-tokens'] })
    },
  })
  const revoke = useMutation({
    mutationFn: api.revokeImportToken,
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['import-tokens'] }),
  })
  function copyOutput(value: string, success: string) {
    void copyText(value)
      .then(() => setCopyNotice({
        id: ++copySequence.current,
        kind: 'success',
        message: success,
      }))
      .catch(() => {
        setCopyNotice({
          id: ++copySequence.current,
          kind: 'error',
          message: t('token.copyFailed'),
        })
      })
  }
  return (
    <section className="settings-section">
      <h2>{t('token.title')}</h2>
      <p>{t('token.securityNote')}</p>
      <form
        className="inline-form compact"
        onSubmit={(event) => {
          event.preventDefault()
          if (preset === 'publish' && !window.confirm(t('token.publishConfirm'))) return
          create.mutate({ name, expiresInDays, preset })
        }}
      >
        <label>
          {t('token.name')}
          <input value={name} onChange={(event) => setName(event.target.value)} required maxLength={80} />
        </label>
        <fieldset>
          <legend>{t('token.accessPreset')}</legend>
          <label className="choice-card">
            <input
              type="radio"
              name="token-preset"
              value="import"
              checked={preset === 'import'}
              onChange={() => {
                setPreset('import')
                setSetupPreset('import')
              }}
            />
            <span>
              <strong>{t('token.presetImport')}</strong>
              <small>{t('token.presetImportDetail')}</small>
            </span>
          </label>
          <label className="choice-card">
            <input
              type="radio"
              name="token-preset"
              value="manage"
              checked={preset === 'manage'}
              onChange={() => {
                setPreset('manage')
                setSetupPreset('manage')
              }}
            />
            <span>
              <strong>{t('token.presetManage')}</strong>
              <small>{t('token.presetManageDetail')}</small>
            </span>
          </label>
          <label className="choice-card">
            <input
              type="radio"
              name="token-preset"
              value="publish"
              checked={preset === 'publish'}
              onChange={() => {
                setPreset('publish')
                setSetupPreset('publish')
                setExpiresInDays(30)
              }}
            />
            <span>
              <strong>{t('token.presetPublish')}</strong>
              <small>{t('token.presetPublishDetail')}</small>
            </span>
          </label>
        </fieldset>
        <section className="source-note" aria-labelledby="token-setup-flow">
          <h3 id="token-setup-flow">{t('token.setupFlow')}</h3>
          <ol>
            <li>{t('token.setupCreate')}</li>
            <li>{t('token.setupStore')}</li>
            <li>{t('token.setupInstall')}</li>
            <li>{t('token.setupTest')}</li>
            <li>{t('token.setupFirstTask')}</li>
          </ol>
        </section>
        <details className="source-note">
          <summary>{t('token.requestFormat')}</summary>
          <p>{t('token.requestFormatDetail')}</p>
          <h4>{t('token.draftExample')}</h4>
          <pre><code>{JSON.stringify(importDraftExample, null, 2)}</code></pre>
          <button
            type="button"
            className="button ghost"
            onClick={() => copyOutput(JSON.stringify(importDraftExample, null, 2), t('token.exampleCopied'))}
          >
            {t('token.copyExample')}
          </button>
          <a href={COOK_IMPORT_SCHEMA_URL} target="_blank" rel="noreferrer">
            {t('token.openDraftSchema')}
          </a>
          <h4>{t('token.recipeExamples')}</h4>
          <pre><code>{JSON.stringify(createRecipeExample, null, 2)}</code></pre>
          <pre><code>{JSON.stringify(updateRecipeExample, null, 2)}</code></pre>
          <button
            type="button"
            className="button ghost"
            onClick={() => copyOutput(
              `${JSON.stringify(createRecipeExample, null, 2)}\n\n${JSON.stringify(updateRecipeExample, null, 2)}`,
              t('token.exampleCopied'),
            )}
          >
            {t('token.copyExample')}
          </button>
          <a href={COOK_AI_RECIPE_SCHEMA_URL} target="_blank" rel="noreferrer">
            {t('token.openRecipeSchema')}
          </a>
          <p>{t('token.coverMultipart')}</p>
        </details>
        {preset === 'publish' && (
          <p className="expiry-warning" role="alert">{t('token.publishRisk')}</p>
        )}
        <label>
          {t('token.expiry')}
          <select
            value={expiresInDays ?? 'none'}
            onChange={(event) => {
              const value = event.target.value
              setExpiresInDays(value === 'none' ? null : value === '30' ? 30 : 90)
            }}
          >
            <option value="30">{t('token.expiry30')}</option>
            <option value="90">{t('token.expiry90')}</option>
            <option value="none">{t('token.expiryNone')}</option>
          </select>
        </label>
        {preset === 'publish' && expiresInDays === null && (
          <p className="expiry-warning" role="alert">{t('token.publishNeverExpires')}</p>
        )}
        <button className="button primary" disabled={create.isPending}>{t('token.create')}</button>
      </form>
      {secret && (
        <div className="secret-once">
          <strong role="status">{t('token.copyNow')}</strong>
          <code>{secret}</code>
          <button
            className="button secondary"
            aria-label={t('token.copyToken')}
            onClick={() => copyOutput(secret, t('token.tokenCopied'))}
          >
            {t('token.copyToken')}
          </button>
          <button
            type="button"
            className="button secondary"
            disabled={connection.isPending}
            onClick={() => connection.mutate()}
          >
            {connection.isPending ? t('token.testingConnection') : t('token.testConnection')}
          </button>
          {connection.data && (
            <div className="notice" role="status">
              <strong>{t('token.connectionSuccess')}</strong>
              <p>{t('token.connectionScopes', { scopes: connection.data.scopes.join(', ') })}</p>
              <p>
                {connection.data.expiresAt
                  ? t('token.connectionExpiry', { date: formatDateTime(connection.data.expiresAt) })
                  : t('token.connectionNoExpiry')}
              </p>
            </div>
          )}
          {connection.error && (
            <p className="error" role="alert">{t('token.connectionFailed')}</p>
          )}
          <button className="button primary" onClick={() => {
            setSecret(null)
          }}>
            {t('token.savedClose')}
          </button>
        </div>
      )}
      <section className="source-note" aria-labelledby="token-client-setup">
        <h3 id="token-client-setup">{t('token.clientSetup')}</h3>
        <p>{t('token.clientSetupDetail')}</p>
        <p>{t('token.secretSeparate')}</p>
        <label>
          {t('token.baseUrlLabel')}
          <input readOnly value={baseUrlAssignment} />
        </label>
        <p>{t('token.baseUrlDetail', { baseUrl })}</p>
        <button
          type="button"
          className="button ghost"
          onClick={() => copyOutput(baseUrlAssignment, t('token.baseUrlCopied'))}
        >
          {t('token.copyBaseUrl')}
        </button>
        <details className="client-setup-details">
          <summary>{t('token.configureEnvironment')}</summary>
          <p>{t('token.configureEnvironmentDetail')}</p>
          <h4>{t('token.githubCopilotSetup')}</h4>
          <p>{t('token.githubCopilotSetupDetail')}</p>
          <h4>{t('token.genericClientSetup')}</h4>
          <p>{t('token.genericClientSetupDetail')}</p>
          <h4>{t('token.temporarySetup')}</h4>
          <p>{t('token.temporarySetupDetail')}</p>
          <h5>{t('token.windowsPowerShell')}</h5>
          <pre><code>{windowsTemporarySetup(baseUrl)}</code></pre>
          <button
            type="button"
            className="button ghost"
            onClick={() => copyOutput(windowsTemporarySetup(baseUrl), t('token.temporaryCommandsCopied'))}
          >
            {t('token.copyWindowsTemporary')}
          </button>
          <h5>{t('token.macosLinux')}</h5>
          <pre><code>{unixTemporarySetup(baseUrl)}</code></pre>
          <button
            type="button"
            className="button ghost"
            onClick={() => copyOutput(unixTemporarySetup(baseUrl), t('token.temporaryCommandsCopied'))}
          >
            {t('token.copyUnixTemporary')}
          </button>
          <details>
            <summary>{t('token.persistentSetup')}</summary>
            <p>{t('token.persistentSetupDetail')}</p>
            <h5>{t('token.windowsPowerShell')}</h5>
            <pre><code>{windowsPersistentBaseUrlSetup(baseUrl)}</code></pre>
            <button
              type="button"
              className="button ghost"
              onClick={() => copyOutput(
                windowsPersistentBaseUrlSetup(baseUrl),
                t('token.persistentBaseUrlCopied'),
              )}
            >
              {t('token.copyWindowsPersistent')}
            </button>
            <h5>{t('token.macosLinux')}</h5>
            <pre><code>{unixPersistentBaseUrlSetup(baseUrl)}</code></pre>
            <button
              type="button"
              className="button ghost"
              onClick={() => copyOutput(
                unixPersistentBaseUrlSetup(baseUrl),
                t('token.persistentBaseUrlCopied'),
              )}
            >
              {t('token.copyUnixPersistent')}
            </button>
          </details>
        </details>
        <h4>{t('token.installSkill')}</h4>
        <p>{t('token.installSkillDetail')}</p>
        <p>{t('token.skillMainNote')}</p>
        <label>
          {t('token.skillUrlLabel')}
          <input readOnly value={COOK_SKILL_URL} />
        </label>
        <a className="button secondary" href={COOK_SKILL_URL} rel="noreferrer" target="_blank">
          {t('token.reviewSkillSource')}
        </a>
        <button
          type="button"
          className="button ghost"
          onClick={() => copyOutput(
            skillInstructionsForPreset(setupPreset, baseUrl),
            t('token.installCopied'),
          )}
        >
          {t('token.copyInstall')}
        </button>
        <h4>{t('token.firstTask')}</h4>
        <p>{firstTaskForPreset(setupPreset, locale)}</p>
        <button
          type="button"
          className="button ghost"
          onClick={() => copyOutput(
            firstTaskForPreset(setupPreset, locale),
            t('token.firstTaskCopied'),
          )}
        >
          {t('token.copyFirstTask')}
        </button>
        <details>
          <summary>{t('token.detailedRules')}</summary>
          <p>{t('token.bulkRules')}</p>
          <p>{t('token.imageRules')}</p>
        </details>
        <h4>{t('token.copyPrompt')}</h4>
        <p>{t('token.promptDetail')}</p>
        <button
          type="button"
          className="button ghost"
          onClick={() => copyOutput(
            promptForPreset(setupPreset, baseUrl),
            t('token.promptCopied'),
          )}
        >
          {t('token.copyPrompt')}
        </button>
      </section>
      {copyNotice?.kind === 'success' && (
        <p key={copyNotice.id} role="status" aria-live="polite">
          {copyNotice.message}
        </p>
      )}
      {copyNotice?.kind === 'error' && (
        <p key={copyNotice.id} className="error" role="alert">
          {copyNotice.message}
        </p>
      )}
      <QueryState query={tokens} empty={t('token.empty')}>
        <div className="card-list">
          {tokens.data?.map((token) => {
            const tokenPreset = tokenPresetForScopes(token.scopes)
            const tokenPresetLabel = tokenPreset === 'publish'
              ? t('token.presetPublish')
              : tokenPreset === 'manage'
                ? t('token.presetManage')
                : tokenPreset === 'import'
                  ? t('token.presetImport')
                  : null
            return (
              <article className="recipe-card" key={token.id}>
              <strong>{token.name}</strong>
              <span>{token.prefix}… · {token.scopes.join(', ')}</span>
              {tokenPresetLabel && <span>{t('token.tokenPreset', { preset: tokenPresetLabel })}</span>}
              <span>{t('token.created', { date: formatDateTime(token.createdAt) })}</span>
              <span>
                {token.lastUsedAt
                  ? t('token.lastUsed', { date: formatDateTime(token.lastUsedAt) })
                  : t('token.neverUsed')}
              </span>
              <span>
                {token.revokedAt
                  ? t('token.revoked')
                  : t('token.expires', {
                      date: token.expiresAt ? formatDateTime(token.expiresAt) : t('common.never'),
                    })}
              </span>
              {!token.revokedAt && (
                <>
                  {tokenPreset && (
                    <button
                      type="button"
                      className="button ghost"
                      aria-label={t('token.useSetupPresetLabel', {
                        name: token.name,
                        prefix: token.prefix,
                        preset: tokenPresetLabel ?? '',
                      })}
                      onClick={() => {
                        setPreset(tokenPreset)
                        setSetupPreset(tokenPreset)
                      }}
                    >
                      {t('token.useSetupPreset')}
                    </button>
                  )}
                  <button
                    className="button ghost"
                    onClick={() => {
                      if (window.confirm(t('token.revokeConfirm', { name: token.name }))) {
                        revoke.mutate(token.id)
                      }
                    }}
                  >
                    {t('token.revoke')}
                  </button>
                </>
              )}
            </article>
            )
          })}
        </div>
      </QueryState>
      {create.error && <ErrorText error={create.error} />}
      {revoke.error && <ErrorText error={revoke.error} />}
    </section>
  )
}

function ImportsPage() {
  const { t } = useI18n()
  const drafts = useQuery({ queryKey: ['import-drafts'], queryFn: api.importDrafts })
  return (
    <Page title={t('imports.title')} intro={t('imports.intro')}>
      <QueryState query={drafts} empty={t('imports.empty')}>
        <div className="card-list">
          {drafts.data?.map((draft) => (
            <article className="recipe-card" key={draft.id}>
              <span className="status verify">{importStatusLabel(draft.status, t)}</span>
              <h2>{draft.title}</h2>
              <p>{draft.sourceTitle ?? draft.sourceUrl}</p>
              <Link to="/imports/$draftId" params={{ draftId: draft.id }}>{t('imports.review')}</Link>
            </article>
          ))}
        </div>
      </QueryState>
    </Page>
  )
}

function ImportDraftPage() {
  const { formatNumber, t } = useI18n()
  const { draftId } = importDraftRoute.useParams()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const draft = useQuery({ queryKey: ['import-draft', draftId], queryFn: () => api.importDraft(draftId) })
  const accept = useMutation({
    mutationFn: () => api.acceptImportDraft(draftId),
    onSuccess: async (recipe) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['import-drafts'] }),
        queryClient.invalidateQueries({ queryKey: ['recipes'] }),
      ])
      await navigate({ to: '/recipes/$recipeId', params: { recipeId: recipe.id } })
    },
  })
  const discard = useMutation({
    mutationFn: () => api.discardImportDraft(draftId),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['import-drafts'] })
      await navigate({ to: '/imports' })
    },
  })
  return (
    <Page title={t('import.reviewTitle')} intro={t('import.reviewIntro')}>
      <QueryState query={draft} empty={t('import.notFound')}>
        {draft.data && (
          <>
            <ImportDraftEditor draft={draft.data} />
            <section className="source-note">
              <h2>{t('import.sourceWarnings')}</h2>
              <a href={draft.data.sourceUrl} rel="noreferrer">{draft.data.sourceTitle ?? draft.data.sourceUrl}</a>
              {draft.data.sourceAuthor && (
                <p>{t('import.sourceAuthor', { author: draft.data.sourceAuthor })}</p>
              )}
              {draft.data.sourceAttribution && <p>{draft.data.sourceAttribution}</p>}
              {draft.data.sourceLicense && (
                <p>{t('recipe.sourceLicenseValue', { license: draft.data.sourceLicense })}</p>
              )}
              {(draft.data.sourceLicense || draft.data.sourceAttribution) && (
                <small>{t('recipe.sourceLicenseUnverified')}</small>
              )}
              <p>
                {t('import.confidence', {
                  confidence: draft.data.confidence === null
                    ? t('common.notProvided')
                    : formatNumber(draft.data.confidence),
                })}
              </p>
              {draft.data.warnings.map((warning) => <p key={warning}>{warning}</p>)}
            </section>
            {draft.data.status === 'draft' && (
              <div className="dialog-actions">
                <button
                  className="button ghost"
                  onClick={() => {
                    if (window.confirm(t('import.discardConfirm'))) {
                      discard.mutate()
                    }
                  }}
                >
                  {t('import.discard')}
                </button>
                <button className="button primary" onClick={() => accept.mutate()}>{t('import.accept')}</button>
              </div>
            )}
            {(accept.error || discard.error) && <ErrorText error={accept.error ?? discard.error} />}
          </>
        )}
      </QueryState>
    </Page>
  )
}

function ImportDraftEditor({ draft }: { draft: ImportDraft }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const form = useForm<RecipeForm>({
    resolver: zodResolver(recipeSchema(t)),
    defaultValues: {
      title: draft.title,
      description: draft.description,
      instructions: draft.instructions,
      visibility: 'private',
      difficulty: draft.difficulty,
      estimatedCalories: draft.estimatedCalories,
      sourceUrl: '',
      sourceTitle: '',
      sourceAuthor: '',
      sourceLicense: '',
      sourceAttribution: '',
      ingredientGroups: recipeIngredientGroups(draft),
    },
  })
  const save = useMutation({
    mutationFn: (value: RecipeForm) =>
      api.saveImportDraft(draft.id, {
        title: value.title,
        description: value.description,
        instructions: value.instructions,
        difficulty: value.difficulty,
        estimatedCalories: value.estimatedCalories,
        ingredientModel: 'groups-v1',
        ingredientGroups: normalizeIngredientGroupOrder(value.ingredientGroups),
      }),
    onSuccess: (value) => queryClient.setQueryData(['import-draft', draft.id], value),
  })
  if (draft.status !== 'draft') {
    return (
      <StatePanel
        title={t('import.closed')}
        detail={t('import.closedDetail', { status: importStatusLabel(draft.status, t) })}
      />
    )
  }
  return (
    <form className="editor-form" onSubmit={form.handleSubmit((value) => save.mutate(value))}>
      <label>{t('recipe.title')}<input {...form.register('title')} /></label>
      <label>{t('recipe.description')}<textarea rows={3} {...form.register('description')} /></label>
      <MarkdownEditor
        id="import-instructions"
        label={t('recipe.instructions')}
        value={form.watch('instructions')}
        onChange={(instructions) => form.setValue('instructions', instructions, { shouldDirty: true })}
        maxLength={20000}
      />
      <RecipeMetadataFields form={form} />
      <IngredientGroupsEditor form={form} />
      <button className="button secondary" disabled={save.isPending}>{t('import.saveChanges')}</button>
      {save.isSuccess && <p role="status">{t('import.saved')}</p>}
      {save.error && <ErrorText error={save.error} />}
    </form>
  )
}

function SettingsForm({ initial }: { initial: RecommendationSettings }) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const form = useForm<RecommendationSettings>({ defaultValues: initial })
  const save = useMutation({
    mutationFn: api.saveSettings,
    onSuccess: (value) => queryClient.setQueryData(['settings'], value),
  })
  return (
    <form className="settings-form" onSubmit={form.handleSubmit((value) => save.mutate(value))}>
      <label className="check-row">
        <input type="checkbox" {...form.register('includeFavoriteRecipes')} /> {t('sources.favoriteRecipes')}
      </label>
      <label className="check-row">
        <input type="checkbox" {...form.register('includeOwn')} /> {t('sources.own')}
      </label>
      <label className="check-row">
        <input type="checkbox" {...form.register('includeFavoriteAccounts')} /> {t('sources.favoriteAccounts')}
      </label>
      <label className="check-row">
        <input type="checkbox" {...form.register('includeOtherPublic')} /> {t('sources.otherPublic')}
      </label>
      <label>
        {t('sources.recheck')}
        <input type="number" min={1} max={90} {...form.register('verificationDays', { valueAsNumber: true })} />
      </label>
      <button className="button primary" disabled={save.isPending}>
        {t('sources.save')}
      </button>
      {save.isSuccess && <p role="status">{t('common.saved')}</p>}
      {save.error && <ErrorText error={save.error} />}
    </form>
  )
}

export function PantryConfirmation({
  open,
  onOpenChange,
  names,
  onSaved,
}: {
  open: boolean
  onOpenChange: (value: boolean) => void
  names: string[]
  onSaved?: () => void
}) {
  const { t } = useI18n()
  const queryClient = useQueryClient()
  const [answers, setAnswers] = useState<Record<string, 'have' | 'buy' | 'used'>>({})
  const save = useMutation({
    mutationFn: () =>
      api.savePantry(
        Object.entries(answers).map(([name, answer]) => ({
          name,
          status: answer === 'have' ? ('have' as const) : ('out' as const),
          quantityNote: answer === 'buy' ? 'Can buy' : answer === 'used' ? 'Used up' : '',
        })),
      ),
    onSuccess: async (items) => {
      queryClient.setQueryData(['pantry'], items)
      await queryClient.invalidateQueries({ queryKey: ['recommendations'] })
      onSaved?.()
      onOpenChange(false)
      setAnswers({})
    },
  })
  return (
    <Dialog.Root
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) setAnswers({})
        onOpenChange(nextOpen)
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="dialog-overlay" />
        <Dialog.Content className="confirmation-sheet">
          <Dialog.Title>{t('confirm.title')}</Dialog.Title>
          <Dialog.Description>
            {t('confirm.description')}
          </Dialog.Description>
          <div className="confirmation-list">
            {names.map((name) => (
              <fieldset key={name}>
                <legend>{name}</legend>
                {[
                  ['have', t('confirm.have')],
                  ['buy', t('confirm.buy')],
                  ['used', t('confirm.used')],
                ].map(([value, label]) => (
                  <label className="radio-pill" key={value}>
                    <input
                      type="radio"
                      name={`confirm-${name}`}
                      value={value}
                      checked={answers[name] === value}
                      onChange={() => setAnswers({ ...answers, [name]: value as 'have' | 'buy' | 'used' })}
                    />
                    {label}
                  </label>
                ))}
              </fieldset>
            ))}
          </div>
          <div className="dialog-actions">
            <Dialog.Close asChild>
              <button className="button ghost">{t('confirm.skip')}</button>
            </Dialog.Close>
            <button className="button primary" disabled={Object.keys(answers).length === 0 || save.isPending} onClick={() => save.mutate()}>
              {t('confirm.save')}
            </button>
          </div>
          {save.error && <ErrorText error={save.error} />}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

function Page({
  title,
  intro,
  action,
  children,
}: {
  title: string
  intro: string
  action?: React.ReactNode
  children: React.ReactNode
}) {
  const { t } = useI18n()
  return (
    <div className="page">
      <header className="page-header">
        <div>
          <p className="eyebrow">{t('app.workspace')}</p>
          <h1>{title}</h1>
          {intro && <p>{intro}</p>}
        </div>
        {action}
      </header>
      {children}
    </div>
  )
}

function QueryState<T>({
  query,
  empty,
  children,
}: {
  query: { isLoading: boolean; error: Error | null; data?: T[] | T }
  empty: string
  children: React.ReactNode
}) {
  const { t } = useI18n()
  if (query.isLoading) return <StatePanel title={t('common.loading')} detail={t('common.loadingDetail')} />
  if (query.error) return <StatePanel title={t('common.errorTitle')} detail={errorMessage(query.error, t)} />
  if (Array.isArray(query.data) && query.data.length === 0) {
    return <StatePanel title={t('common.emptyTitle')} detail={empty} />
  }
  return children
}

function StatePanel({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="state-panel" role="status">
      <h1>{title}</h1>
      <p>{detail}</p>
    </div>
  )
}

function Metric({
  label,
  value,
  to,
  search,
}: {
  label: string
  value: string | number
  to: '/recipes' | '/pantry'
  search?: { scope?: 'mine'; status?: PantryStatus }
}) {
  return (
    <Link
      className="metric"
      to={to}
      search={search ?? {}}
      aria-label={`${label}: ${value}`}
    >
      <strong aria-hidden="true">{value}</strong>
      <span>{label}</span>
    </Link>
  )
}

function IngredientSummary({ label, items }: { label: string; items: string[] }) {
  if (items.length === 0) return null
  return (
    <div className="ingredient-summary">
      <strong>{label}</strong>
      <ul className="ingredient-summary-list">
        {items.map((item) => <li key={item}>{item}</li>)}
      </ul>
    </div>
  )
}

type RecommendationGroupRow = {
  id: string
  status: 'available' | 'verify' | 'missing'
  optional: boolean
  note: string
  alternatives: Array<IngredientReference & { id: string; quantityNote: string }>
  inShoppingList: boolean
}

function recommendationGroupRows(recommendation: Recommendation): RecommendationGroupRow[] {
  const groups = recipeIngredientGroups(recommendation.recipe)
  return groups.flatMap((group) => {
    const reason = recommendation.ingredientGroupReasons?.find((item) => item.groupId === group.id)
    if (reason && !reason.effective) return []
    const names = group.alternatives.map((alternative) => normalizeIngredient(alternative.name))
    const legacyIncludes = (values: string[]) => values.some(
      (value) => names.includes(normalizeIngredient(value)),
    )
    const status = reason
      ? (reason.code === 'AVAILABLE' ? 'available'
        : reason.code === 'MISSING' || reason.code === 'OUT' ? 'missing'
          : 'verify')
      : legacyIncludes(recommendation.available) ? 'available'
        : legacyIncludes(recommendation.verify) ? 'verify'
          : 'missing'
    return [{
      id: group.id,
      status,
      optional: group.optional,
      note: group.note,
      alternatives: group.alternatives,
      inShoppingList: legacyIncludes(recommendation.inShoppingList),
    }]
  })
}

function RecommendationIngredientGroups({
  recommendation,
  shoppingItems,
  pendingIdentity,
  onAdd,
}: {
  recommendation: Recommendation
  shoppingItems: Array<{
    name: string
    canonicalIngredientId: string | null
    choiceAlternatives?: Array<IngredientReference & { id: string }>
  }>
  pendingIdentity: string | null
  onAdd: (
    ingredient: IngredientReference,
    groupId?: string | null,
    alternativeId?: string | null,
  ) => void
}) {
  const { t } = useI18n()
  const rows = recommendationGroupRows(recommendation)
  const [choices, setChoices] = useState<Record<string, string>>({})
  return (
    <>
      {(['available', 'verify', 'missing'] as const).map((status) => {
        const statusRows = rows.filter((row) => row.status === status)
        if (statusRows.length === 0) return null
        const label = t(`recommend.${status}`)
        return (
          <div className="ingredient-summary" aria-label={label} key={status}>
            <strong>{label}</strong>
            <ul className="ingredient-summary-list">
              {statusRows.map((row) => {
                const reason = recommendation.ingredientGroupReasons
                  ?.find((item) => item.groupId === row.id)
                const selectedId = choices[row.id] ?? reason?.selectedAlternativeId ?? ''
                const selected = row.alternatives.find((alternative) => alternative.id === selectedId)
                  ?? (row.alternatives.length === 1 ? row.alternatives[0] : undefined)
                const fallback = selected ?? row.alternatives[0]
                const activeIdentities = shoppingItems.flatMap((item) => [
                  ingredientIdentity(item),
                  ...(item.choiceAlternatives ?? []).map(ingredientIdentity),
                ])
                const inShoppingList = row.inShoppingList || row.alternatives.some(
                  (alternative) => activeIdentities.includes(ingredientIdentity(alternative)),
                )
                const isStructured = recommendation.recipe.ingredientModel === 'groups-v1'
                const pendingKey = isStructured
                  ? `g:${row.id}:${selected?.id ?? 'any'}`
                  : ingredientIdentity(fallback)
                const pending = pendingIdentity === pendingKey
                const groupName = row.alternatives.map((alternative) => alternative.name)
                  .join(` ${t('recipe.or')} `)
                return (
                  <li key={row.id} className="recommendation-ingredient-group">
                    <span>
                      {groupName}
                      {row.optional && <> <span className="optional-badge">{t('recipe.optional')}</span></>}
                    </span>
                    {row.note && <small>{row.note}</small>}
                    {status === 'missing' && inShoppingList && (
                      <span className="shopping-membership-status">
                        <span className="sr-only">{t(
                          row.alternatives.length === 1 ? 'shopping.inList' : 'shopping.groupInList',
                          { name: groupName },
                        )}</span>
                        <span aria-hidden="true">{t('shopping.inListShort')}</span>
                      </span>
                    )}
                    {status === 'missing' && !inShoppingList && fallback && (
                      <div className="shopping-choice">
                        {row.alternatives.length > 1 && (
                          <label>
                            <span className="sr-only">{t('shopping.chooseAlternative', { name: groupName })}</span>
                            <select
                              value={selected?.id ?? ''}
                              onChange={(event) => setChoices({
                                ...choices,
                                [row.id]: event.target.value,
                              })}
                            >
                              <option value="">{t('shopping.anyAlternative')}</option>
                              {row.alternatives.map((alternative) => (
                                <option key={alternative.id} value={alternative.id}>
                                  {alternative.name}
                                </option>
                              ))}
                            </select>
                          </label>
                        )}
                        <button
                          type="button"
                          className="button ghost"
                          disabled={pending}
                          aria-label={pending
                            ? t('shopping.addingMissing', { name: selected?.name ?? groupName })
                            : t('shopping.addMissing', { name: selected?.name ?? groupName })}
                          onClick={() => onAdd(
                            selected ? asIngredientReference(selected) : {
                              name: groupName,
                              canonicalIngredientId: null,
                            },
                            isStructured ? row.id : null,
                            selected?.id ?? null,
                          )}
                        >
                          {pending
                            ? t('shopping.addingMissingShort')
                            : t('shopping.addMissingShort')}
                        </button>
                      </div>
                    )}
                  </li>
                )
              })}
            </ul>
          </div>
        )
      })}
    </>
  )
}

function FieldError({ id, message }: { id?: string; message?: string }) {
  return message ? <span className="field-error" id={id}>{message}</span> : null
}

function ErrorText({ error }: { error: unknown }) {
  const { t } = useI18n()
  return <p className="error" role="alert">{errorMessage(error, t)}</p>
}

function errorMessage(error: unknown, t: Translate) {
  if (error instanceof ApiError) {
    const messages: Record<string, ReturnType<Translate>> = {
      unauthorized: t('error.unauthorized'),
      rate_limited: t('error.rateLimited'),
      auth_unavailable: t('error.authUnavailable'),
      conflict: t('error.conflict'),
      forbidden: t('error.forbidden'),
      payload_too_large: t('error.payloadTooLarge'),
      validation_error: t('error.validation'),
      internal_error: t('error.internal'),
    }
    return messages[error.code] ?? t('error.internal')
  }
  return error instanceof Error ? error.message : t('common.unexpectedError')
}

function nextStatus(status: PantryStatus): PantryStatus {
  return status === 'have' ? 'unknown' : status === 'unknown' ? 'out' : 'have'
}

function pantryStatusLabel(status: PantryStatus, t: Translate): string {
  return status === 'have'
    ? t('pantry.available')
    : status === 'out'
      ? t('pantry.out')
      : t('pantry.unknown')
}

function recommendationReasonLabel(
  reason: RecommendationReason,
  t: Translate,
  formatDate: (value: string | Date) => string,
  formatNumber: (value: number) => string,
): string {
  switch (reason.code) {
    case 'confirmedIngredients':
      return t('recommend.reason.confirmed', {
        confirmed: formatNumber(reason.confirmed),
        total: formatNumber(reason.total),
      })
    case 'preferredIngredients':
      return t('recommend.reason.preferred', { count: formatNumber(reason.count) })
    case 'ingredientExpired':
      return t('recommend.reason.expired', {
        ingredient: reason.ingredient,
        date: formatDate(reason.expiresOn),
      })
    case 'ingredientExpiresToday':
      return t('recommend.reason.expiresToday', { ingredient: reason.ingredient })
    case 'pantryOnlyBlocked':
      return t('recommend.reason.pantryOnly')
  }
}

function recommendationSourceLabel(source: RecommendationSource, t: Translate): string {
  switch (source) {
    case 'OWN':
      return t('recommend.source.own')
    case 'FAVORITE_RECIPE':
      return t('recommend.source.favoriteRecipe')
    case 'FAVORITE_ACCOUNT':
      return t('recommend.source.favoriteAccount')
    case 'OTHER_PUBLIC':
      return t('recommend.source.otherPublic')
  }
}

function recommendationIngredientReasonLabel(
  reason: Recommendation['ingredientReasons'][number],
  t: Translate,
  formatDate: (value: string | Date) => string,
): string {
  if ((reason.code === 'EXPIRED' || reason.code === 'EXPIRES_TODAY') && reason.expiresOn) {
    return t(`recommend.ingredient.${reason.code}`, {
      ingredient: reason.ingredient,
      date: formatDate(reason.expiresOn),
    })
  }
  return t(`recommend.ingredient.${reason.code}`, { ingredient: reason.ingredient })
}

function recommendationDesiredCoverage(
  recommendation: Recommendation,
  fallbackTotal: number,
): { matched: number; total: number } {
  const intent = recommendation.rankingIntent
  const matched = typeof intent === 'object'
    && (intent.version === 'intent-v1' || intent.version === 'intent-v2')
    && typeof intent.desiredMatched === 'number'
    ? intent.desiredMatched
    : recommendation.desiredMatches.length
  const total = typeof intent === 'object'
    && (intent.version === 'intent-v1' || intent.version === 'intent-v2')
    && typeof intent.desiredTotal === 'number'
    ? intent.desiredTotal
    : fallbackTotal
  return {
    matched: Math.max(0, Math.min(matched, total)),
    total: Math.max(0, total),
  }
}

function recommendationShoppingCoverage(
  recommendation: Recommendation,
  shoppingItems: ShoppingItem[],
): { covered: number; total: number } {
  const reasons = recommendation.ingredientGroupReasons?.filter((reason) => reason.effective) ?? []
  if (reasons.length === 0) return { covered: 0, total: 0 }
  const groups = recipeIngredientGroups(recommendation.recipe)
  const activeIdentities = new Set(shoppingItems.flatMap((item) => [
    ingredientIdentity(item),
    ...(item.choiceAlternatives ?? []).map(ingredientIdentity),
  ]))
  const listedNames = new Set(recommendation.inShoppingList.map(normalizeIngredient))
  const covered = reasons.filter((reason) => {
    if (reason.code === 'AVAILABLE') return true
    const group = groups.find((candidate) => candidate.id === reason.groupId)
    return group?.alternatives.some((alternative) =>
      activeIdentities.has(ingredientIdentity(alternative))
      || listedNames.has(normalizeIngredient(alternative.name)),
    ) ?? false
  }).length
  return { covered, total: reasons.length }
}

function importStatusLabel(status: ImportDraft['status'], t: Translate): string {
  return t(`imports.status.${status}`)
}

async function copyText(value: string): Promise<void> {
  await navigator.clipboard.writeText(value)
}
