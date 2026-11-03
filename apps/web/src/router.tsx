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
  SecuritySettings as SecuritySettingsData,
  User,
} from './types'
import { IngredientCombobox } from './IngredientCombobox'
import {
  expirationLabel,
  expirationShortcut,
  localDateValue,
  oneMonthAfter,
  splitIngredientList,
} from './ingredients'
import { createPasskey, getPasskey } from './webauthn'

interface RouterContext {
  queryClient: QueryClient
}

const rootRoute = createRootRouteWithContext<RouterContext>()({
  component: Root,
  notFoundComponent: () => <StatePanel title="Page not found" detail="Choose a page from the navigation." />,
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
  const me = useQuery({ queryKey: ['me'], queryFn: api.me })
  if (me.isLoading) return <StatePanel title="Opening your kitchen…" detail="Loading your private workspace." />
  if (me.error instanceof ApiError && me.error.status === 401) {
    return <Login onLogin={() => void queryClient.invalidateQueries({ queryKey: ['me'] })} />
  }
  if (me.error) return <StatePanel title="Cook could not start" detail={errorMessage(me.error)} />

  return (
    <div className="app-shell">
      <header className="site-header">
        <Link to="/" className="brand">
          Cook
        </Link>
        <nav aria-label="Primary">
          <Link to="/recommendations">Recommend</Link>
          <Link to="/recipes">Recipes</Link>
          <Link to="/pantry">Pantry</Link>
          <Link to="/imports">Imports</Link>
          <Link to="/settings">Settings</Link>
        </nav>
        <button
          className="button ghost"
          onClick={() =>
            void api.logout().then(() => queryClient.setQueryData(['me'], undefined)).finally(() => location.reload())
          }
        >
          Sign out
        </button>
      </header>
      <main id="main">
        <Outlet />
      </main>
      {me.data && !me.data.onboardingCompleted && <ProfileSetup user={me.data} />}
    </div>
  )
}

function Login({ onLogin }: { onLogin: () => void }) {
  const [name, setName] = useState('Demo Cook')
  const config = useQuery({ queryKey: ['auth-config'], queryFn: api.authConfig })
  const login = useMutation({ mutationFn: api.devLogin, onSuccess: onLogin })
  if (config.isLoading) return <StatePanel title="Loading sign-in…" detail="Checking the available sign-in method." />
  if (config.error) return <StatePanel title="Sign-in unavailable" detail={errorMessage(config.error)} />
  return (
    <main id="main" className="login">
      <section className="hero-card">
        <p className="eyebrow">Private pantry, practical dinner</p>
        <h1>Cook with what you already have.</h1>
        <p>Save recipes, track pantry confidence, and get deterministic recommendations with reasons.</p>
        {config.data?.mode === 'development' && (
          <form
            onSubmit={(event) => {
              event.preventDefault()
              login.mutate(name)
            }}
          >
            <label>
              Development display name
              <input value={name} onChange={(event) => setName(event.target.value)} required />
            </label>
            <button className="button primary" disabled={login.isPending}>
              {login.isPending ? 'Signing in…' : 'Enter development kitchen'}
            </button>
            {login.error && <ErrorText error={login.error} />}
          </form>
        )}
        {config.data?.emailOtp && <EmailOtpLogin onLogin={onLogin} passkeyEnabled={config.data.passkey} />}
        {config.data?.google && (
          <a className="button secondary" href="/api/auth/google/start">
            Continue with Google
          </a>
        )}
        <small>
          Email codes and passkeys use Cook's Host-only session. Google authorization codes and provider tokens never reach JavaScript.
        </small>
      </section>
    </main>
  )
}

export function EmailOtpLogin({
  onLogin,
  passkeyEnabled,
}: {
  onLogin: () => void
  passkeyEnabled: boolean
}) {
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
      <h2>Sign in with email</h2>
      <label>
        Email address
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
            Email me a code
          </button>
          {passkeyEnabled && (
            <button className="button secondary" disabled={!email || passkey.isPending} onClick={() => passkey.mutate()}>
              Sign in with a passkey
            </button>
          )}
        </div>
      ) : (
        <>
          <p role="status">If delivery is available, a six-digit code has been sent. Enter it below.</p>
          <label>
            Verification code
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
              Verify and sign in
            </button>
            <button
              className="button ghost"
              disabled={resendSeconds > 0 || request.isPending}
              onClick={() => request.mutate()}
            >
              {resendSeconds > 0 ? `Resend in ${resendSeconds}s` : 'Resend code'}
            </button>
            <button className="button ghost" onClick={() => setChallenge(null)}>
              Use another email
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
  const recipes = useQuery({ queryKey: ['recipes'], queryFn: api.recipes })
  const pantry = useQuery({ queryKey: ['pantry'], queryFn: api.pantry })
  return (
    <Page title="Your kitchen" intro="Move from uncertain pantry notes to an explainable meal choice.">
      <div className="metric-grid">
        <Metric label="Visible recipes" value={recipes.data?.length ?? '—'} />
        <Metric label="Pantry items" value={pantry.data?.length ?? '—'} />
        <Metric label="Confirmed available" value={pantry.data?.filter((item) => item.status === 'have').length ?? '—'} />
      </div>
      <div className="action-grid">
        <Link className="action-card" to="/recommendations">
          <strong>Find something to cook</strong>
          <span>Filter by ingredients and see READY, VERIFY, SHOP, or BLOCKED.</span>
        </Link>
        <Link className="action-card" to="/pantry">
          <strong>Update pantry confidence</strong>
          <span>Mark items as available, used up, or uncertain.</span>
        </Link>
        <Link className="action-card" to="/recipes/new">
          <strong>Add a recipe</strong>
          <span>Recipes are private until you explicitly publish them.</span>
        </Link>
      </div>
    </Page>
  )
}

function RecommendationsPage() {
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

  return (
    <Page title="Recommendation workbench" intro="Hard filters run before deterministic scoring; every result explains itself.">
      <form
        className="filter-panel"
        onSubmit={(event) => {
          event.preventDefault()
          setSubmitted({ ...filters, today: localDateValue(new Date()) })
        }}
      >
        <IngredientFilterInput
          label="Hope to use"
          values={filters.preferred}
          onChange={(preferred) => setFilters({ ...filters, preferred })}
        />
        <IngredientFilterInput
          label="Must use"
          values={filters.required}
          onChange={(required) => setFilters({ ...filters, required })}
        />
        <IngredientFilterInput
          label="Do not use"
          values={filters.excluded}
          onChange={(excluded) => setFilters({ ...filters, excluded })}
        />
        <label className="check-row">
          <input
            type="checkbox"
            checked={filters.pantryOnly}
            onChange={(event) => setFilters({ ...filters, pantryOnly: event.target.checked })}
          />
          Only use pantry ingredients
        </label>
        <button className="button primary">Refresh recommendations</button>
      </form>
      {verifyNames.length > 0 && (
        <div className="notice">
          <span>{verifyNames.length} ingredient(s) need a quick check.</span>
          <button className="button secondary" onClick={() => setConfirmOpen(true)}>
            Check pantry
          </button>
        </div>
      )}
      <QueryState query={recommendations} empty="No recipes match these hard filters.">
        <div className="card-list">
          {recommendations.data?.map((result) => (
            <article className="recommendation-card" key={result.recipe.id}>
              <div className="card-heading">
                <div>
                  <span className={`status ${result.status.toLowerCase()}`}>{result.status}</span>
                  <h2>{result.recipe.title}</h2>
                </div>
                <Link to="/recipes/$recipeId" params={{ recipeId: result.recipe.id }}>
                  Details
                </Link>
              </div>
              <p>{result.reasons.join(' · ')}</p>
              <IngredientSummary label="Available" items={result.available} />
              <IngredientSummary label="Verify" items={result.verify} />
              <IngredientSummary label="Missing" items={result.missing} />
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
        label={`Add ingredient to ${label.toLowerCase()}`}
        value={draft}
        onChange={setDraft}
        onCommit={add}
        onPasteList={(names) => {
          onChange([...values, ...names.map((name) => ({ name, canonicalIngredientId: null }))])
          setDraft({ name: '', canonicalIngredientId: null })
        }}
        placeholder="Type or paste ingredients"
      />
      {values.length > 0 && (
        <ul className="ingredient-chips" aria-label={`${label} ingredients`}>
          {values.map((value, index) => (
            <li key={`${value.name}-${index}`}>
              <span>{value.name}</span>
              <button
                type="button"
                className="button ghost"
                aria-label={`Remove ${value.name} from ${label.toLowerCase()}`}
                onClick={() => onChange(values.filter((_, itemIndex) => itemIndex !== index))}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}
    </fieldset>
  )
}

function RecipesPage() {
  const recipes = useQuery({ queryKey: ['recipes'], queryFn: api.recipes })
  const queryClient = useQueryClient()
  const favorite = useMutation({
    mutationFn: ({ id, next }: { id: string; next: boolean }) => api.favorite(id, next),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['recipes'] }),
  })
  return (
    <Page
      title="Recipes"
      intro="Your private recipes and public recipes shared by other cooks."
      action={
        <Link className="button primary" to="/recipes/new">
          New recipe
        </Link>
      }
    >
      <QueryState query={recipes} empty="No recipes yet. Add your first recipe.">
        <div className="card-list">
          {recipes.data?.map((recipe) => (
            <article className="recipe-card" key={recipe.id}>
              <div className="card-heading">
                <div>
                  <span className="eyebrow">{recipe.visibility}</span>
                  <h2>{recipe.title}</h2>
                </div>
                <button
                  className="button ghost"
                  aria-pressed={recipe.isFavorite}
                  onClick={() => favorite.mutate({ id: recipe.id, next: !recipe.isFavorite })}
                >
                  {recipe.isFavorite ? '★ Saved' : '☆ Save'}
                </button>
              </div>
              <p>{recipe.description || 'No description.'}</p>
              <p className="byline">By {recipe.authorNickname ?? 'Anonymous user'}</p>
              <Link to="/recipes/$recipeId" params={{ recipeId: recipe.id }}>
                Open recipe
              </Link>
            </article>
          ))}
        </div>
      </QueryState>
    </Page>
  )
}

function RecipeDetailPage() {
  const { recipeId } = recipeRoute.useParams()
  const recipe = useQuery({ queryKey: ['recipe', recipeId], queryFn: () => api.recipe(recipeId) })
  return (
    <Page title={recipe.data?.title ?? 'Recipe'} intro={recipe.data?.description ?? ''}>
      <QueryState query={recipe} empty="Recipe not found.">
        {recipe.data && (
          <article className="detail-card">
            <div className="card-heading">
              <span className="status verify">{recipe.data.visibility}</span>
              {recipe.data.isOwner && (
                <Link className="button secondary" to="/recipes/$recipeId/edit" params={{ recipeId }}>
                  Edit
                </Link>
              )}
            </div>
            <p className="byline">By {recipe.data.authorNickname ?? 'Anonymous user'}</p>
            {recipe.data.source && (
              <aside className="source-note">
                <strong>Imported source</strong>
                <a href={recipe.data.source.url} rel="noreferrer">
                  {recipe.data.source.title ?? recipe.data.source.url}
                </a>
                {recipe.data.source.author && <span> by {recipe.data.source.author}</span>}
                {recipe.data.source.warnings.map((warning) => (
                  <p key={warning}>{warning}</p>
                ))}
              </aside>
            )}
            <h2>Ingredients</h2>
            <ul>
              {recipe.data.ingredients.map((ingredient, index) => (
                <li key={`${ingredient.name}-${index}`}>
                  {ingredient.name} {ingredient.quantityNote && <span>— {ingredient.quantityNote}</span>}
                </li>
              ))}
            </ul>
            <h2>Method</h2>
            <p className="pre-line">{recipe.data.instructions || 'No instructions yet.'}</p>
          </article>
        )}
      </QueryState>
    </Page>
  )
}

function EditRecipePage() {
  const { recipeId } = editRecipeRoute.useParams()
  const recipe = useQuery({ queryKey: ['recipe', recipeId], queryFn: () => api.recipe(recipeId) })
  return (
    <Page title="Edit recipe" intro="Changes are only allowed for the recipe owner.">
      <QueryState query={recipe} empty="Recipe not found.">
        {recipe.data && recipe.data.isOwner ? (
          <RecipeEditor recipe={recipe.data} />
        ) : recipe.data ? (
          <StatePanel title="Owner access required" detail="Only the recipe owner can edit this recipe." />
        ) : null}
      </QueryState>
    </Page>
  )
}

const recipeSchema = z.object({
  title: z.string().trim().min(1, 'Title is required'),
  description: z.string(),
  instructions: z.string(),
  visibility: z.enum(['private', 'public']),
  ingredients: z.array(z.object({
    name: z.string().trim().min(1, 'Ingredient name is required'),
    quantityNote: z.string(),
    canonicalIngredientId: z.string().nullable(),
  })).min(1),
})
type RecipeForm = z.infer<typeof recipeSchema>

function RecipeEditor({ recipe }: { recipe?: Recipe }) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const errorSummaryRef = useRef<HTMLDivElement>(null)
  const form = useForm<RecipeForm>({
    resolver: zodResolver(recipeSchema),
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
    mutationFn: (value: RecipeForm) => api.saveRecipe(value, recipe?.id),
    onSuccess: async (saved) => {
      await queryClient.invalidateQueries({ queryKey: ['recipes'] })
      await navigate({ to: '/recipes/$recipeId', params: { recipeId: saved.id } })
    },
  })
  const ingredientErrors = Array.isArray(form.formState.errors.ingredients)
    ? form.formState.errors.ingredients
    : []
  const validationMessages = [
    form.formState.errors.title?.message,
    ...ingredientErrors.map((error, index) =>
      error?.name?.message ? `Ingredient ${index + 1}: ${error.name.message}` : undefined,
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
          <strong>Check the highlighted fields.</strong>
          <ul>
            {validationMessages.map((message) => <li key={message}>{message}</li>)}
          </ul>
        </div>
      )}
      <label>
        Title
        <input
          {...form.register('title')}
          aria-invalid={Boolean(form.formState.errors.title)}
          aria-describedby={form.formState.errors.title ? 'recipe-title-error' : undefined}
        />
        <FieldError id="recipe-title-error" message={form.formState.errors.title?.message} />
      </label>
      <label>
        Description
        <textarea {...form.register('description')} rows={3} />
      </label>
      <label>
        Instructions
        <textarea {...form.register('instructions')} rows={8} />
      </label>
      <label>
        Visibility
        <select {...form.register('visibility')}>
          <option value="private">Private (default)</option>
          <option value="public">Public</option>
        </select>
      </label>
      <fieldset>
        <legend>Ingredients</legend>
        {fields.fields.map((field, index) => (
          <div className="ingredient-row" key={field.id}>
            <div>
              <IngredientCombobox
                label={`Ingredient ${index + 1}`}
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
                placeholder="Type or paste ingredients"
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
              <span className="sr-only">Quantity note for ingredient {index + 1}</span>
              <input {...form.register(`ingredients.${index}.quantityNote`)} placeholder="Quantity note" />
            </label>
            {fields.fields.length > 1 && (
              <button type="button" className="button ghost" onClick={() => fields.remove(index)}>
                Remove
              </button>
            )}
          </div>
        ))}
        <button type="button" className="button secondary" onClick={() => fields.append({ name: '', quantityNote: '', canonicalIngredientId: null })}>
          Add ingredient
        </button>
      </fieldset>
      <button className="button primary" disabled={save.isPending}>
        {save.isPending ? 'Saving…' : 'Save recipe'}
      </button>
      {save.error && <ErrorText error={save.error} />}
    </form>
  )
}

function PantryPage() {
  const pantry = useQuery({ queryKey: ['pantry'], queryFn: api.pantry })
  const queryClient = useQueryClient()
  const [draft, setDraft] = useState({
    name: '',
    status: 'have' as PantryStatus,
    quantityNote: '',
    expiresOn: null as string | null,
    canonicalIngredientId: null as string | null,
  })
  const [editingName, setEditingName] = useState<string | null>(null)
  const save = useMutation({
    mutationFn: api.savePantry,
    onSuccess: (items) => {
      queryClient.setQueryData(['pantry'], items)
      setDraft({ name: '', status: 'have', quantityNote: '', expiresOn: null, canonicalIngredientId: null })
      setEditingName(null)
    },
  })
  return (
    <Page title="Private pantry" intro="Track availability and an optional calendar expiration date for each ingredient.">
      <form
        className="inline-form pantry-form"
        onSubmit={(event) => {
          event.preventDefault()
          save.mutate([draft])
        }}
      >
        <IngredientCombobox
          label="Ingredient"
          value={{ name: draft.name, canonicalIngredientId: draft.canonicalIngredientId }}
          onChange={(value) => setDraft({ ...draft, ...value })}
          onPasteList={(names) =>
            setDraft({ ...draft, name: names.join(', '), canonicalIngredientId: null })
          }
          placeholder="Type or paste one or more ingredients"
        />
        <label>
          Status
          <select
            value={draft.status}
            onChange={(event) => setDraft({ ...draft, status: event.target.value as PantryStatus })}
          >
            <option value="have">Have</option>
            <option value="out">Used up</option>
            <option value="unknown">Uncertain</option>
          </select>
        </label>
        <label>
          Quantity note
          <input value={draft.quantityNote} onChange={(event) => setDraft({ ...draft, quantityNote: event.target.value })} />
        </label>
        <ExpirationEditor
          value={draft.expiresOn}
          onChange={(expiresOn) => setDraft({ ...draft, expiresOn })}
        />
        <div className="form-actions">
          <button className="button primary" disabled={!draft.name.trim() || save.isPending}>
            {editingName ? `Save changes to ${editingName}` : 'Save pantry item'}
          </button>
          {editingName && (
            <button
              type="button"
              className="button ghost"
              onClick={() => {
                setDraft({ name: '', status: 'have', quantityNote: '', expiresOn: null, canonicalIngredientId: null })
                setEditingName(null)
              }}
            >
              Cancel editing
            </button>
          )}
        </div>
      </form>
      {editingName && <p role="status">Editing {editingName}. Change any field, then save.</p>}
      {save.error && <ErrorText error={save.error} />}
      <QueryState query={pantry} empty="Your pantry is empty. Add an ingredient above.">
        <div className="pantry-table" role="table" aria-label="Pantry items">
          {pantry.data?.map((item) => (
            <div className="pantry-row" role="row" key={item.id}>
              <strong role="cell">{item.name}</strong>
              <span role="cell" className={`status ${item.status === 'have' ? 'ready' : item.status === 'out' ? 'shop' : 'verify'}`}>
                {pantryStatusLabel(item.status)}
              </span>
              <span role="cell">{item.quantityNote || 'No quantity note'}</span>
              <span role="cell" className={item.expiresOn && item.expiresOn <= localDateValue(new Date()) ? 'expiry-warning' : ''}>
                {expirationLabel(item.expiresOn)}
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
                  })
                  setEditingName(item.name)
                }}
              >
                Edit item
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
                    },
                  ])
                }
              >
                Change status
              </button>
            </div>
          ))}
        </div>
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
  const today = localDateValue(now)
  const isPast = value !== null && value < today
  const options = [
    ['No expiration date', null],
    ['Today', today],
    ['Tomorrow', expirationShortcut(1, now)],
    ['One week', expirationShortcut(7, now)],
    ['One month', oneMonthAfter(now)],
  ] as const
  return (
    <fieldset className="expiration-editor">
      <legend>Expiration date (optional)</legend>
      <label>
        Choose a calendar date
        <input type="date" value={value ?? ''} onChange={(event) => onChange(event.target.value || null)} />
      </label>
      <div className="date-shortcuts" role="group" aria-label="Expiration date shortcuts">
        {options.map(([label, date]) => (
          <button
            type="button"
            className={value === date ? 'button secondary selected' : 'button ghost'}
            aria-pressed={value === date}
            key={label}
            onClick={() => onChange(date)}
          >
            {label}
          </button>
        ))}
      </div>
      <p className="field-help">Dates use your calendar day and do not include a time zone.</p>
      {isPast && (
        <p className="expiry-warning" role="alert">
          This date is in the past. You can still save it, but Cook will require verification before recommending this ingredient.
        </p>
      )}
    </fieldset>
  )
}

