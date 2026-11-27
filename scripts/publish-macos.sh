#!/bin/bash
# Hanmadi public release, the same way Moolle ships: bump the version → arm64 build signed with
# Developer ID and notarized → DMG + auto-update archive → GitHub release on yahoai/hanmadi →
# commit and push the version.
#
# Usage:
#   scripts/publish-macos.sh <patch|minor|major|X.Y.Z> [--notes-file FILE]
#
#   - X.Y.Z equal to the current version rebuilds that version without bumping (retry after a failure).
#   - Without --notes-file the notes are the feat/fix commits since the last release.
#   - Apple Silicon only. Cross-builds fine on an Intel Mac (rustup target add aarch64-apple-darwin).
#   - Run scripts/build-engines.sh first: the engines in build/engines/ go into the app.
#     Models are not bundled; the app downloads them on first run.
#
# Needs (all non-interactive):
#   - Developer ID Application certificate (two share the name, so it is pinned by SHA-1)
#   - App Store Connect API key ~/.appstoreconnect/private_keys/AuthKey_<id>.p8 (notarization)
#   - Updater signing key ~/.tauri/hanmadi-updater.key + keychain item hanmadi-updater-key-password
#     (lose either and installed copies can never update again: back them up)
#   - Tauri CLI (cargo install tauri-cli), gh logged in with repo access, yahoai/hanmadi public
set -euo pipefail

# non-login shells (Moolle jobs, agents) lack cargo/homebrew on PATH
export PATH="$HOME/.cargo/bin:$PATH:/usr/local/bin:/opt/homebrew/bin"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAURI_DIR="$ROOT/app/src-tauri"
CONF="$TAURI_DIR/tauri.conf.json"
CARGO_TOML="$TAURI_DIR/Cargo.toml"
ENGINES="$ROOT/build/engines"
ENTITLEMENTS="$TAURI_DIR/Entitlements.plist"

RELEASE_REPO="yahoai/hanmadi"
APP_NAME="Hanmadi"
BIN_NAME="hanmadi"
TRIPLE="aarch64-apple-darwin"

SIGN_IDENTITY="${APPLE_SIGNING_IDENTITY:-9C7AFA5ABFA8A4B4A224C589838FD7A5E78706C5}"
ASC_KEY_ID="${APPLE_API_KEY:-Z994W3ZA2G}"
ASC_ISSUER_ID="${APPLE_API_ISSUER:-69a6de79-4f28-47e3-e053-5b8c7c11a4d1}"
ASC_KEY_PATH="${APPLE_API_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_${ASC_KEY_ID}.p8}"
UPDATER_KEY="${HANMADI_UPDATER_KEY:-$HOME/.tauri/hanmadi-updater.key}"

log() { echo "[publish] $*"; }
die() { echo "[publish] ERROR: $*" >&2; exit 1; }

# ── args ─────────────────────────────────────────────────────────────
BUMP="${1:-}"
[ -n "$BUMP" ] || die "usage: publish-macos.sh <patch|minor|major|X.Y.Z> [--notes-file FILE]"
shift
NOTES_FILE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --notes-file) NOTES_FILE="$2"; shift 2 ;;
    *) die "unknown arg: $1" ;;
  esac
done
[ -z "$NOTES_FILE" ] || [ -f "$NOTES_FILE" ] || die "notes file not found: $NOTES_FILE"

# ── preflight ────────────────────────────────────────────────────────
log "preflight"
security find-identity -v -p codesigning | grep -qF "$SIGN_IDENTITY" || die "signing identity not in keychain: $SIGN_IDENTITY"
[ -f "$ASC_KEY_PATH" ] || die "App Store Connect key missing: $ASC_KEY_PATH"
[ -f "$UPDATER_KEY" ] || die "updater key missing: $UPDATER_KEY"
UPDATER_KEY_PASSWORD="$(security find-generic-password -a "$USER" -s hanmadi-updater-key-password -w 2>/dev/null)" \
  || die "keychain item hanmadi-updater-key-password missing"
gh auth status >/dev/null 2>&1 || die "gh not logged in"
[ "$(gh repo view "$RELEASE_REPO" --json visibility -q .visibility)" = PUBLIC ] \
  || die "$RELEASE_REPO is private: the app can't download updates from it (gh repo edit $RELEASE_REPO --visibility public --accept-visibility-change-consequences)"
