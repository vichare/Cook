import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  AccountSecuritySettings,
  EmailOtpLogin,
  ExpirationEditor,
  PantryConfirmation,
  ProfileSetup,
} from './router'

const { requestEmailLink, requestEmailOtp, saveProfile, securitySettings, verifyEmailLink, verifyEmailOtp } = vi.hoisted(() => ({
  saveProfile: vi.fn().mockImplementation((nickname: string | null) =>
    Promise.resolve({
      id: 'user',
      email: 'private@example.test',
      displayName: 'Google Name',
      nickname,
      onboardingCompleted: true,
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
  }),
  securitySettings: vi.fn().mockResolvedValue({ linkedEmails: [], passkeys: [] }),
  requestEmailLink: vi.fn().mockResolvedValue({
    challengeId: 'link-challenge',
    expiresAt: '2026-11-04T00:10:00Z',
    resendAllowedAt: '2026-11-04T00:00:00Z',
  }),
  verifyEmailLink: vi.fn().mockResolvedValue({
    linkedEmails: ['linked@example.test'],
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
  }

  it('defaults to locale-rendered anonymous state without publishing the Google name', async () => {
    saveProfile.mockClear()
    render(
      <QueryClientProvider client={new QueryClient()}>
        <ProfileSetup user={user} />
      </QueryClientProvider>,
    )
    expect(screen.getByLabelText('Anonymous user (default)')).toBeChecked()
    fireEvent.click(screen.getByRole('button', { name: 'Save public profile' }))
    await waitFor(() => expect(saveProfile).toHaveBeenCalledWith(null))
  })

  it('copies the Google display name only after explicit selection', async () => {
    saveProfile.mockClear()
    render(
      <QueryClientProvider client={new QueryClient()}>
        <ProfileSetup user={user} />
      </QueryClientProvider>,
    )
    fireEvent.click(screen.getByLabelText('Use “Google Name” publicly'))
    fireEvent.click(screen.getByRole('button', { name: 'Save public profile' }))
    await waitFor(() => expect(saveProfile).toHaveBeenCalledWith('Google Name'))
  })
})

describe('pantry confirmation', () => {
  it('supports skipping and clears skipped answers', () => {
    render(
      <QueryClientProvider client={new QueryClient()}>
        <PantryConfirmation open onOpenChange={() => undefined} names={['Egg', 'Milk', 'Rice']} />
      </QueryClientProvider>,
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
        <ExpirationEditor value="2026-01-31" onChange={onChange} now={new Date(2026, 0, 31, 12)} />,
      )
      expect(screen.getByRole('button', { name: 'Today' })).toHaveAttribute('aria-pressed', 'true')
      fireEvent.click(screen.getByRole('button', { name: 'One month' }))
      expect(onChange).toHaveBeenLastCalledWith('2026-02-28')
      fireEvent.click(screen.getByRole('button', { name: 'No expiration date' }))
      expect(onChange).toHaveBeenLastCalledWith(null)
    })
  })
})

describe('email OTP sign-in', () => {
  it('accepts a pasted six-digit code with one-time-code autocomplete', async () => {
    requestEmailOtp.mockClear()
    verifyEmailOtp.mockClear()
    const onLogin = vi.fn()
    render(
      <QueryClientProvider client={new QueryClient()}>
        <EmailOtpLogin onLogin={onLogin} passkeyEnabled={false} />
      </QueryClientProvider>,
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
    securitySettings.mockResolvedValueOnce({ linkedEmails: [], passkeys: [] })
    render(
      <QueryClientProvider client={new QueryClient()}>
        <AccountSecuritySettings />
      </QueryClientProvider>,
    )
    expect(await screen.findByRole('button', { name: 'Register passkey' })).toBeDisabled()
    fireEvent.change(screen.getByLabelText('Email address'), { target: { value: 'linked@example.test' } })
    fireEvent.click(screen.getByRole('button', { name: 'Send linking code' }))
    const code = await screen.findByLabelText('Verification code')
    fireEvent.change(code, { target: { value: '654321' } })
    fireEvent.click(screen.getByRole('button', { name: 'Verify and link email' }))
    await waitFor(() => expect(verifyEmailLink).toHaveBeenCalledWith('link-challenge', '654321'))
  })
})