function SettingsPage() {
  const settings = useQuery({ queryKey: ['settings'], queryFn: api.settings })
  const me = useQuery({ queryKey: ['me'], queryFn: api.me })
  return (
    <Page title="Settings" intro="Manage your public profile, recommendation sources, and import access.">
      {me.data && <ProfileSettings user={me.data} />}
      <AccountSecuritySettings />
      <QueryState query={settings} empty="Settings unavailable.">
        {settings.data && <SettingsForm initial={settings.data} />}
      </QueryState>
      <ImportTokenSettings />
    </Page>
  )
}

export function ProfileSetup({ user }: { user: User }) {
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
          <Dialog.Title>Choose your public recipe name</Dialog.Title>
          <Dialog.Description>
            Your Google profile stays private. Only the nickname you explicitly choose is shown on public recipes.
          </Dialog.Description>
          <fieldset className="choice-list">
            <label className="radio-pill">
              <input
                type="radio"
                name="profile-choice"
                checked={choice === 'anonymous'}
                onChange={() => setChoice('anonymous')}
              />
              Anonymous user (default)
            </label>
            <label className="radio-pill">
              <input
                type="radio"
                name="profile-choice"
                checked={choice === 'suggested'}
                onChange={() => setChoice('suggested')}
              />
              Use “{user.displayName}” publicly
            </label>
            <label className="radio-pill">
              <input
                type="radio"
                name="profile-choice"
                checked={choice === 'custom'}
                onChange={() => setChoice('custom')}
              />
              Choose a custom nickname
            </label>
            {choice === 'custom' && (
              <input
                aria-label="Custom public nickname"
                value={custom}
                maxLength={40}
                onChange={(event) => setCustom(event.target.value)}
              />
            )}
          </fieldset>
          <div className="dialog-actions">
            <button className="button ghost" onClick={() => save.mutate(null)}>
              Skip and stay anonymous
            </button>
            <button
              className="button primary"
              disabled={save.isPending || (choice === 'custom' && !custom.trim())}
              onClick={() =>
                save.mutate(choice === 'suggested' ? user.displayName : choice === 'custom' ? custom : null)
              }
            >
              Save public profile
            </button>
          </div>
          {save.error && <ErrorText error={save.error} />}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

export function AccountSecuritySettings() {
  const queryClient = useQueryClient()
  const security = useQuery({ queryKey: ['security-settings'], queryFn: api.securitySettings })
  const [email, setEmail] = useState('')
  const [code, setCode] = useState('')
  const [challenge, setChallenge] = useState<Awaited<ReturnType<typeof api.requestEmailLink>> | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const [passkeyName, setPasskeyName] = useState('My passkey')
  useEffect(() => {
    if (!challenge) return
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [challenge])
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
      setPasskeyName('My passkey')
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
      <h2>Sign-in security</h2>
      <p>Email identities are linked only after verification. Cook never merges accounts merely because Google reports the same email.</p>
      <QueryState query={security} empty="No email identity or passkey is linked yet.">
        <>
          <div>
            <h3>Linked email</h3>
            {security.data?.linkedEmails.length ? (
              <ul>{security.data.linkedEmails.map((value) => <li key={value}>{value}</li>)}</ul>
            ) : (
              <p>Link an email before registering a passkey.</p>
            )}
          </div>
          <div>
            <h3>Passkeys</h3>
            {security.data?.passkeys.length ? (
              <div className="card-list">
                {security.data.passkeys.map((passkey) => (
                  <article className="recipe-card" key={passkey.id}>
                    <strong>{passkey.name}</strong>
                    <span>Added {new Date(passkey.createdAt).toLocaleDateString()}</span>
                    <span>{passkey.lastUsedAt ? `Last used ${new Date(passkey.lastUsedAt).toLocaleString()}` : 'Not used yet'}</span>
                    <button
                      className="button ghost"
                      onClick={() => {
                        if (window.confirm(`Revoke passkey “${passkey.name}”? You cannot undo this action.`)) {
                          revoke.mutate(passkey.id)
                        }
                      }}
                    >
                      Revoke passkey
                    </button>
                  </article>
                ))}
              </div>
            ) : (
              <p>No passkeys registered.</p>
            )}
          </div>
        </>
      </QueryState>
      <div className="security-grid">
        <div>
          <h3>Link email sign-in</h3>
          <label>
            Email address
            <input type="email" autoComplete="email" value={email} onChange={(event) => setEmail(event.target.value)} />
          </label>
          {challenge && (
            <>
              <p role="status">If delivery is available, a six-digit linking code has been sent.</p>
              <label>
                Verification code
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
              {challenge ? 'Verify and link email' : 'Send linking code'}
            </button>
            {challenge && (
              <button
                className="button ghost"
                disabled={resendSeconds > 0 || linkRequest.isPending}
                onClick={() => linkRequest.mutate()}
              >
                {resendSeconds > 0 ? `Resend in ${resendSeconds}s` : 'Resend code'}
              </button>
            )}
          </div>
        </div>
        <div>
          <h3>Add a passkey</h3>
          <label>
            Passkey name
            <input maxLength={80} value={passkeyName} onChange={(event) => setPasskeyName(event.target.value)} />
          </label>
          <button
            className="button secondary"
            disabled={!passkeyName.trim() || !security.data?.linkedEmails.length || register.isPending}
            onClick={() => register.mutate()}
          >
            Register passkey
          </button>
        </div>
      </div>
      {(linkRequest.error || linkVerify.error || register.error || revoke.error) && (
        <ErrorText error={linkRequest.error ?? linkVerify.error ?? register.error ?? revoke.error} />
      )}
    </section>
  )
}

function ProfileSettings({ user }: { user: User }) {
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
      <h2>Public profile</h2>
      <p>Your sign-in email and Google display name are private. Empty nickname means Anonymous user.</p>
      <form
        className="settings-form"
        onSubmit={(event) => {
          event.preventDefault()
          save.mutate()
        }}
      >
        <label>
          Public nickname
          <input value={nickname} maxLength={40} onChange={(event) => setNickname(event.target.value)} />
        </label>
        <button className="button primary" disabled={save.isPending}>Save profile</button>
        {save.isSuccess && <p role="status">Profile saved.</p>}
        {save.error && <ErrorText error={save.error} />}
      </form>
    </section>
  )
}

function ImportTokenSettings() {
  const queryClient = useQueryClient()
  const tokens = useQuery({ queryKey: ['import-tokens'], queryFn: api.importTokens })
  const [name, setName] = useState('My AI client')
  const [secret, setSecret] = useState<string | null>(null)
  const create = useMutation({
    mutationFn: () => api.createImportToken(name, 90),
    onSuccess: async (created) => {
      setSecret(created.secret)
      await queryClient.invalidateQueries({ queryKey: ['import-tokens'] })
    },
  })
  const revoke = useMutation({
    mutationFn: api.revokeImportToken,
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: ['import-tokens'] }),
  })
  return (
    <section className="settings-section">
      <h2>AI recipe import tokens</h2>
      <p>Tokens have only the recipes:import scope. Store the secret in your AI client secret store; it is shown once.</p>
      <form
        className="inline-form compact"
        onSubmit={(event) => {
          event.preventDefault()
          create.mutate()
        }}
      >
        <label>
          Token name
          <input value={name} onChange={(event) => setName(event.target.value)} required maxLength={80} />
        </label>
        <button className="button primary" disabled={create.isPending}>Create 90-day token</button>
      </form>
      {secret && (
        <div className="secret-once" role="status">
          <strong>Copy this secret now. It cannot be retrieved later.</strong>
          <code>{secret}</code>
          <button className="button ghost" onClick={() => setSecret(null)}>I saved it</button>
        </div>
      )}
      <QueryState query={tokens} empty="No import tokens.">
        <div className="card-list">
          {tokens.data?.map((token) => (
            <article className="recipe-card" key={token.id}>
              <strong>{token.name}</strong>
              <span>{token.prefix}… · {token.scope}</span>
              <span>{token.revokedAt ? 'Revoked' : `Expires ${token.expiresAt ?? 'never'}`}</span>
              {!token.revokedAt && (
                <button
                  className="button ghost"
                  onClick={() => {
                    if (window.confirm(`Revoke import token “${token.name}”? You cannot undo this action.`)) {
                      revoke.mutate(token.id)
                    }
                  }}
                >
                  Revoke token
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
  const drafts = useQuery({ queryKey: ['import-drafts'], queryFn: api.importDrafts })
  return (
    <Page title="Recipe import drafts" intro="AI output is untrusted. Review and edit every draft before creating a private recipe.">
      <QueryState query={drafts} empty="No import drafts yet. Use the official Cook skill from your AI client.">
        <div className="card-list">
          {drafts.data?.map((draft) => (
            <article className="recipe-card" key={draft.id}>
              <span className="status verify">{draft.status}</span>
              <h2>{draft.title}</h2>
              <p>{draft.sourceTitle ?? draft.sourceUrl}</p>
              <Link to="/imports/$draftId" params={{ draftId: draft.id }}>Review draft</Link>
            </article>
          ))}
        </div>
      </QueryState>
    </Page>
  )
}

function ImportDraftPage() {
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
    <Page title="Review import draft" intro="Confirm the source, warnings, ingredients, and rewritten steps before accepting.">
      <QueryState query={draft} empty="Import draft not found.">
        {draft.data && (
          <>
            <ImportDraftEditor draft={draft.data} />
            <section className="source-note">
              <h2>Source and extraction warnings</h2>
              <a href={draft.data.sourceUrl} rel="noreferrer">{draft.data.sourceTitle ?? draft.data.sourceUrl}</a>
              {draft.data.sourceAuthor && <p>Source author: {draft.data.sourceAuthor}</p>}
              <p>Confidence: {draft.data.confidence ?? 'not provided'}</p>
              {draft.data.warnings.map((warning) => <p key={warning}>{warning}</p>)}
            </section>
            {draft.data.status === 'draft' && (
              <div className="dialog-actions">
                <button
                  className="button ghost"
                  onClick={() => {
                    if (window.confirm('Discard this import draft? You cannot undo this action.')) {
                      discard.mutate()
                    }
                  }}
                >
                  Discard draft
                </button>
                <button className="button primary" onClick={() => accept.mutate()}>Accept as private recipe</button>
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
  const queryClient = useQueryClient()
  const form = useForm<RecipeForm>({
    resolver: zodResolver(recipeSchema),
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
  if (draft.status !== 'draft') return <StatePanel title="Draft closed" detail={`This draft is ${draft.status}.`} />
  return (
    <form className="editor-form" onSubmit={form.handleSubmit((value) => save.mutate(value))}>
      <label>Title<input {...form.register('title')} /></label>
      <label>Description<textarea rows={3} {...form.register('description')} /></label>
      <label>Instructions<textarea rows={10} {...form.register('instructions')} /></label>
      <fieldset>
        <legend>Ingredients</legend>
        {fields.fields.map((field, index) => (
          <div className="ingredient-row" key={field.id}>
            <input aria-label={`Ingredient ${index + 1}`} {...form.register(`ingredients.${index}.name`)} />
            <input aria-label={`Quantity ${index + 1}`} {...form.register(`ingredients.${index}.quantityNote`)} />
            {fields.fields.length > 1 && <button type="button" className="button ghost" onClick={() => fields.remove(index)}>Remove</button>}
          </div>
        ))}
        <button type="button" className="button secondary" onClick={() => fields.append({ name: '', quantityNote: '', canonicalIngredientId: null })}>Add ingredient</button>
      </fieldset>
      <button className="button secondary" disabled={save.isPending}>Save draft changes</button>
      {save.isSuccess && <p role="status">Draft saved.</p>}
      {save.error && <ErrorText error={save.error} />}
    </form>
  )
}

function SettingsForm({ initial }: { initial: RecommendationSettings }) {
  const queryClient = useQueryClient()
  const form = useForm<RecommendationSettings>({ defaultValues: initial })
  const save = useMutation({
    mutationFn: api.saveSettings,
    onSuccess: (value) => queryClient.setQueryData(['settings'], value),
  })
  return (
    <form className="settings-form" onSubmit={form.handleSubmit((value) => save.mutate(value))}>
      <label className="check-row">
        <input type="checkbox" {...form.register('includeFavorites')} /> Favorites
      </label>
      <label className="check-row">
        <input type="checkbox" {...form.register('includeOwn')} /> My recipes
      </label>
      <label className="check-row">
        <input type="checkbox" {...form.register('includePublic')} /> Other users’ public recipes
      </label>
      <label>
        Recheck pantry after this many days
        <input type="number" min={1} max={90} {...form.register('verificationDays', { valueAsNumber: true })} />
      </label>
      <button className="button primary" disabled={save.isPending}>
        Save sources
      </button>
      {save.isSuccess && <p role="status">Saved.</p>}
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
          <Dialog.Title>Quick pantry check</Dialog.Title>
          <Dialog.Description>
            Confirm up to five uncertain ingredients. Skip any item you cannot answer now.
          </Dialog.Description>
          <div className="confirmation-list">
            {names.map((name) => (
              <fieldset key={name}>
                <legend>{name}</legend>
                {[
                  ['have', 'Still have it'],
                  ['buy', 'Can buy it'],
                  ['used', 'Used up'],
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
              <button className="button ghost">Skip for now</button>
            </Dialog.Close>
            <button className="button primary" disabled={Object.keys(answers).length === 0 || save.isPending} onClick={() => save.mutate()}>
              Save answers
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
  return (
    <div className="page">
      <header className="page-header">
        <div>
          <p className="eyebrow">Cook workspace</p>
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
  if (query.isLoading) return <StatePanel title="Loading…" detail="Fetching the latest kitchen data." />
  if (query.error) return <StatePanel title="Something went wrong" detail={errorMessage(query.error)} />
  if (Array.isArray(query.data) && query.data.length === 0) return <StatePanel title="Nothing here yet" detail={empty} />
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
  return <p className="error" role="alert">{errorMessage(error)}</p>
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Unexpected error'
}

function nextStatus(status: PantryStatus): PantryStatus {
  return status === 'have' ? 'unknown' : status === 'unknown' ? 'out' : 'have'
}

function pantryStatusLabel(status: PantryStatus): string {
  return status === 'have' ? 'Available' : status === 'out' ? 'Used up' : 'Uncertain'
}
