import { useQuery } from '@tanstack/react-query'
import { useEffect, useId, useMemo, useState } from 'react'
import { api } from './api'
import { normalizeIngredient, splitIngredientList } from './ingredients'
import { useI18n } from './i18n'
import type { IngredientReference, IngredientSuggestion } from './types'

export function IngredientCombobox({
  label,
  value,
  onChange,
  onPasteList,
  onCommit,
  placeholder,
  invalid,
  describedBy,
}: {
  label: string
  value: IngredientReference
  onChange: (value: IngredientReference) => void
  onPasteList?: (values: string[]) => void
  onCommit?: (value: IngredientReference) => void
  placeholder?: string
  invalid?: boolean
  describedBy?: string
}) {
  const { t } = useI18n()
  const id = useId()
  const listId = `${id}-listbox`
  const statusId = `${id}-status`
  const [open, setOpen] = useState(false)
  const [activeIndex, setActiveIndex] = useState(-1)
  const [debouncedQuery, setDebouncedQuery] = useState('')
  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedQuery(value.name.trim()), 150)
    return () => window.clearTimeout(timer)
  }, [value.name])
  const suggestions = useQuery({
    queryKey: ['ingredient-suggestions', debouncedQuery],
    queryFn: () => api.ingredientSuggestions(debouncedQuery),
    enabled: debouncedQuery.length > 0,
    staleTime: 30_000,
  })
  const queryReady = debouncedQuery === value.name.trim()
  const rawItems = useMemo(
    () => (queryReady ? suggestions.data ?? [] : []),
    [queryReady, suggestions.data],
  )
  const normalizedQuery = normalizeIngredient(value.name)
  const hasHan = /\p{Script=Han}/u.test(normalizedQuery)
  const singleHan = hasHan && [...normalizedQuery].length === 1
  const exactItems = useMemo(
    () => rawItems.filter((item) => {
      if (item.reason !== 'exact' && item.reason !== 'alias') return false
      const matched = normalizeIngredient(item.matchedText)
      const display = normalizeIngredient(item.displayName)
      return normalizedQuery === matched || (item.reason === 'exact' && normalizedQuery === display)
    }),
    [normalizedQuery, rawItems],
  )
  const uniqueExactMatches = useMemo(
    () => [...new Map(
      exactItems.map((item) => [item.canonicalIngredientId, item]),
    ).values()],
    [exactItems],
  )
  const uniqueExactMatch = uniqueExactMatches.length === 1
  const advisoryItems = useMemo(
    () => [...new Map(
      rawItems
        .filter((item) => !hasHan || (item.reason !== 'substring' && item.reason !== 'similar'))
        .map((item) => [item.canonicalIngredientId, item]),
    ).values()],
    [hasHan, rawItems],
  )
  const items = (uniqueExactMatches.length > 0 ? uniqueExactMatches : advisoryItems).slice(0, 4)
  const hasStrictSingleHanMatches = singleHan && uniqueExactMatches.length > 0
  const showKeep = uniqueExactMatches.length === 0 && !singleHan
  const optionCount =
    singleHan && !hasStrictSingleHanMatches ? 0 : items.length + (showKeep ? 1 : 0)
  const candidateOffset = showKeep ? 1 : 0
  const listVisible = open && optionCount > 0
  const selected = useMemo(
    () => items.find((item) => item.canonicalIngredientId === value.canonicalIngredientId),
    [items, value.canonicalIngredientId],
  )
  useEffect(() => {
    const exactMatch = uniqueExactMatch ? uniqueExactMatches[0] : null
    if (exactMatch && value.canonicalIngredientId !== exactMatch.canonicalIngredientId) {
      onChange({
        name: value.name || exactMatch.displayName,
        canonicalIngredientId: exactMatch.canonicalIngredientId,
      })
    }
  }, [onChange, uniqueExactMatch, uniqueExactMatches, value.canonicalIngredientId, value.name])
  const activeStatus =
    activeIndex >= 0
      ? showKeep && activeIndex === 0
        ? t('ingredient.optionKeep', { current: 1, total: optionCount, input: value.name })
        : t('ingredient.optionSuggestion', {
            current: activeIndex + 1,
            total: optionCount,
            candidate: items[activeIndex - candidateOffset].displayName,
          })
      : null

  function choose(suggestion: IngredientSuggestion) {
    const next = {
      name: value.name || suggestion.displayName,
      canonicalIngredientId: suggestion.canonicalIngredientId,
    }
    onChange(next)
    onCommit?.(next)
    setOpen(false)
    setActiveIndex(-1)
  }

  return (
    <div className="ingredient-combobox">
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        role="combobox"
        aria-autocomplete="list"
        aria-haspopup="listbox"
        aria-expanded={listVisible}
        aria-controls={listId}
        aria-describedby={[statusId, describedBy].filter(Boolean).join(' ')}
        aria-invalid={invalid || undefined}
        aria-busy={!queryReady || suggestions.isFetching}
        aria-activedescendant={listVisible && activeIndex >= 0 ? `${id}-option-${activeIndex}` : undefined}
        autoComplete="off"
        placeholder={placeholder}
        value={value.name}
        onFocus={() => setOpen(value.name.trim().length > 0)}
        onChange={(event) => {
          onChange({ name: event.target.value, canonicalIngredientId: null })
          setOpen(event.target.value.trim().length > 0)
          setActiveIndex(-1)
        }}
        onPaste={(event) => {
          const values = splitIngredientList(event.clipboardData.getData('text'))
          if (values.length > 1 && onPasteList) {
            event.preventDefault()
            onPasteList(values)
            setOpen(false)
          }
        }}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown') {
            if (optionCount === 0) return
            event.preventDefault()
            setOpen(true)
            setActiveIndex((current) => (current + 1 + optionCount) % optionCount)
          } else if (event.key === 'ArrowUp') {
            if (optionCount === 0) return
            event.preventDefault()
            setOpen(true)
            setActiveIndex((current) => (current - 1 + optionCount) % optionCount)
          } else if (event.key === 'Enter' && listVisible && activeIndex >= 0) {
            event.preventDefault()
            if (showKeep && activeIndex === 0) {
              onChange({ name: value.name, canonicalIngredientId: null })
              onCommit?.({ name: value.name, canonicalIngredientId: null })
              setOpen(false)
            } else choose(items[activeIndex - candidateOffset])
          } else if (event.key === 'Enter' && value.name.trim() && onCommit) {
            event.preventDefault()
            onCommit(value)
            setOpen(false)
          } else if (event.key === 'Escape') {
            setOpen(false)
            setActiveIndex(-1)
          }
        }}
      />
      <div id={statusId} className="combobox-status" aria-live="polite">
        {activeStatus ??
        (!queryReady || suggestions.isFetching
          ? t('ingredient.searching')
          : selected
            ? t('ingredient.matched', { name: selected.displayName })
            : value.canonicalIngredientId
              ? t('ingredient.canonicalSelected')
              : singleHan && uniqueExactMatches.length === 0
                ? t('ingredient.shortCjk')
              : items.length > 0
                ? uniqueExactMatch
                  ? t('ingredient.oneExactMatch')
                  : items.length === 1
                  ? t('ingredient.oneSuggestion')
                  : t('ingredient.suggestions', { count: items.length })
                : value.name.trim()
                  ? t('ingredient.noMatches')
                  : '')}
      </div>
      {value.canonicalIngredientId && !uniqueExactMatch && (
        <button
          type="button"
          className="button ghost ingredient-undo"
          onClick={() => {
            onChange({ name: value.name, canonicalIngredientId: null })
            setOpen(value.name.trim().length > 0)
            setActiveIndex(-1)
          }}
        >
          {t('ingredient.undo')}
        </button>
      )}
      {listVisible && value.name.trim() && (
        <div
          id={listId}
          role="listbox"
          className="ingredient-listbox"
          aria-label={t('ingredient.suggestionsLabel', { label })}
        >
          {showKeep && (
            <button
              type="button"
              role="option"
              id={`${id}-option-0`}
              aria-selected={!value.canonicalIngredientId}
              className={activeIndex === 0 ? 'ingredient-option active' : 'ingredient-option'}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => {
                const next = { name: value.name, canonicalIngredientId: null }
                onChange(next)
                onCommit?.(next)
                setOpen(false)
                setActiveIndex(-1)
              }}
            >
              <strong>{t('ingredient.keep', { name: value.name })}</strong>
              <span>{t('ingredient.keepDetail', { name: value.name })}</span>
            </button>
          )}
          {items.map((item, index) => {
            const optionIndex = index + candidateOffset
            return (
            <button
              type="button"
              role="option"
              id={`${id}-option-${optionIndex}`}
              aria-selected={value.canonicalIngredientId === item.canonicalIngredientId}
              aria-label={t('ingredient.existingCandidateLabel', { name: item.displayName })}
              className={optionIndex === activeIndex ? 'ingredient-option active' : 'ingredient-option'}
              key={item.canonicalIngredientId}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => choose(item)}
            >
              <strong>{item.displayName}</strong>
              <span>{t('ingredient.existingCandidate')}</span>
            </button>
          )})}
          {suggestions.error && <p className="error">{t('ingredient.unavailable')}</p>}
        </div>
      )}
    </div>
  )
}
