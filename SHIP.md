# Shipping Anvil

How to cut a **signed + notarized** macOS release with working **auto-update**.
The repo is already wired (`release.yml`, updater plugin, `createUpdaterArtifacts: true`,
in-app install flow). What's left are the credential steps only you can do — Anvil
never enters your Apple or signing credentials for you.

## One-time setup

### 1. Updater signing keypair

The updater verifies every downloaded update against a public key baked into
`src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). You need the matching
**private** key.

```sh
pnpm tauri signer generate -w ~/.anvil-updater.key
```

- Copy the printed **public key** into `tauri.conf.json` → `plugins.updater.pubkey`
  (only if it differs from what's committed — changing it means installs signed
  with the old key can't auto-update).
- Keep the private key file secret. Never commit it.

### 2. Apple Developer credentials (for notarization)

Requires the paid Apple Developer Program. Gather:

- **Developer ID Application** certificate, exported as a base64 `.p12` + its password.
- An **app-specific password** for your Apple ID (appleid.apple.com → Sign-In & Security).
- Your **Team ID** (10 chars, from developer.apple.com → Membership).

### 3. GitHub repo secrets

Settings → Secrets and variables → Actions → **New repository secret**. Names
must match `release.yml` exactly:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | contents of `~/.anvil-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | its password (empty if you set none) |
| `APPLE_CERTIFICATE` | base64 of the Developer ID `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID` | your Apple ID email |
| `APPLE_PASSWORD` | the app-specific password |
| `APPLE_TEAM_ID` | your Team ID |

Without the Apple secrets the build still succeeds but ships **unsigned**.
Without the `TAURI_SIGNING_*` secrets it ships without updater artifacts (no
auto-update).

## Cut a release

```sh
# 1. Bump the version in BOTH files (keep them in sync):
#    src-tauri/tauri.conf.json  →  "version"
#    package.json               →  "version"

# 2. Commit, tag, push the tag — release.yml triggers on tags matching v*
git commit -am "release: v0.1.1"
git tag v0.1.1
git push origin main --tags
```

`release.yml` builds on macOS/Linux/Windows, signs + notarizes the macOS build,
emits the updater artifacts (`latest.json` + signed `.app.tar.gz`), and opens a
**draft** GitHub Release. Review it, then **Publish** — the updater endpoint
(`releases/latest/download/latest.json`) only resolves once the release is published.

## Verify auto-update end-to-end

1. Download + install the published **v0.1.1** `.dmg`. Launch it — it should open
   without a Gatekeeper warning (notarized).
2. Cut and publish **v0.1.2** (repeat *Cut a release*).
3. Launch the installed **v0.1.1**. ~8s after launch the quiet check runs
   (`autoCheckUpdate` in `+page.svelte`); when an update exists you get a
   confirm dialog → **Install** downloads the signature-verified update and
   relaunches into v0.1.2.
4. Manual path: ⌘K → **Check for Updates** does the same on demand.
   ⌘K → **Update Channel** switches stable/beta.

## Notes

- The updater repo/endpoint is set in `tauri.conf.json` → `plugins.updater.endpoints`.
  Point it at the repo that hosts your releases.
- First-run onboarding (the stepped tour) shows once, gated by the
  `anvil-onboarded` localStorage key.
