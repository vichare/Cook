import { useQuery } from '@tanstack/react-query'
import { useId, useMemo, useState } from 'react'
import { api } from './api'
import { splitIngredientList } from './ingredients'
import type { IngredientReference, IngredientSuggestion } from './types'

const reasonText: Record<IngredientSuggestion['reason'], string> = {
  exact: 'Exact normalized match',
  alias: 'Your confirmed alias',
  prefix: 'Name starts with your text',
  substring: 'Name contains your text',
  similar: 'Similar spelling — review carefully',
}

export function IngredientCombobox({
  label,
  value,
  onChange,
  onPasteList,
  onCommit,
  placeholder,
}: {
  label: string
  value: IngredientReference
  onChange: (value: IngredientReference) => void
  onPasteList?: (values: string[]) => void
  onCommit?: (value: IngredientReference) => void
  placeholder?: string
}) {
  const id = useId()
  const listId = `${id}-listbox`
  const statusId = `${id}-status`
  const [open, setOpen] = useState(false)
  const [activeIndex, setActiveIndex] = useState(-1)
  const suggestions = useQuery({
    queryKey: ['ingredient-suggestions', value.name],
    queryFn: () => api.ingredientSuggestions(value.name),
    enabled: value.name.trim().length > 0,
    staleTime: 30_000,
  })
  const items = suggestions.data ?? []
  const optionCount = items.length + 1
  const selected = useMemo(
    () => items.find((item) => item.canonicalIngredientId === value.canonicalIngredientId),
    [items, value.canonicalIngredientId],
  )

  function choose(suggestion: IngredientSuggestion) {
    const next = {
      name: value.name.trim() || suggestion.displayName,
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
        aria-expanded={open}
        aria-controls={listId}
        aria-describedby={statusId}
        aria-activedescendant={activeIndex >= 0 ? `${id}-option-${activeIndex}` : undefined}
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
            event.preventDefault()
            setOpen(true)
            setActiveIndex((current) => (current + 1 + optionCount) % optionCount)
          } else if (event.key === 'ArrowUp') {
            event.preventDefault()
            setOpen(true)
            setActiveIndex((current) => (current - 1 + optionCount) % optionCount)
          } else if (event.key === 'Enter' && open && activeIndex >= 0) {
            event.preventDefault()
            if (activeIndex < items.length) choose(items[activeIndex])
            else {
              onChange({ name: value.name, canonicalIngredientId: null })
              setOpen(false)
            }
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
        {suggestions.isFetching
          ? 'Looking for ingredient matches.'
          : selected
            ? `Matched to ${selected.displayName}.`
            : value.canonicalIngredientId
              ? 'Canonical ingredient selected.'
              : items.length > 0
                ? `${items.length} suggestion${items.length === 1 ? '' : 's'} available.`
                : value.name.trim()
                  ? 'No close catalog matches. You can keep your entry.'
                  : ''}
      </div>
      {open && value.name.trim() && (
        <div id={listId} role="listbox" className="ingredient-listbox" aria-label={`${label} suggestions`}>
          {items.map((item, index) => (
            <button
              type="button"
              role="option"
              id={`${id}-option-${index}`}
              aria-selected={value.canonicalIngredientId === item.canonicalIngredientId}
              className={index === activeIndex ? 'ingredient-option active' : 'ingredient-option'}
              key={item.canonicalIngredientId}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => choose(item)}
            >
              <strong>{item.displayName}</strong>
              <span>{reasonText[item.reason]}</span>
              {item.matchedText !== item.displayName && <small>Matched “{item.matchedText}”</small>}
            </button>
          ))}
          <button
            type="button"
            role="option"
            id={`${id}-option-${items.length}`}
            aria-selected={!value.canonicalIngredientId}
            className={items.length === activeIndex ? 'ingredient-option active' : 'ingredient-option'}
            onMouseDown={(event) => event.preventDefault()}
            onClick={() => {
              const next = { name: value.name, canonicalIngredientId: null }
              onChange(next)
              onCommit?.(next)
              setOpen(false)
            }}
          >
            <strong>Keep “{value.name}” as entered</strong>
            <span>Do not bind this entry to a suggestion.</span>
          </button>
          {suggestions.error && <p className="error">Suggestions are unavailable. Your typed value is preserved.</p>}
        </div>
      )}
    </div>
  )
}
