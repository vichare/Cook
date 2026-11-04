import { useCallback, useEffect, useRef, useState } from 'react'
import { ApiError, api } from './api'
import { localDateValue } from './ingredients'
import {
  canonicalizeRecommendationControls,
  encodeRecommendationControls,
  parseRecommendationControls,
  RECOMMENDATION_URL_MAX_LENGTH,
  recommendationControlsFromSettings,
  recommendationControlsKey,
  tryDowngradeRecommendationCanonicalIds,
  type RecommendationControls,
  type RecommendationUrlIssue,
} from './recommendation-url'
import type { Recommendation, RecommendationSettings } from './types'

export const RECOMMENDATION_REFRESH_DEBOUNCE_MS = 250

export interface RecommendationResultState {
  data: Recommendation[]
  error: Error | null
  status: 'idle' | 'loading' | 'success' | 'error'
}

export function useRecommendationWorkbench(settings: RecommendationSettings | undefined) {
  const [controls, setControlsState] = useState<RecommendationControls | null>(null)
  const [activeControls, setActiveControls] = useState<RecommendationControls | null>(null)
  const [urlIssues, setUrlIssues] = useState<RecommendationUrlIssue[]>([])
  const [retrySequence, setRetrySequence] = useState(0)
  const [results, setResults] = useState<RecommendationResultState>({
    data: [],
    error: null,
    status: 'idle',
  })
  const activeKeyRef = useRef('')
  const requestGeneration = useRef(0)
  const urlCanonicalIds = useRef(new Set<string>())
  const latestControls = useRef<RecommendationControls | null>(null)
  const activeRequestController = useRef<AbortController | null>(null)
  const activeRequestKey = useRef('')
  const scheduledRequestKey = useRef('')
  const successfulResults = useRef(new Map<string, Recommendation[]>())

  const invalidateActiveRequest = useCallback(() => {
    requestGeneration.current += 1
    activeRequestKey.current = ''
    scheduledRequestKey.current = ''
    activeRequestController.current?.abort()
    activeRequestController.current = null
  }, [])

  const applyLocation = useCallback((nextSettings: RecommendationSettings) => {
    const defaults = recommendationControlsFromSettings(nextSettings)
    const parsed = parseRecommendationControls(window.location.search, defaults)
    const query = encodeRecommendationControls(parsed.controls)
    invalidateActiveRequest()
    replaceRecommendationUrl(query)
    activeKeyRef.current = recommendationControlsKey(parsed.controls)
    scheduledRequestKey.current = activeKeyRef.current
    urlCanonicalIds.current = canonicalIds(parsed.controls)
    latestControls.current = parsed.controls
    setUrlIssues(parsed.issues)
    setControlsState(parsed.controls)
    setActiveControls(parsed.controls)
    setResults((current) => ({ ...current, error: null, status: 'loading' }))
  }, [invalidateActiveRequest])

  useEffect(() => {
    if (!settings || controls) return
    applyLocation(settings)
  }, [applyLocation, controls, settings])

  useEffect(() => {
    if (!settings) return
    const onPopState = () => applyLocation(settings)
    window.addEventListener('popstate', onPopState)
    return () => window.removeEventListener('popstate', onPopState)
  }, [applyLocation, settings])

  useEffect(() => {
    if (!controls) return
    const key = recommendationControlsKey(controls)
    if (key === activeKeyRef.current) {
      const requestIsActive = activeRequestKey.current === key
        && !activeRequestController.current?.signal.aborted
      if (requestIsActive || scheduledRequestKey.current === key) return
      const cached = successfulResults.current.get(key)
      if (cached) {
        setResults({ data: cached, error: null, status: 'success' })
        return
      }
      scheduledRequestKey.current = key
      if (
        activeControls
        && recommendationControlsKey(activeControls) === key
      ) {
        setRetrySequence((sequence) => sequence + 1)
      } else {
        setActiveControls(controls)
      }
      return
    }
    const timer = window.setTimeout(() => {
      replaceRecommendationUrl(encodeRecommendationControls(controls))
      activeKeyRef.current = key
      scheduledRequestKey.current = key
      setUrlIssues([])
      setActiveControls(controls)
    }, RECOMMENDATION_REFRESH_DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [activeControls, controls])

  useEffect(() => {
    if (!activeControls) return
    const generation = ++requestGeneration.current
    const controller = new AbortController()
    const requestKey = recommendationControlsKey(activeControls)
    activeRequestController.current = controller
    activeRequestKey.current = requestKey
    scheduledRequestKey.current = requestKey
    setResults((current) => ({ ...current, error: null, status: 'loading' }))
    const request = {
      preferred: activeControls.preferred,
      required: activeControls.required,
      excluded: activeControls.excluded,
      ingredientScope: activeControls.ingredientScope,
      ...activeControls.sources,
      today: localDateValue(new Date()),
    }
    void api.recommendations(request, controller.signal).then(
      (data) => {
        if (
          generation !== requestGeneration.current
          || activeRequestKey.current !== requestKey
        ) return
        scheduledRequestKey.current = ''
        successfulResults.current.set(requestKey, data)
        setResults({ data, error: null, status: 'success' })
      },
      (error: unknown) => {
        if (
          controller.signal.aborted
          || generation !== requestGeneration.current
          || activeRequestKey.current !== requestKey
        ) return
        scheduledRequestKey.current = ''
        if (error instanceof ApiError && error.code === 'canonical_ingredient_unavailable') {
          const currentControls = latestControls.current
          const currentCanonicalIds = currentControls ? canonicalIds(currentControls) : new Set<string>()
          const eligible = new Set(
            error.canonicalIngredientIds.filter((id) =>
              urlCanonicalIds.current.has(id) && currentCanonicalIds.has(id)
            ),
          )
          if (eligible.size > 0 && currentControls) {
            const downgrade = tryDowngradeRecommendationCanonicalIds(currentControls, eligible)
            if (!downgrade.controls) {
              setUrlIssues((issues) => [
                ...new Set([...issues, 'canonicalDowngradeConflict' as const]),
              ])
              setResults((current) => ({
                ...current,
                error,
                status: 'error',
              }))
              return
            }
            const downgraded = downgrade.controls
            for (const id of eligible) urlCanonicalIds.current.delete(id)
            const query = encodeRecommendationControls(downgraded)
            replaceRecommendationUrl(query)
            activeKeyRef.current = recommendationControlsKey(downgraded)
            scheduledRequestKey.current = activeKeyRef.current
            latestControls.current = downgraded
            setUrlIssues((issues) => [...new Set([...issues, 'canonicalDowngraded' as const])])
            setControlsState(downgraded)
            setActiveControls(downgraded)
            return
          }
        }
        setResults((current) => ({
          ...current,
          error: error instanceof Error ? error : new Error(String(error)),
          status: 'error',
        }))
      },
    )
    return () => {
      controller.abort()
      if (activeRequestController.current === controller) {
        activeRequestController.current = null
      }
    }
  }, [activeControls, retrySequence])

  const setControls = useCallback((next: RecommendationControls) => {
    const canonical = canonicalizeRecommendationControls(next)
    if (encodeRecommendationControls(canonical).length > RECOMMENDATION_URL_MAX_LENGTH) {
      setUrlIssues((issues) => [...new Set([...issues, 'oversized' as const])])
      return
    }
    if (
      latestControls.current
      && recommendationControlsKey(latestControls.current) === recommendationControlsKey(canonical)
    ) return
    const retainedCanonicalIds = canonicalIds(canonical)
    urlCanonicalIds.current = new Set(
      [...urlCanonicalIds.current].filter((id) => retainedCanonicalIds.has(id)),
    )
    latestControls.current = canonical
    invalidateActiveRequest()
    setResults((current) => ({ ...current, error: null, status: 'loading' }))
    setControlsState(canonical)
  }, [invalidateActiveRequest])

  const retry = useCallback(() => {
    const current = latestControls.current
    if (!current) return
    invalidateActiveRequest()
    const key = recommendationControlsKey(current)
    successfulResults.current.delete(key)
    replaceRecommendationUrl(encodeRecommendationControls(current))
    activeKeyRef.current = key
    scheduledRequestKey.current = key
    setActiveControls(current)
    setRetrySequence((sequence) => sequence + 1)
  }, [invalidateActiveRequest])

  const updateResults = useCallback((update: (current: Recommendation[]) => Recommendation[]) => {
    setResults((current) => {
      const data = update(current.data)
      if (current.status === 'success' && activeKeyRef.current) {
        successfulResults.current.set(activeKeyRef.current, data)
      }
      return { ...current, data }
    })
  }, [])

  return {
    activeControls,
    controls,
    results,
    retry,
    setControls,
    updateResults,
    urlIssues,
  }
}

function canonicalIds(controls: RecommendationControls): Set<string> {
  return new Set(
    [controls.preferred, controls.required, controls.excluded]
      .flat()
      .flatMap((value) => value.canonicalIngredientId ? [value.canonicalIngredientId] : []),
  )
}

function replaceRecommendationUrl(query: string) {
  const next = `${window.location.pathname}?${query}${window.location.hash}`
  const current = `${window.location.pathname}${window.location.search}${window.location.hash}`
  if (next !== current) window.history.replaceState(window.history.state, '', next)
}
