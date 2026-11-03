import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  AccountSecuritySettings,
  EmailOtpLogin,
  ExpirationEditor,
  GuestLanguageSelector,
  LanguageSettings,
  PantryConfirmation,
  ProfileSetup,
} from './router'
import { LocaleProvider, type Locale } from './i18n'

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

const { requestEmailLink, requestEmailOtp, saveLocale, saveProfile, securitySettings, verifyEmailLink, verifyEmailOtp } = vi.hoisted(() => ({
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
}))

vi.mock('./api', async () => {
  const actual = await vi.importActual<typeof import('./api')>('./api')
  return {
    ...actual,
    api: {
      ...actual.api,
      savePantry: vi.fn().mockResolvedValue([]),
      saveProfile,
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

  describe('expiration editor', () => {
    it('offers large explicit shortcuts and can clear the date', () => {
      const onChange = vi.fn()
      render(
        <Providers>
          <ExpirationEditor value="2026-01-31" onChange={onChange} now={new Date(2026, 0, 31, 12)} />
        </Providers>,
      )
      expect(screen.getByRole('button', { name: 'Today' })).toHaveAttribute('aria-pressed', 'true')
      fireEvent.click(screen.getByRole('button', { name: 'One month' }))
      expect(onChange).toHaveBeenLastCalledWith('2026-02-28')
      fireEvent.click(screen.getByRole('button', { name: 'No expiration date' }))
      expect(onChange).toHaveBeenLastCalledWith(null)
    })

    it('warns before saving a past date without clearing or changing pantry status', () => {
      render(
        <Providers>
          <ExpirationEditor value="2026-01-30" onChange={() => undefined} now={new Date(2026, 0, 31, 12)} />
        </Providers>,
      )
      expect(screen.getByRole('alert')).toHaveTextContent(
        'This date is in the past. You can still save it',
      )
      expect(screen.getByLabelText('Choose a calendar date')).toHaveValue('2026-01-30')
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
