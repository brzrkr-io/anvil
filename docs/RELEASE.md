# Release: sign, notarize, auto-update

The app config is wired for signed/notarized macOS builds and Tauri auto-update.
Both need secrets that aren't in the repo — supply them as environment variables
(locally or as CI secrets), then run the bundle.

## #94 — Sign + notarize (macOS)

Config already in place:
- `src-tauri/entitlements.plist` — hardened-runtime entitlements (JIT, network client, apple-events).
- `src-tauri/tauri.conf.json` → `bundle.macOS.entitlements` + `minimumSystemVersion`.

Provide these env vars, then run `tauri build` (no `--no-bundle`):

```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export APPLE_ID="you@example.com"
export APPLE_PASSWORD="app-specific-password"   # appleid.apple.com → App-Specific Passwords
export APPLE_TEAM_ID="TEAMID"
./node_modules/.bin/tauri build            # produces a signed + notarized .app/.dmg
```

Tauri reads these automatically: it codesigns with the identity, then submits to
Apple's notary service and staples the ticket. No code change needed — just the cert.

## #95 — Auto-update (Tauri updater)

Requires (a) a signing keypair and (b) a place to host `latest.json` + artifacts
(GitHub Releases works).

1. Generate the update keypair (local, no network):
   ```sh
   ./node_modules/.bin/tauri signer generate -w ~/.anvil/updater.key
   ```
   Keep the private key secret (`TAURI_SIGNING_PRIVATE_KEY`); the printed public
   key goes into `tauri.conf.json` → `plugins.updater.pubkey`.

2. Add the updater plugin + config (one-time wiring):
   - `cargo add tauri-plugin-updater` (in `src-tauri`)
   - `pnpm add @tauri-apps/plugin-updater`
   - `tauri.conf.json`:
     ```json
     "plugins": { "updater": {
       "pubkey": "<public key from step 1>",
       "endpoints": ["https://github.com/<you>/anvil/releases/latest/download/latest.json"]
     } }
     ```
   - capability: add `"updater:default"`.
   - in `lib.rs`: `.plugin(tauri_plugin_updater::Builder::new().build())`.

3. On release, sign artifacts with `TAURI_SIGNING_PRIVATE_KEY` set; `tauri build`
   emits `latest.json`. Upload it + the artifacts to the endpoint.

The app calls `check()` on demand (e.g. a "Check for Updates" command), so a missing
endpoint degrades gracefully — it never blocks startup.
```
