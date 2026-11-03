import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { StrictMode, useState } from 'react'
import { afterEach, describe, expect, it } from 'vitest'
import { LocaleProvider } from './i18n'
import { applyMarkdownFormat, MarkdownEditor, SafeMarkdown, safeMarkdownUrl } from './MarkdownEditor'

afterEach(cleanup)

function Harness({ initial = '' }: { initial?: string }) {
  const [value, setValue] = useState(initial)
  return (
    <LocaleProvider initialLocale="en">
      <MarkdownEditor
        id="instructions"
        label="Instructions"
        value={value}
        onChange={setValue}
        maxLength={20000}
      />
    </LocaleProvider>
  )
}

describe('safe Markdown editing', () => {
  it('preserves Chinese, emoji, nested lists, tables, and safe links', () => {
    const value =
      '## 步骤 🍲\n\n- 第一项\n  - 子项\n\n| 食材 | 数量 |\n| --- | --- |\n| 米 | 1 杯 |\n\n[来源](https://example.test/recipe)'
    render(
      <LocaleProvider initialLocale="zh-CN">
        <SafeMarkdown value={value} />
      </LocaleProvider>,
    )
    expect(screen.getByRole('heading', { name: '步骤 🍲' })).toBeVisible()
    expect(screen.getByRole('table')).toBeVisible()
    expect(screen.getByRole('link', { name: '来源' })).toHaveAttribute(
      'rel',
      'noopener noreferrer',
    )
  })

  it('removes raw HTML and rejects dangerous URL schemes', () => {
    const { container } = render(
      <SafeMarkdown value={'<script>alert(1)</script>\n\n[x](javascript:alert(1))\n\n![x](data:text/html,x)'} />,
    )
    expect(container.querySelector('script')).toBeNull()
    expect(screen.queryByRole('link', { name: 'x' })).toBeNull()
    expect(container.querySelector('img')).toBeNull()
    expect(safeMarkdownUrl('vbscript:msgbox(1)')).toBe('')
    expect(safeMarkdownUrl('https://example.test/path')).toBe('https://example.test/path')
  })

  it('formats the current selection and keeps source and preview synchronized', () => {
    render(<Harness initial="Stir gently" />)
    const textarea = screen.getByLabelText('Instructions')
    textarea.focus()
    ;(textarea as HTMLTextAreaElement).setSelectionRange(0, 4)
    fireEvent.click(screen.getByRole('button', { name: 'Bold' }))
    expect(textarea).toHaveValue('**Stir** gently')
    expect(screen.getByText('Stir', { selector: 'strong' })).toBeVisible()
  })

  it('creates deterministic source edits without losing surrounding text', () => {
    expect(applyMarkdownFormat('before after', 7, 12, 'link')).toEqual({
      value: 'before [after](https://)',
      selectionStart: 15,
      selectionEnd: 23,
    })
  })

  it('keeps legacy plain text and malformed Markdown usable in StrictMode', () => {
    const malformed = `Legacy plain text <b>stays safe</b>\n\n[unfinished](javascript:\n\n${'x'.repeat(5000)}`
    const { container } = render(
      <StrictMode>
        <Harness initial={malformed} />
      </StrictMode>,
    )
    expect(screen.getByLabelText('Instructions')).toHaveValue(malformed)
    expect(container.querySelector('.markdown-preview')).toHaveTextContent('Legacy plain text')
    expect(screen.queryByRole('link', { name: 'unfinished' })).toBeNull()
  })
})