rustup target list --installed | grep -qx "$TRIPLE" || die "rust target missing: rustup target add $TRIPLE"
cargo tauri --version >/dev/null 2>&1 || die "Tauri CLI missing: cargo install tauri-cli"
for e in whisper-cli llama-server sherpa-onnx-vad sherpa-onnx-offline-speaker-diarization ffmpeg ffprobe; do
  [ -f "$ENGINES/$e" ] || die "engine missing: $ENGINES/$e (run scripts/build-engines.sh)"
  [ "$(lipo -archs "$ENGINES/$e")" = arm64 ] || die "$e is not arm64"
done
[ -z "$(git -C "$ROOT" status --porcelain -- app scripts)" ] || die "uncommitted changes under app/ or scripts/"

CURRENT="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["version"])' "$CONF")"
VERSION="$(python3 - "$CURRENT" "$BUMP" <<'EOF'
import re, sys
cur, bump = sys.argv[1:]
if re.fullmatch(r"\d+\.\d+\.\d+", bump):
    print(bump); sys.exit()
a, b, c = map(int, cur.split("."))
nxt = {"major": (a + 1, 0, 0), "minor": (a, b + 1, 0), "patch": (a, b, c + 1)}.get(bump)
if not nxt:
    sys.exit("bad bump: " + bump)
print(".".join(map(str, nxt)))
EOF
)"
TAG="v$VERSION"
if [ "$VERSION" != "$CURRENT" ] && gh release view "$TAG" -R "$RELEASE_REPO" >/dev/null 2>&1; then
  die "$TAG already released in $RELEASE_REPO"
fi
log "version $CURRENT → $VERSION"

# ── 1. version bump ──────────────────────────────────────────────────
if [ "$VERSION" != "$CURRENT" ]; then
  python3 - "$VERSION" "$CONF" "$CARGO_TOML" <<'EOF'
import re, sys
ver, *files = sys.argv[1:]
for f in files:
    src = open(f).read()
    out = (re.sub(r'^version = "[^"]+"', f'version = "{ver}"', src, count=1, flags=re.M) if f.endswith(".toml")
           else re.sub(r'"version": "[^"]+"', f'"version": "{ver}"', src, count=1))
    if out == src:
        sys.exit("version not found in " + f)
    open(f, "w").write(out)
EOF
fi

OUT_DIR="$TAURI_DIR/target/publish/$TAG"
APP="$TAURI_DIR/target/$TRIPLE/release/bundle/macos/$APP_NAME.app"
DMG="${APP_NAME}_${VERSION}_aarch64.dmg"
UPDATE="${APP_NAME}_${VERSION}_aarch64.app.tar.gz"
mkdir -p "$OUT_DIR"
export APPLE_SIGNING_IDENTITY="$SIGN_IDENTITY" APPLE_API_KEY="$ASC_KEY_ID" \
  APPLE_API_ISSUER="$ASC_ISSUER_ID" APPLE_API_KEY_PATH="$ASC_KEY_PATH"

# stapler fetches the ticket from Apple's CloudKit; a dropped connection (Error 68, -1005) is worth a retry
staple() {
  local attempt
  for attempt in 1 2 3 4 5; do
    xcrun stapler staple "$1" && return 0
    log "  staple failed (attempt $attempt/5), retrying in 20s"
    sleep 20
  done
  die "stapler failed 5 times: $1"
}

if [ -f "$OUT_DIR/$DMG" ] && [ -f "$OUT_DIR/$UPDATE.sig" ]; then
  log "already built for $VERSION, skipping the build"
