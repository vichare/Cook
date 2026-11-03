import { useRef } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { useI18n } from './i18n'

const allowedElements = [
  'p',
  'br',
  'hr',
  'h1',
  'h2',
  'h3',
  'h4',
  'strong',
  'em',
  'blockquote',
  'ul',
  'ol',
  'li',
  'a',
  'table',
  'thead',
  'tbody',
  'tr',
  'th',
  'td',
]

export function safeMarkdownUrl(value: string): string {
  try {
    const url = new URL(value)
    return url.protocol === 'http:' || url.protocol === 'https:' ? url.toString() : ''
  } catch {
    return ''
  }
}

export function SafeMarkdown({ value }: { value: string }) {
  return (
    <div className="safe-markdown">
      <ReactMarkdown
        allowedElements={allowedElements}
        skipHtml
        remarkPlugins={[remarkGfm]}
        urlTransform={safeMarkdownUrl}
        components={{
          a: ({ children, href }) =>
            href ? (
              <a href={href} target="_blank" rel="noopener noreferrer">
                {children}
              </a>
            ) : (
              <span>{children}</span>
            ),
        }}
      >
        {value}
      </ReactMarkdown>
    </div>
  )
}

type Format = 'strong' | 'emphasis' | 'heading' | 'list' | 'link' | 'table'

export function applyMarkdownFormat(
  value: string,
  start: number,
  end: number,
  format: Format,
): { value: string; selectionStart: number; selectionEnd: number } {
  const selected = value.slice(start, end)
  let replacement: string
  let selectFrom = 0
  let selectLength = 0
  switch (format) {
    case 'strong':
      replacement = `**${selected || 'text'}**`
      selectFrom = 2
      selectLength = selected.length || 4
      break
    case 'emphasis':
      replacement = `*${selected || 'text'}*`
      selectFrom = 1
      selectLength = selected.length || 4
      break
    case 'heading':
      replacement = `## ${selected || 'Heading'}`
      selectFrom = 3
      selectLength = selected.length || 7
      break
    case 'list':
      replacement = (selected || 'Item')
        .split('\n')
        .map((line) => `- ${line}`)
        .join('\n')
      selectFrom = 2
      selectLength = selected.length || 4
      break
    case 'link':
      replacement = `[${selected || 'link text'}](https://)`
      selectFrom = replacement.indexOf('https://')
      selectLength = 'https://'.length
      break
    case 'table':
      replacement = '| Column 1 | Column 2 |\n| --- | --- |\n| Value 1 | Value 2 |'
      selectFrom = 2
      selectLength = 8
      break
  }
  return {
    value: `${value.slice(0, start)}${replacement}${value.slice(end)}`,
    selectionStart: start + selectFrom,
    selectionEnd: start + selectFrom + selectLength,
  }
}

export function MarkdownEditor({
  id,
  label,
  value,
  onChange,
  maxLength,
}: {
  id: string
  label: string
  value: string
  onChange: (value: string) => void
  maxLength: number
}) {
  const { t } = useI18n()
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  function format(kind: Format) {
    const textarea = textareaRef.current
    if (!textarea) return
    const next = applyMarkdownFormat(
      value,
      textarea.selectionStart,
      textarea.selectionEnd,
      kind,
    )
    if (next.value.length > maxLength) return
    onChange(next.value)
    window.requestAnimationFrame(() => {
      textarea.focus()
      textarea.setSelectionRange(next.selectionStart, next.selectionEnd)
    })
  }

  return (
    <div className="markdown-editor">
      <label htmlFor={id}>{label}</label>
      <div className="markdown-toolbar" role="toolbar" aria-label={t('markdown.toolbar')}>
        {(
          [
            ['strong', 'markdown.bold'],
            ['emphasis', 'markdown.italic'],
            ['heading', 'markdown.heading'],
            ['list', 'markdown.list'],
            ['link', 'markdown.link'],
            ['table', 'markdown.table'],
          ] as const
        ).map(([formatName, message]) => (
          <button
            type="button"
            className="button ghost"
            key={formatName}
            onClick={() => format(formatName)}
          >
            {t(message)}
          </button>
        ))}
      </div>
      <div className="markdown-workspace">
        <div>
          <strong>{t('markdown.source')}</strong>
          <textarea
            id={id}
            ref={textareaRef}
            rows={10}
            maxLength={maxLength}
            value={value}
            onChange={(event) => onChange(event.target.value)}
          />
        </div>
        <div className="markdown-preview" aria-live="polite">
          <strong>{t('markdown.preview')}</strong>
          {value.trim() ? <SafeMarkdown value={value} /> : <p>{t('markdown.empty')}</p>}
        </div>
      </div>
      <p className="field-note">{t('markdown.help')}</p>
    </div>
  )
}
