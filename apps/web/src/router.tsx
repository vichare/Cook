import * as Dialog from '@radix-ui/react-dialog'
import { zodResolver } from '@hookform/resolvers/zod'
import {
  Link,
  Outlet,
  createRootRouteWithContext,
  createRoute,
  createRouter,
  useNavigate,
} from '@tanstack/react-router'
import { QueryClient, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useFieldArray, useForm } from 'react-hook-form'
import { z } from 'zod'
import { ApiError, api } from './api'
import type {
  ImportDraft,
  IngredientReference,
  PantryItem,
  PantryStatus,
  Recipe,
  RecommendationSettings,
  RecommendationReason,
  SecuritySettings as SecuritySettingsData,
  User,
} from './types'
import { IngredientCombobox } from './IngredientCombobox'
import {
  expirationLabel,
  expirationRelativeDays,
  expirationShortcut,
  localDateValue,
  normalizeIngredient,
  splitIngredientList,
} from './ingredients'
import { createPasskey, getPasskey } from './webauthn'
import { useI18n, type Locale, type Translate } from './i18n'

interface RouterContext {
  queryClient: QueryClient
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
const editRecipeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/recipes/$recipeId/edit',
  component: EditRecipePage,
})
const pantryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/pantry',
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
  editRecipeRoute,
  pantryRoute,
  shoppingRoute,
  settingsRoute,
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
        <nav aria-label={t('app.primaryNavigation')}>
          <Link to="/recommendations">{t('app.recommend')}</Link>
          <Link to="/recipes">{t('app.recipes')}</Link>
          <Link to="/pantry">{t('app.pantry')}</Link>
          <Link to="/shopping-list">{t('app.shopping')}</Link>
          <Link to="/imports">{t('app.imports')}</Link>
          <Link to="/settings">{t('app.settings')}</Link>
        </nav>
        <button
          className="button ghost"
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

