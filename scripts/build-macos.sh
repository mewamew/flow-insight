#!/bin/zsh
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin ]] || { echo '仅支持 macOS 14 及以上'; exit 1; }
flow_identity="$(./scripts/setup-signing.sh)"
cargo build --locked
flow_bundle="$PWD/target/Flow Insight.app"
flow_stage="$(mktemp -d "$PWD/target/macos-build.XXXXXX")"
flow_staged_bundle="$flow_stage/Flow Insight.app"
flow_cleanup() {
  if [[ -d "$flow_stage/previous.app" && ! -d "$flow_bundle" ]]; then
    # Keep the previous app if even rollback fails; never delete the last copy.
    mv "$flow_stage/previous.app" "$flow_bundle" || return
  fi
  rm -rf -- "$flow_stage"
}
trap flow_cleanup EXIT
mkdir -p "$flow_staged_bundle/Contents/MacOS" "$flow_staged_bundle/Contents/Resources/web"
cp src/web/* "$flow_staged_bundle/Contents/Resources/web/"
xcrun swiftc -parse-as-library -swift-version 5 -O -target "$(uname -m)-apple-macosx14.0" src/native/CaptureHelper.swift src/native/PresenceDetector.swift src/native/DesktopUI.swift -o "$flow_staged_bundle/Contents/MacOS/capture-helper"
cp target/debug/flow-insight "$flow_staged_bundle/Contents/MacOS/flow-insight"
cp src/native/Info.plist "$flow_staged_bundle/Contents/Info.plist"
./scripts/build-icon.sh "$flow_staged_bundle/Contents/Resources/FlowInsight.icns"
codesign --force --sign "$flow_identity" --timestamp=none \
  --identifier com.mewamew.flow-insight.capture-helper "$flow_staged_bundle/Contents/MacOS/capture-helper"
codesign --force --sign "$flow_identity" --timestamp=none \
  --identifier com.mewamew.flow-insight "$flow_staged_bundle"
codesign --verify --deep --strict "$flow_staged_bundle"
# Do not partially overwrite the existing app if compilation or signing fails.
if [[ -d "$flow_bundle" ]]; then
  if lsof -t "$flow_bundle/Contents/MacOS/flow-insight" >/dev/null 2>&1; then
    echo '新应用已验证，但旧应用仍在运行；请先 ./scripts/stop.sh，再重新构建。' >&2
    exit 1
  fi
  mv "$flow_bundle" "$flow_stage/previous.app"
fi
if ! mv "$flow_staged_bundle" "$flow_bundle"; then
  [[ ! -d "$flow_stage/previous.app" ]] || mv "$flow_stage/previous.app" "$flow_bundle"
  exit 1
fi
echo "已构建：$flow_bundle"
