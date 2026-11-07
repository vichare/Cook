# v0.7.0 account assistance release checklist

Automated tests cover the security invariants, but production credentials, real devices, assistive technology, and operational handling still require human verification. Do not tag, release, or deploy until every applicable item is signed off.

## Assignment and authorization

- Create the first `account_assistance` claim code only through the trusted host command; confirm the plaintext appears once and expires after 15 minutes.
- Claim with an existing user and a real Passkey. Confirm an ordinary user and a `content_moderation`-only user receive 404 from account-assistance APIs.
- Confirm claim and administration are denied from an assisted target session.
- Revoke the assignment by UUID on the host. Confirm the assistant's sessions and every target assisted session issued by that assignment fail immediately, every unconsumed challenge for that generation is deleted, and a later claim creates a different immutable assignment ID and alias.

## Issuance and secret handling

- Confirm inspect and issue require the exact production Origin and a Passkey assertion from the current session within ten minutes.
- Confirm the modal names new versus existing account only after email submission and fresh step-up, and warns that existing-account access includes private recipes and pantry data.
- Confirm Cook generates an eight-digit code; administrators cannot choose it.
- Confirm the plaintext OTP is visible once, is absent from URLs, logs, database rows, audit, notifications, browser persistence, and reusable clipboard instructions, and cannot be recovered after leaving the reveal.
- Copy the code repeatedly and confirm every copy produces a fresh screen-reader announcement.
- Confirm replacement invalidates the old challenge atomically and that expiry, replay, five failed attempts, and concurrent consumption all return the stable safe error.

## Account and session behavior

- Test both a new email and an existing account. Confirm existing sessions remain active and both issuance and consumption create user-visible notifications.
- Confirm a new identity stores `admin_assisted` provenance and cannot trigger exact-Gmail Google auto-linking.
- Complete an ordinary mailbox OTP later; confirm provenance upgrades to `mailbox_verified` and normal linking semantics resume.
- Confirm the assisted session uses the Host-only cookie, expires server-side within one hour, shows the persistent warning, and records the non-PII assistant alias.
- Using an existing target account with both administrative capabilities, confirm the initial assisted OTP completion response, `/api/me`, and profile/onboarding responses all return an empty `capabilities` array and every admin API remains denied.
- Confirm onboarding/profile, recipes, pantry, shopping, recommendations, reports/appeals, notifications, import drafts, and locale changes work for the in-person test.
- Confirm the centralized default-deny write policy blocks logout-all/session management, email linking, Passkey registration/revocation, AI/import token creation or revocation, administrative claims/reauth, and all administrative writes. Confirm an unknown future write route is denied until classified.
- Confirm Passkey registration requires `mailbox_verified` provenance and rejects an identity whose only provenance is `admin_assisted`.
- Register a Passkey on the user's own device, sign out, and confirm a later Passkey login receives a normal unrestricted session.

## Audit and privacy

- Verify issuance and consumption audit rows contain only event type, assignment ID/alias, fixed reason, keyed target digest, account-existence bit, note-presence bit, and time.
- Confirm audit and notifications contain no plaintext email, optional note, OTP, cookie, Passkey material, recipe body, or pantry body.
- Confirm the account-assistant UI never displays nickname, recipes, pantry, sessions, Passkeys, or other private target data.

## Rate limits and recovery

- Verify 5/hour per active assistant assignment, 3/24 hours per normalized target, client-address partition, bounded global issuance, and `Retry-After`.
- Verify backup and restore preserve assignments, provenance, notifications, audit, and active challenge/session invalidation semantics.
- Verify host revocation remains available if the web UI or Passkey ceremony is unavailable.

## Accessibility and localization

- Complete keyboard-only, NVDA or VoiceOver, TalkBack, 200% zoom, 320 CSS px reflow, forced-colors, and mobile touch checks.
- Confirm every control has at least a 44 px target, focus remains visible, the confirmation dialog traps/restores focus, and errors/status changes are announced.
- Confirm English and Simplified Chinese cover claim, step-up, warning, new/existing status, one-time reveal, copy, login, restricted-session banner, notifications, errors, and expiry.