function HomePage() {
  const { formatNumber, t } = useI18n()
  const recipes = useQuery({ queryKey: ['recipes'], queryFn: api.recipes })
  const pantry = useQuery({ queryKey: ['pantry'], queryFn: api.pantry })
  return (
    <Page title={t('home.title')} intro={t('home.intro')}>
      <div className="metric-grid">
        <Metric label={t('home.visibleRecipes')} value={recipes.data ? formatNumber(recipes.data.length) : '—'} />
        <Metric label={t('home.pantryItems')} value={pantry.data ? formatNumber(pantry.data.length) : '—'} />
        <Metric
          label={t('home.confirmedAvailable')}
          value={pantry.data ? formatNumber(pantry.data.filter((item) => item.status === 'have').length) : '—'}
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

function RecommendationsPage() {
  const { formatDate, formatNumber, t } = useI18n()
  const queryClient = useQueryClient()
  const [filters, setFilters] = useState({
    preferred: [] as IngredientReference[],
    required: [] as IngredientReference[],
    excluded: [] as IngredientReference[],
    pantryOnly: false,
  })
  const [submitted, setSubmitted] = useState({
    preferred: [] as IngredientReference[],
    required: [] as IngredientReference[],
    excluded: [] as IngredientReference[],
    pantryOnly: false,
    today: localDateValue(new Date()),
  })
  const recommendations = useQuery({
    queryKey: ['recommendations', submitted],
    queryFn: () => api.recommendations(submitted),
  })
  const verifyNames = useMemo(
    () => [...new Set(recommendations.data?.flatMap((item) => item.verify) ?? [])].slice(0, 5),
    [recommendations.data],
  )
  const [confirmOpen, setConfirmOpen] = useState(false)
  const addShopping = useMutation({
    mutationFn: ({
      recipeId,
      ingredient,
    }: {
      recipeId: string
      ingredient: IngredientReference
    }) => api.addRecipeIngredientToShopping(recipeId, ingredient),
    onSuccess: (items) => queryClient.setQueryData(['shopping-list'], items),
  })

  return (
    <Page title={t('recommend.title')} intro={t('recommend.intro')}>
      <form
        className="filter-panel"
        onSubmit={(event) => {
          event.preventDefault()
          setSubmitted({ ...filters, today: localDateValue(new Date()) })
        }}
      >
        <IngredientFilterInput
          label={t('recommend.hopeToUse')}
          values={filters.preferred}
          onChange={(preferred) => setFilters({ ...filters, preferred })}
        />
        <IngredientFilterInput
          label={t('recommend.mustUse')}
          values={filters.required}
          onChange={(required) => setFilters({ ...filters, required })}
        />
        <IngredientFilterInput
          label={t('recommend.doNotUse')}
          values={filters.excluded}
          onChange={(excluded) => setFilters({ ...filters, excluded })}
        />
        <label className="check-row">
          <input
            type="checkbox"
            checked={filters.pantryOnly}
            onChange={(event) => setFilters({ ...filters, pantryOnly: event.target.checked })}
          />
          {t('recommend.pantryOnly')}
        </label>
        <button className="button primary">{t('recommend.refresh')}</button>
      </form>
      {verifyNames.length > 0 && (
        <div className="notice">
          <span>{t('recommend.checkCount', { count: formatNumber(verifyNames.length) })}</span>
          <button className="button secondary" onClick={() => setConfirmOpen(true)}>
            {t('recommend.checkPantry')}
          </button>
        </div>
      )}
      <QueryState query={recommendations} empty={t('recommend.noMatches')}>
        <div className="card-list">
          {recommendations.data?.map((result) => (
            <article className="recommendation-card" key={result.recipe.id}>
              <div className="card-heading">
                <div>
                  <span className={`status ${result.status.toLowerCase()}`}>{t(`status.${result.status}`)}</span>
                  <h2>{result.recipe.title}</h2>
                </div>
                <Link to="/recipes/$recipeId" params={{ recipeId: result.recipe.id }}>
                  {t('recommend.details')}
                </Link>
              </div>
              <p>
                {result.reasonDetails
                  .map((reason) => recommendationReasonLabel(reason, t, formatDate, formatNumber))
                  .join(' · ')}
              </p>
              <IngredientSummary label={t('recommend.available')} items={result.available} />
              <IngredientSummary label={t('recommend.verify')} items={result.verify} />
              <IngredientSummary label={t('recommend.missing')} items={result.missing} />
              {result.missing.length > 0 && (
                <div className="form-actions" aria-label={t('shopping.addMissingGroup')}>
                  {result.missing.map((name) => {
                    const ingredient = result.recipe.ingredients.find(
                      (candidate) => normalizeIngredient(candidate.name) === normalizeIngredient(name),
                    )
                    return (
                      <button
                        type="button"
                        className="button ghost"
                        key={name}
                        disabled={addShopping.isPending}
                        onClick={() => {
                          if (!window.confirm(t('shopping.addMissingConfirm', { name }))) return
                          addShopping.mutate({
                            recipeId: result.recipe.id,
                            ingredient: {
                              name,
                              canonicalIngredientId: ingredient?.canonicalIngredientId ?? null,
                            },
                          })
                        }}
                      >
                        {t('shopping.addMissing', { name })}
                      </button>
                    )
                  })}
                </div>
              )}
            </article>
          ))}
        </div>
      </QueryState>
      <PantryConfirmation open={confirmOpen} onOpenChange={setConfirmOpen} names={verifyNames} />
    </Page>
  )
}

function IngredientFilterInput({
  label,
  values,
  onChange,
}: {
  label: string
  values: IngredientReference[]
  onChange: (values: IngredientReference[]) => void
}) {
  const { t } = useI18n()
  const [draft, setDraft] = useState<IngredientReference>({ name: '', canonicalIngredientId: null })
  function add(next: IngredientReference) {
    const additions = splitIngredientList(next.name).map((name, index) => ({
      name,
      canonicalIngredientId: index === 0 ? next.canonicalIngredientId : null,
    }))
    const existing = new Set(values.map((value) => value.name.normalize('NFKC').toLocaleLowerCase()))
    onChange([
      ...values,
      ...additions.filter((value) => !existing.has(value.name.normalize('NFKC').toLocaleLowerCase())),
    ])
    setDraft({ name: '', canonicalIngredientId: null })
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
          onChange([...values, ...names.map((name) => ({ name, canonicalIngredientId: null }))])
          setDraft({ name: '', canonicalIngredientId: null })
        }}
        placeholder={t('recommend.filterPlaceholder')}
      />
      {values.length > 0 && (
        <ul className="ingredient-chips" aria-label={t('recommend.filterList', { label })}>
          {values.map((value, index) => (
            <li key={`${value.name}-${index}`}>
              <span>{value.name}</span>
              <button
                type="button"
                className="button ghost"
                aria-label={t('recommend.removeFromFilter', { name: value.name, label })}
                onClick={() => onChange(values.filter((_, itemIndex) => itemIndex !== index))}
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

function RecipesPage() {
  const { t } = useI18n()
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
      <QueryState query={recipes} empty={t('recipes.empty')}>
        <div className="card-list">
          {recipes.data?.map((recipe) => (
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
                  <h2>{recipe.title}</h2>
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
              <p className="byline">
                {t('common.byAuthor', { author: recipe.authorNickname ?? t('common.anonymousUser') })}
              </p>
              <Link to="/recipes/$recipeId" params={{ recipeId: recipe.id }}>
                {t('recipes.open')}
              </Link>
            </article>
          ))}
        </div>
      </QueryState>
    </Page>
  )
}

function RecipeDetailPage() {
  const { t } = useI18n()
  const { recipeId } = recipeRoute.useParams()
  const recipe = useQuery({ queryKey: ['recipe', recipeId], queryFn: () => api.recipe(recipeId) })
  return (
    <Page title={recipe.data?.title ?? t('recipe.fallbackTitle')} intro={recipe.data?.description ?? ''}>
      <QueryState query={recipe} empty={t('recipe.notFound')}>
        {recipe.data && (
          <article className="detail-card">
            <div className="card-heading">
              <span className="status verify">
                {recipe.data.visibility === 'public' ? t('common.public') : t('common.private')}
              </span>
              {recipe.data.isOwner && (
                <Link className="button secondary" to="/recipes/$recipeId/edit" params={{ recipeId }}>
                  {t('common.edit')}
                </Link>
              )}
            </div>
            <p className="byline">
              {t('common.byAuthor', { author: recipe.data.authorNickname ?? t('common.anonymousUser') })}
            </p>
            {recipe.data.coverImage && (
              <figure className="recipe-cover-detail">
                <img
                  src={recipe.data.coverImage.url}
                  alt={recipe.data.coverImage.altText}
                  width={recipe.data.coverImage.width}
                  height={recipe.data.coverImage.height}
                />
                {(recipe.data.coverImage.sourceAttribution || recipe.data.coverImage.sourceLicense) && (
                  <figcaption>
                    {recipe.data.coverImage.sourceAttribution}
                    {recipe.data.coverImage.sourceAttribution && recipe.data.coverImage.sourceLicense && ' · '}
                    {recipe.data.coverImage.sourceLicense}
                  </figcaption>
                )}
                {recipe.data.coverImage.sourceUrl && (
                  <a href={recipe.data.coverImage.sourceUrl} rel="noreferrer" target="_blank">
                    {t('recipe.coverSourceLink')}
                  </a>
                )}
              </figure>
            )}
            {recipe.data.source && (
              <aside className="source-note">
                <strong>{t('recipe.importedSource')}</strong>
                <a href={recipe.data.source.url} rel="noreferrer">
                  {recipe.data.source.title ?? recipe.data.source.url}
                </a>
                {recipe.data.source.author && (
                  <span>{t('recipe.sourceBy', { author: recipe.data.source.author })}</span>
                )}
                {recipe.data.source.warnings.map((warning) => (
                  <p key={warning}>{warning}</p>
                ))}
              </aside>
            )}
            <h2>{t('recipe.ingredients')}</h2>
            <ul>
              {recipe.data.ingredients.map((ingredient, index) => (
                <li key={`${ingredient.name}-${index}`}>
                  {ingredient.name} {ingredient.quantityNote && <span>— {ingredient.quantityNote}</span>}
                </li>
              ))}
            </ul>
            <h2>{t('recipe.method')}</h2>
            <p className="pre-line">{recipe.data.instructions || t('recipe.noInstructions')}</p>
            {recipe.data.isOwner && <RevisionReview recipeId={recipeId} />}
          </article>
        )}
      </QueryState>
    </Page>
  )
}

function RevisionReview({ recipeId }: { recipeId: string }) {
  const { t } = useI18n()
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
            <ul>
              {revision.ingredients.map((ingredient, index) => (
                <li key={`${ingredient.name}-${index}`}>{ingredient.name} {ingredient.quantityNote}</li>
              ))}
            </ul>
            <h4>{t('recipe.method')}</h4>
            <p className="pre-line">{revision.instructions}</p>
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

function recipeSchema(t: Translate) {
  return z.object({
  title: z.string().trim().min(1, t('recipe.titleRequired')),
  description: z.string(),
  instructions: z.string(),
  visibility: z.enum(['private', 'public']),
  ingredients: z.array(z.object({
    name: z.string().trim().min(1, t('recipe.ingredientRequired')),
    quantityNote: z.string(),
    canonicalIngredientId: z.string().nullable(),
  })).min(1),
  })
}
type RecipeForm = z.infer<ReturnType<typeof recipeSchema>>

function RecipeEditor({ recipe }: { recipe?: Recipe }) {
  const { t } = useI18n()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const errorSummaryRef = useRef<HTMLDivElement>(null)
  const [coverFile, setCoverFile] = useState<File | null>(null)
  const [removeCover, setRemoveCover] = useState(false)
  const [coverPreview, setCoverPreview] = useState<string | null>(recipe?.coverImage?.url ?? null)
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
    defaultValues: recipe ?? {
      title: '',
      description: '',
      instructions: '',
      visibility: 'private',
      ingredients: [{ name: '', quantityNote: '', canonicalIngredientId: null }],
    },
  })
  const fields = useFieldArray({ control: form.control, name: 'ingredients' })
  const save = useMutation({
    mutationFn: async (value: RecipeForm) => {
      const saved = await api.saveRecipe(value, recipe?.id)
      if (coverFile) {
        await api.uploadRecipeCover(saved.id, coverFile, coverMetadata)
      } else if (removeCover && recipe?.coverImage) {
        await api.deleteRecipeCover(saved.id)
      } else if (
        recipe?.coverImage &&
        (
          coverMetadata.altText !== recipe.coverImage.altText ||
          coverMetadata.sourceUrl !== (recipe.coverImage.sourceUrl ?? '') ||
          coverMetadata.sourceLicense !== (recipe.coverImage.sourceLicense ?? '') ||
          coverMetadata.sourceAttribution !== (recipe.coverImage.sourceAttribution ?? '')
        )
      ) {
        await api.updateRecipeCoverMetadata(saved.id, coverMetadata)
      }
      return saved
    },
    onSuccess: async (saved) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ['recipes'] }),
        queryClient.invalidateQueries({ queryKey: ['recipe', saved.id] }),
      ])
      await navigate({ to: '/recipes/$recipeId', params: { recipeId: saved.id } })
    },
  })
  const ingredientErrors = Array.isArray(form.formState.errors.ingredients)
    ? form.formState.errors.ingredients
    : []
  const validationMessages = [
    form.formState.errors.title?.message,
    ...ingredientErrors.map((error, index) =>
      error?.name?.message
        ? t('recipe.ingredientError', { number: index + 1, message: error.name.message })
        : undefined,
    ),
  ].filter((message): message is string => Boolean(message))
  return (
    <form
      className="editor-form"
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
      <label>
        {t('recipe.instructions')}
        <textarea {...form.register('instructions')} rows={8} maxLength={20000} />
      </label>
      <label>
        {t('recipe.visibility')}
        <select {...form.register('visibility')}>
          <option value="private">{t('recipe.privateDefault')}</option>
          <option value="public">{t('recipe.public')}</option>
        </select>
      </label>
      <RecipeCoverFields
        hasImage={Boolean(coverFile || recipe?.coverImage)}
        removed={removeCover}
        preview={coverPreview}
        metadata={coverMetadata}
        onMetadataChange={setCoverMetadata}
        onFile={(file) => {
          setCoverFile(file)
          if (file) setRemoveCover(false)
        }}
        onRemove={() => {
          setCoverFile(null)
          setCoverPreview(null)
          setRemoveCover(Boolean(recipe?.coverImage))
        }}
      />
      <fieldset>
        <legend>{t('recipe.ingredients')}</legend>
        {fields.fields.map((field, index) => (
          <div className="ingredient-row" key={field.id}>
            <div>
              <IngredientCombobox
                label={t('recipe.ingredientNumber', { number: index + 1 })}
                value={{
                  name: form.watch(`ingredients.${index}.name`),
                  canonicalIngredientId: form.watch(`ingredients.${index}.canonicalIngredientId`),
                }}
                onChange={(value) => {
                  form.setValue(`ingredients.${index}.name`, value.name, { shouldDirty: true })
                  form.setValue(`ingredients.${index}.canonicalIngredientId`, value.canonicalIngredientId, { shouldDirty: true })
                }}
                onPasteList={(names) => {
                  form.setValue(`ingredients.${index}.name`, names[0], { shouldDirty: true })
                  form.setValue(`ingredients.${index}.canonicalIngredientId`, null, { shouldDirty: true })
                  for (const name of names.slice(1)) {
                    fields.append({ name, quantityNote: '', canonicalIngredientId: null })
                  }
                }}
                placeholder={t('recommend.filterPlaceholder')}
                invalid={Boolean(form.formState.errors.ingredients?.[index]?.name)}
                describedBy={
                  form.formState.errors.ingredients?.[index]?.name
                    ? `recipe-ingredient-${index}-error`
                    : undefined
                }
              />
              <FieldError
                id={`recipe-ingredient-${index}-error`}
                message={form.formState.errors.ingredients?.[index]?.name?.message}
              />
            </div>
            <label>
              <span className="sr-only">{t('recipe.quantityFor', { number: index + 1 })}</span>
              <input
                {...form.register(`ingredients.${index}.quantityNote`)}
                placeholder={t('recipe.quantityNote')}
              />
            </label>
            {fields.fields.length > 1 && (
              <button type="button" className="button ghost" onClick={() => fields.remove(index)}>
                {t('common.remove')}
              </button>
            )}
          </div>
        ))}
        <button type="button" className="button secondary" onClick={() => fields.append({ name: '', quantityNote: '', canonicalIngredientId: null })}>
          {t('recipe.addIngredient')}
        </button>
      </fieldset>
      <button className="button primary" disabled={save.isPending}>
        {save.isPending ? t('recipe.saving') : t('recipe.saveRecipe')}
      </button>
      {save.error && <ErrorText error={save.error} />}
    </form>
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

function PantryPage() {
  const { formatDate, t } = useI18n()
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
      <form
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
          {pantry.data?.map((item) => (
            <div className="pantry-row" role="row" key={item.id}>
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
  const [draft, setDraft] = useState<IngredientReference>({
    name: '',
    canonicalIngredientId: null,
  })
  const [editingId, setEditingId] = useState<string | null>(null)
  const [undoItem, setUndoItem] = useState<PantryItem | null>(null)
  const listMutation = useMutation({
    mutationFn: (operation: () => ReturnType<typeof api.addShoppingItem>) => operation(),
    onSuccess: (items) => {
      queryClient.setQueryData(['shopping-list'], items)
      setDraft({ name: '', canonicalIngredientId: null })
      setEditingId(null)
    },
  })
  const pantryMutation = useMutation({
    mutationFn: api.savePantry,
    onSuccess: (items) => {
      queryClient.setQueryData(['pantry'], items)
      void queryClient.invalidateQueries({ queryKey: ['shopping-list'] })
    },
  })
  const items = shopping.data ?? []
  const automatic = items.filter((item) => item.automatic && !item.completedAt)
  const manual = items.filter((item) => !item.automatic && !item.completedAt)
  const completed = items.filter((item) => item.completedAt && !item.automatic)
  function sourceLabel(kind: string, label: string) {
    if (kind === 'automatic') return t('shopping.sourceAutomatic')
    if (kind === 'manual') return t('shopping.sourceManual')
    return t('shopping.sourceRecipe', { recipe: label })
  }
  function pantryPayload(item: PantryItem, overrides: Partial<PantryItem>) {
    return {
      name: item.name,
      status: overrides.status ?? item.status,
      quantityNote: item.quantityNote,
      expiresOn: item.expiresOn,
      canonicalIngredientId: item.canonicalIngredientId,
      autoAddToShoppingListWhenOut:
        overrides.autoAddToShoppingListWhenOut ?? item.autoAddToShoppingListWhenOut,
    }
  }
  function renderItems(sectionItems: typeof items, automaticSection: boolean) {
    return (
      <div className="card-list">
        {sectionItems.map((item) => {
          const pantryItem = item.pantryItemId
            ? pantry.data?.find((candidate) => candidate.id === item.pantryItemId)
            : undefined
          const canEdit = item.sources.some((source) => source.kind === 'manual')
          return (
            <article className="recipe-card" key={item.id}>
              <strong>{item.name}</strong>
              <span>{item.sources.map((source) => sourceLabel(source.kind, source.label)).join(' · ')}</span>
              <div className="form-actions">
                {automaticSection && pantryItem && (
                  <>
                    <button
                      type="button"
                      className="button primary"
                      aria-label={t('shopping.purchasedLabel', { name: item.name })}
                      onClick={() => {
                        if (!window.confirm(t('shopping.purchasedConfirm', { name: item.name }))) return
                        setUndoItem(pantryItem)
                        listMutation.mutate(async () => {
                          const items = await api.markAutomaticShoppingPurchased(pantryItem.id)
                          await queryClient.invalidateQueries({ queryKey: ['pantry'] })
                          return items
                        })
                      }}
                    >
                      {t('shopping.purchased')}
                    </button>
                    <button
                      type="button"
                      className="button ghost"
                      onClick={() =>
                        pantryMutation.mutate([
                          pantryPayload(pantryItem, {
                            autoAddToShoppingListWhenOut: false,
                          }),
                        ])
                      }
                    >
                      {t('shopping.disableAutomatic')}
                    </button>
                  </>
                )}
                {!automaticSection && !item.completedAt && (
                  <button
                    type="button"
                    className="button primary"
                    onClick={() => listMutation.mutate(() => api.completeShoppingItem(item.id))}
                  >
                    {t('shopping.complete')}
                  </button>
                )}
                {canEdit && !item.completedAt && (
                  <button
                    type="button"
                    className="button ghost"
                    onClick={() => {
                      setEditingId(item.id)
                      setDraft({
                        name: item.name,
                        canonicalIngredientId: item.canonicalIngredientId,
                      })
                    }}
                  >
                    {t('shopping.edit')}
                  </button>
                )}
                {canEdit && (
                  <button
                    type="button"
                    className="button ghost"
                    onClick={() => {
                      if (window.confirm(t('shopping.deleteConfirm', { name: item.name }))) {
                        listMutation.mutate(() => api.deleteShoppingItem(item.id))
                      }
                    }}
                  >
                    {t('shopping.delete')}
                  </button>
                )}
              </div>
            </article>
          )
        })}
      </div>
    )
  }
  return (
    <Page title={t('shopping.title')} intro={t('shopping.intro')}>
      <form
        className="inline-form"
        onSubmit={(event) => {
          event.preventDefault()
          if (!draft.name.trim()) return
          listMutation.mutate(() =>
            editingId
              ? api.updateShoppingItem(editingId, draft)
              : api.addShoppingItem(draft),
          )
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
          <button
            type="button"
            className="button ghost"
            onClick={() => {
              setEditingId(null)
              setDraft({ name: '', canonicalIngredientId: null })
            }}
          >
            {t('common.cancel')}
          </button>
        )}
      </form>
      {undoItem && (
        <div className="status-banner" role="status">
          <span>{t('shopping.purchasedSaved', { name: undoItem.name })}</span>
          <button
            type="button"
            className="button secondary"
            onClick={() => {
              pantryMutation.mutate([pantryPayload(undoItem, { status: 'out' })])
              setUndoItem(null)
            }}
          >
            {t('common.undo')}
          </button>
        </div>
      )}
      {(listMutation.error || pantryMutation.error) && (
        <ErrorText error={listMutation.error ?? pantryMutation.error} />
      )}
      <QueryState query={shopping} empty={t('shopping.empty')}>
        <section>
          <h2>{t('shopping.automaticTitle')}</h2>
          <p>{t('shopping.automaticDetail')}</p>
          {automatic.length ? renderItems(automatic, true) : <p>{t('shopping.automaticEmpty')}</p>}
        </section>
        <section>
          <h2>{t('shopping.manualTitle')}</h2>
          {manual.length ? renderItems(manual, false) : <p>{t('shopping.manualEmpty')}</p>}
        </section>
        {completed.length > 0 && (
          <section>
            <h2>{t('shopping.completedTitle')}</h2>
            {renderItems(completed, false)}
          </section>
        )}
      </QueryState>
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
      <QueryState query={settings} empty={t('settings.unavailable')}>
        {settings.data && <SettingsForm initial={settings.data} />}
      </QueryState>
      <ImportTokenSettings />
    </Page>
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
  'https://github.com/vichare/Cook/blob/main/skills/cook-recipe-import/SKILL.md'
export const COOK_SKILL_INSTALL_INSTRUCTIONS = `Install the official Cook recipe import skill from:
${COOK_SKILL_URL}

Use your AI client's documented “install skill from GitHub URL” flow. Client commands differ, so follow that client's instructions. Configure COOK_IMPORT_TOKEN in the client's secret store or environment and COOK_BASE_URL=https://cook.vichare.net separately. Never paste the token into a prompt or chat.`
export const COOK_MANAGE_SKILL_INSTALL_INSTRUCTIONS = `${COOK_SKILL_INSTALL_INSTRUCTIONS}

This token uses Cook's management preset. The skill may read owned recipes, create or update private recipes, propose reviewed revisions for public recipes, and manage cover images. It cannot publish or delete recipes. Confirm image rights before every upload.`
export const COOK_PUBLISH_SKILL_INSTALL_INSTRUCTIONS = `${COOK_SKILL_INSTALL_INSTRUCTIONS}

This high-risk token includes recipes:publish. The skill may create public recipes, publish owned private recipes, directly update owned public recipes with optimistic concurrency, and manage cover images. It still cannot delete recipes or access another account. Check the granted scopes before every operation, confirm content and image rights, process batch work one recipe at a time, and finish with created/updated/skipped/failed counts plus recipe IDs or URLs.`
export const COOK_IMPORT_PROMPT_TEMPLATE = `English:
Use your browser tools to extract a recipe from a webpage I am authorized to access, preferring schema.org Recipe/JSON-LD. For video, use only legitimately available captions/transcripts; if unavailable, ask me for a transcript. Do not bypass login, paywalls, robots restrictions, access controls, or DRM. Rewrite concise structured ingredients and steps, preserve source URL/title/author, and record uncertainty as warnings. Validate against the Cook import draft schema, then create a private review draft at COOK_BASE_URL using the recipes:import credential from the COOK_IMPORT_TOKEN secret/environment variable. Never print, quote, log, download, or place that token in a URL or prompt. Do not publish or finalize the recipe; tell me to review the private draft in Cook.

简体中文：
请使用浏览器工具，从我有权访问的网页提取菜谱，优先使用 schema.org Recipe/JSON-LD。视频只能使用合法可用的字幕或 transcript；若无法取得，请要求我提供 transcript。不得绕过登录、付费墙、robots 限制、访问控制或 DRM。请将食材和步骤简洁结构化并改写，保留来源 URL、标题和作者，把不确定内容记录为 warnings。按 Cook 导入草稿 schema 验证后，使用 secret store/环境变量中的 COOK_IMPORT_TOKEN（recipes:import 权限）调用 COOK_BASE_URL，创建私密待审核草稿。绝不要打印、引用、记录、下载 token，也不要把 token 放进 URL 或 prompt。不要发布或完成菜谱；请提醒我在 Cook 中人工审核私密草稿。`
export const COOK_MANAGE_PROMPT_TEMPLATE = `English:
Use your browser or transcript tools only for content I am authorized to access. Prefer schema.org Recipe/JSON-LD and legitimately available captions/transcripts. Do not bypass login, paywalls, access controls, robots restrictions, or DRM. Use COOK_BASE_URL with the COOK_IMPORT_TOKEN secret/environment variable to read my recipes and versions, create private recipes, or update my private recipes with optimistic concurrency. Updates to public recipes must create a pending revision for my review and must never publish directly. Before uploading a local JPEG, PNG, or WebP cover image, ask me to confirm that I own it or have permission, then provide accurate alt text and any source/license/attribution. Never fetch or copy a source site's image automatically. Never print, quote, log, download, or place the token in a URL or prompt.

简体中文：
仅使用浏览器或 transcript 工具处理我有权访问的内容。优先使用 schema.org Recipe/JSON-LD 和合法可用的字幕或 transcript。不得绕过登录、付费墙、访问控制、robots 限制或 DRM。使用 COOK_BASE_URL，并从 secret store/环境变量读取 COOK_IMPORT_TOKEN，以读取我的菜谱及版本、创建私密菜谱，或通过乐观并发更新我的私密菜谱。公开菜谱的更新必须创建待审核 revision，绝不能直接发布。上传本地 JPEG、PNG 或 WebP 封面前，先请我确认拥有或获准使用，并提供准确的替代文字及来源、许可、归属信息。绝不要自动抓取或复制来源网站图片。绝不要打印、引用、记录、下载 token，也不要把 token 放进 URL 或 prompt。`
export const COOK_PUBLISH_PROMPT_TEMPLATE = `English:
Use your browser or transcript tools only for content I am authorized to access. Use COOK_BASE_URL with the COOK_IMPORT_TOKEN secret/environment variable, and check that the token has recipes:read, recipes:write, and recipes:publish before requesting public visibility; cover changes also require recipes:images:write. With recipes:publish, you may create public recipes, publish my private recipes, and directly update my public recipes, but every update must use the current recipe version and stop on 409 until you reread it. Never delete recipes, access another account, bypass validation, or publish when recipes:publish is absent. Process bulk work one recipe at a time with a distinct Idempotency-Key for each creation. Do not silently skip failures. At the end, report created, updated, skipped, and failed counts with each recipe ID or URL and error. AI output becomes public immediately and may contain errors, copyrighted material, or private information, so preserve source/license/attribution, confirm image rights before uploading local JPEG/PNG/WebP bytes, and never fetch a source image automatically. Never print, quote, log, download, or place the token in a URL or prompt.

简体中文：
仅使用浏览器或 transcript 工具处理我有权访问的内容。使用 COOK_BASE_URL，并从 secret store/环境变量读取 COOK_IMPORT_TOKEN；请求公开可见性前，必须确认 token 同时具有 recipes:read、recipes:write 和 recipes:publish，修改封面还需要 recipes:images:write。有 recipes:publish 时，可以创建公开菜谱、把我的私密菜谱发布为公开，并直接更新我的公开菜谱；每次更新必须使用当前菜谱 version，遇到 409 必须重新读取后再处理。绝不能删除菜谱、访问其他账号、绕过校验，或在没有 recipes:publish 时发布。批量操作应逐条处理，每个创建使用独立 Idempotency-Key；不得悄悄跳过失败项。结束时输出 created、updated、skipped、failed 数量，以及每项菜谱 ID 或 URL 和错误。AI 输出会立即公开，可能包含错误、受版权保护材料或隐私信息，因此必须保留来源、许可和归属；上传本地 JPEG/PNG/WebP 图片前确认使用权，绝不自动抓取来源图片。绝不要打印、引用、记录、下载 token，也不要把 token 放进 URL 或 prompt。`

type TokenPreset = 'import' | 'manage' | 'publish'

function skillInstructionsForPreset(preset: TokenPreset) {
  if (preset === 'publish') return COOK_PUBLISH_SKILL_INSTALL_INSTRUCTIONS
  if (preset === 'manage') return COOK_MANAGE_SKILL_INSTALL_INSTRUCTIONS
  return COOK_SKILL_INSTALL_INSTRUCTIONS
}

function promptForPreset(preset: TokenPreset) {
  if (preset === 'publish') return COOK_PUBLISH_PROMPT_TEMPLATE
  if (preset === 'manage') return COOK_MANAGE_PROMPT_TEMPLATE
  return COOK_IMPORT_PROMPT_TEMPLATE
}

export function ImportTokenSettings() {
  const { formatDateTime, t } = useI18n()
  const queryClient = useQueryClient()
  const tokens = useQuery({ queryKey: ['import-tokens'], queryFn: api.importTokens })
  const [name, setName] = useState(() => t('token.defaultName'))
  const [preset, setPreset] = useState<TokenPreset>('import')
  const [createdPreset, setCreatedPreset] = useState<TokenPreset>('import')
  const [expiresInDays, setExpiresInDays] = useState<30 | 90 | null>(90)
  const [secret, setSecret] = useState<string | null>(null)
  const [copyStatus, setCopyStatus] = useState<string | null>(null)
  const [copyError, setCopyError] = useState<string | null>(null)
  const create = useMutation({
    mutationFn: () => api.createImportToken(name, expiresInDays, preset),
    onSuccess: async (created) => {
      setSecret(created.secret)
      setCreatedPreset(preset)
      setCopyStatus(null)
      setCopyError(null)
      await queryClient.invalidateQueries({ queryKey: ['import-tokens'] })
    },
  })
  const revoke = useMutation({
    mutationFn: api.revokeImportToken,
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['import-tokens'] }),
  })
  function copyOutput(value: string, success: string) {
    setCopyError(null)
    void copyText(value)
      .then(() => setCopyStatus(success))
      .catch(() => {
        setCopyStatus(null)
        setCopyError(t('token.copyFailed'))
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
          create.mutate()
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
              onChange={() => setPreset('import')}
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
              onChange={() => setPreset('manage')}
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
                setExpiresInDays(30)
              }}
            />
            <span>
              <strong>{t('token.presetPublish')}</strong>
              <small>{t('token.presetPublishDetail')}</small>
            </span>
          </label>
        </fieldset>
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
          <section>
            <h3>{t('token.installSkill')}</h3>
            <p>{t('token.installSkillDetail')}</p>
            <p>{t('token.skillMainNote')}</p>
            <label>
              {t('token.skillUrlLabel')}
              <input readOnly value={COOK_SKILL_URL} />
            </label>
            <a className="button secondary" href={COOK_SKILL_URL} rel="noreferrer" target="_blank">
              {t('token.installSkill')}
            </a>
            <button
              className="button ghost"
              onClick={() => copyOutput(
                skillInstructionsForPreset(createdPreset),
                t('token.installCopied'),
              )}
            >
              {t('token.copyInstall')}
            </button>
          </section>
          <section>
            <h3>{t('token.copyPrompt')}</h3>
            <p>{t('token.promptDetail')}</p>
            <button
              className="button ghost"
              onClick={() => copyOutput(
                promptForPreset(createdPreset),
                t('token.promptCopied'),
              )}
            >
              {t('token.copyPrompt')}
            </button>
          </section>
          {copyStatus && <p role="status">{copyStatus}</p>}
          {copyError && <p className="error" role="alert">{copyError}</p>}
          <button className="button primary" onClick={() => {
            setSecret(null)
            setCopyStatus(null)
            setCopyError(null)
          }}>
            {t('token.savedClose')}
          </button>
        </div>
      )}
      <QueryState query={tokens} empty={t('token.empty')}>
        <div className="card-list">
          {tokens.data?.map((token) => (
            <article className="recipe-card" key={token.id}>
              <strong>{token.name}</strong>
              <span>{token.prefix}… · {token.scopes.join(', ')}</span>
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
              )}
            </article>
          ))}
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
      ingredients: draft.ingredients,
    },
  })
  const fields = useFieldArray({ control: form.control, name: 'ingredients' })
  const save = useMutation({
    mutationFn: (value: RecipeForm) =>
      api.saveImportDraft(draft.id, {
        title: value.title,
        description: value.description,
        instructions: value.instructions,
        ingredients: value.ingredients,
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
      <label>{t('recipe.instructions')}<textarea rows={10} {...form.register('instructions')} /></label>
      <fieldset>
        <legend>{t('recipe.ingredients')}</legend>
        {fields.fields.map((field, index) => (
          <div className="ingredient-row" key={field.id}>
            <input
              aria-label={t('recipe.ingredientNumber', { number: index + 1 })}
              {...form.register(`ingredients.${index}.name`)}
            />
            <input
              aria-label={t('import.quantityNumber', { number: index + 1 })}
              {...form.register(`ingredients.${index}.quantityNote`)}
            />
            {fields.fields.length > 1 && (
              <button type="button" className="button ghost" onClick={() => fields.remove(index)}>
                {t('common.remove')}
              </button>
            )}
          </div>
        ))}
        <button type="button" className="button secondary" onClick={() => fields.append({ name: '', quantityNote: '', canonicalIngredientId: null })}>{t('recipe.addIngredient')}</button>
      </fieldset>
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
        <input type="checkbox" {...form.register('includeFavorites')} /> {t('sources.favorites')}
      </label>
      <label className="check-row">
        <input type="checkbox" {...form.register('includeOwn')} /> {t('sources.own')}
      </label>
      <label className="check-row">
        <input type="checkbox" {...form.register('includePublic')} /> {t('sources.public')}
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
}: {
  open: boolean
  onOpenChange: (value: boolean) => void
  names: string[]
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

function Metric({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="metric">
      <strong>{value}</strong>
      <span>{label}</span>
    </div>
  )
}

function IngredientSummary({ label, items }: { label: string; items: string[] }) {
  if (items.length === 0) return null
  return (
    <p>
      <strong>{label}:</strong> {items.join(', ')}
    </p>
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

function importStatusLabel(status: ImportDraft['status'], t: Translate): string {
  return t(`imports.status.${status}`)
}

async function copyText(value: string): Promise<void> {
  await navigator.clipboard.writeText(value)
}