else
  log "(1/4) signing engines"
  for f in "$ENGINES"/*; do
    if [ "$(basename "$f")" = ffmpeg ]; then
      # ffmpeg records from the microphone: under the hardened runtime it needs audio-input itself
      codesign --force --options runtime --timestamp --entitlements "$ENTITLEMENTS" --sign "$SIGN_IDENTITY" "$f"
    else
      codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$f"
    fi
  done

  log "(2/4) tauri build (signs + notarizes the app)"
  cd "$TAURI_DIR"
  cargo tauri build --target "$TRIPLE" --bundles app --config "$TAURI_DIR/tauri.bundle.conf.json"

  log "(3/4) verify + staple app"
  [ "$(lipo -archs "$APP/Contents/MacOS/$BIN_NAME")" = arm64 ] || die "app binary is not arm64"
  for e in "$ENGINES"/*; do
    b="$APP/Contents/Resources/engines/$(basename "$e")"
    [ -x "$b" ] || die "engine not bundled or not executable: $b"
    codesign --verify --strict "$b"
  done
  codesign -d --entitlements - "$APP/Contents/Resources/engines/ffmpeg" 2>/dev/null | grep -q audio-input \
    || die "bundled ffmpeg lacks the audio-input entitlement"
  codesign --verify --deep --strict "$APP"
  staple "$APP"

  log "(4/4) dmg + updater archive"
  STAGING="$(mktemp -d)"
  ditto "$APP" "$STAGING/$APP_NAME.app"
  ln -s /Applications "$STAGING/Applications"
  # hdiutil directly: Tauri's dmg bundler drives Finder via AppleScript and hangs when nobody is at the screen
  hdiutil create -volname "$APP_NAME" -srcfolder "$STAGING" -ov -format UDZO "$OUT_DIR/$DMG"
  rm -rf "$STAGING"
  codesign --force --timestamp --sign "$SIGN_IDENTITY" "$OUT_DIR/$DMG"
  xcrun notarytool submit "$OUT_DIR/$DMG" --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" --wait
  staple "$OUT_DIR/$DMG"

  # auto-update: the notarized, stapled .app as is, tar.gz + minisign signature
  COPYFILE_DISABLE=1 tar -czf "$OUT_DIR/$UPDATE" -C "$(dirname "$APP")" "$APP_NAME.app"
  cargo tauri signer sign -f "$UPDATER_KEY" -p "$UPDATER_KEY_PASSWORD" "$OUT_DIR/$UPDATE" >/dev/null
  [ -f "$OUT_DIR/$UPDATE.sig" ] || die "updater signature not created"
fi

# ── notes + latest.json ──────────────────────────────────────────────
if [ -z "$NOTES_FILE" ]; then
  NOTES_FILE="$OUT_DIR/notes.md"
  PREV_COMMIT="$(git -C "$ROOT" log --format=%h --grep='^chore: Release v' -1 || true)"
  RANGE="${PREV_COMMIT:+$PREV_COMMIT..}HEAD"
  {
    echo "## What's new"
    echo
    git -C "$ROOT" log --format='- %s' "$RANGE" | grep -E '^- (feat|fix)' | head -30 || echo "- Maintenance release"
  } > "$NOTES_FILE"
fi

python3 - "$VERSION" "$NOTES_FILE" "$OUT_DIR/$UPDATE" "https://github.com/$RELEASE_REPO/releases/download/$TAG/$UPDATE" "$OUT_DIR/latest.json" <<'EOF'
import json, sys, datetime
ver, notes, archive, url, out = sys.argv[1:]
manifest = {
    "version": ver,
    "notes": open(notes).read().strip(),
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {"darwin-aarch64": {"signature": open(archive + ".sig").read().strip(), "url": url}},
}
open(out, "w").write(json.dumps(manifest, indent=2) + "\n")
EOF

RELEASE_NOTES="$OUT_DIR/release-notes.md"
{
  cat "$NOTES_FILE"
  echo
  echo "## Download"
  echo
  echo "| Platform | File | SHA-256 |"
  echo "|---|---|---|"
  echo "| macOS (Apple Silicon) | \`$DMG\` | \`$(shasum -a 256 "$OUT_DIR/$DMG" | cut -d' ' -f1)\` |"
  echo
  echo "Signed with Developer ID and notarized by Apple. Installed apps update automatically."
  echo "The speech and translation models (about 3 GB) download on first launch."
} > "$RELEASE_NOTES"

# ── publish + commit version ─────────────────────────────────────────
log "GitHub release $TAG"
ASSETS=("$OUT_DIR/$DMG" "$OUT_DIR/$UPDATE" "$OUT_DIR/latest.json")
if gh release view "$TAG" -R "$RELEASE_REPO" >/dev/null 2>&1; then
  gh release upload "$TAG" -R "$RELEASE_REPO" --clobber "${ASSETS[@]}"
  gh release edit "$TAG" -R "$RELEASE_REPO" --notes-file "$RELEASE_NOTES"
else
  gh release create "$TAG" -R "$RELEASE_REPO" --latest \
    --title "$APP_NAME $VERSION" --notes-file "$RELEASE_NOTES" "${ASSETS[@]}"
fi

cd "$ROOT"
git add "$CONF" "$CARGO_TOML" "$TAURI_DIR/Cargo.lock"
if ! git diff --cached --quiet; then
  git commit -q -m "chore: Release v$VERSION"
  git push -q
fi

log "DONE → https://github.com/$RELEASE_REPO/releases/tag/$TAG"
