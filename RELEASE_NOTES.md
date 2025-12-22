### Changes
- Overhauled credential scheduling with the new `CredentialManager`: per-model queues, improved cooldown reclamation, batch invalidation/refresh handling, and unsupported-model blacklisting.
- Added a rate-limited OAuth refresh job pipeline (`OAUTH_TPS`) with retry-aware `loadCodeAssist`/`onboardUser` operations for more reliable token refresh and onboarding.
- Improved Google OAuth onboarding by resolving effective Code Assist tiers, honoring default tiers, and failing fast on ineligible accounts.
- Hardened Gemini CLI upstream calls with configurable retry ceilings (`GEMINI_RETRY_MAX_TIMES`), jittered backoff, and richer credential selection logging.
- Made model handling configuration-driven (`MODEL_LIST`) for request validation and model list responses, returning clearer errors for unsupported models.
