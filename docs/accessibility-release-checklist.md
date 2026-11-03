# Accessibility and matching release checklist

This checklist is a release gate, not a claim that every assistive-technology combination has already passed. Record the browser, operating system, device, date, and result for each manual run before publishing a release that includes the pantry and ingredient-matching UI.

## Automated coverage

- [x] ASCII commas/semicolons/newlines and full-width comma, Chinese enumeration comma, and full-width semicolon split consistently.
- [x] NFKC, Unicode case lowering, whitespace collapse, canonical deduplication, and display-name retention have backend and frontend coverage.
- [x] A high-risk similar pair (`baking soda` and `baking powder`) is not silently matched.
- [x] Expired, expires-today, pantry-only, explicit user-local `today`, cross-month, and leap-day cases are covered.
- [x] Combobox Arrow Up/Down, Enter, Escape, live result count/current option, explicit unresolved choice, and undo are covered.
- [x] A past date remains editable and produces a persistent pre-save warning.

The current typo sample is intentionally small: it includes an English transposition and multilingual normalization/splitting, but it is not a representative English-and-Chinese typo corpus. Cook therefore makes **no Top-3 ≥95% claim yet**. Before making that claim, add a reviewed benchmark corpus with expected Top-3 candidates and publish the corpus size, language distribution, and scoring method.

The combobox uses a 150 ms client debounce. The API p95 ≤300 ms target has not been measured against a production-sized catalog, so a representative load benchmark is also a release gate.

## Manual assistive-technology matrix

- [ ] NVDA with Chrome on Windows: landmarks, labels, listbox count/current option, selection, undo, errors, and pantry status.
- [ ] VoiceOver with Safari on macOS/iOS: rotor order, labels, listbox interaction, date warning, and result explanations.
- [ ] TalkBack with Chrome on Android: touch exploration, target size, listbox selection, and bottom-sheet pantry confirmation.
- [ ] Keyboard only: visible focus, logical order, no focus trap, Arrow/Enter/Escape behavior, error-summary focus, and destructive confirmations.
- [ ] Windows High Contrast/forced colors: controls, selected suggestion, status, warning, and focus remain distinguishable.

## Zoom, reflow, and contrast

- [ ] 200% browser text zoom preserves all content and controls.
- [ ] 400% browser zoom or a 320 CSS-pixel viewport has no lost content and no nonessential horizontal scrolling.
- [ ] Body text is at least 16 CSS pixels; key pantry/recommendation status is at least 18 CSS pixels where practical.
- [ ] Text contrast is at least 4.5:1 and non-text controls/focus indicators are at least 3:1, measured with an accessibility tool rather than visual estimation.
- [ ] Touch targets are at least 44 by 44 CSS pixels.

## Date and recommendation review

- [ ] Test the user's local date boundary immediately before and after midnight in at least two materially different time zones.
- [ ] Confirm past dates are allowed with warning, never inferred, never converted to timestamps, and never change pantry status to `out`.
- [ ] Confirm expired and expires-today `have` items are `VERIFY` normally and `BLOCKED` in pantry-only mode.
- [ ] Confirm ambiguous or similar suggestions never alter must-use, excluded, or pantry-only hard matching until the user explicitly selects a canonical identity.
