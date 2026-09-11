#!/bin/zsh
# Build a separate signed release without replacing the local development app.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin ]] || { echo '仅支持 macOS'; exit 1; }
flow_mode="${1:-build}"
if [[ "$flow_mode" == notarize ]]; then
  [[ $# == 2 && -f "$2" && "$2" == *.dmg ]] || { echo '用法：release-macos.sh notarize /绝对路径/安装包.dmg'; exit 1; }
  flow_dmg="$2"
  xcrun notarytool submit "$flow_dmg" --keychain-profile "${FLOW_INSIGHT_NOTARY_PROFILE:-flow-insight-notary}" --wait
  xcrun stapler staple "$flow_dmg"
  xcrun stapler validate "$flow_dmg"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$flow_dmg"
  shasum -a 256 "$flow_dmg" > "$flow_dmg.sha256"
  echo "公证和系统检查通过：$flow_dmg"
  exit 0
fi
[[ "$flow_mode" == build ]] || { echo '支持 build 或 notarize'; exit 1; }
: "${FLOW_INSIGHT_SIGN_IDENTITY:?请设置 Developer ID Application 证书名称或指纹}"
flow_identity="$FLOW_INSIGHT_SIGN_IDENTITY"
security find-identity -v -p codesigning | /usr/bin/grep -F "$flow_identity" | /usr/bin/grep -q 'Developer ID Application:' || { echo '未找到指定的有效 Developer ID Application 身份'; exit 1; }
flow_arch="$(uname -m)"
flow_target=aarch64-apple-darwin
[[ "$flow_arch" != x86_64 ]] || flow_target=x86_64-apple-darwin
MACOSX_DEPLOYMENT_TARGET=14.0 cargo build --release --locked --target "$flow_target"
mkdir -p target/releases
flow_stage="$(mktemp -d "$PWD/target/releases/release.XXXXXX")"
flow_bundle="$flow_stage/image/Flow Insight.app"
mkdir -p "$flow_bundle/Contents/MacOS" "$flow_bundle/Contents/Resources/web"
cp src/web/* "$flow_bundle/Contents/Resources/web/"
cp src/native/Info.plist "$flow_bundle/Contents/Info.plist"
./scripts/build-icon.sh "$flow_bundle/Contents/Resources/FlowInsight.icns"
cp "target/$flow_target/release/flow-insight" "$flow_bundle/Contents/MacOS/flow-insight"
xcrun swiftc -parse-as-library -swift-version 5 -O -target "$flow_arch-apple-macosx14.0" src/native/CaptureHelper.swift src/native/PresenceDetector.swift src/native/DesktopUI.swift -o "$flow_bundle/Contents/MacOS/capture-helper"
codesign --force --sign "$flow_identity" --options runtime --timestamp --entitlements src/native/release.entitlements --identifier com.mewamew.flow-insight.capture-helper "$flow_bundle/Contents/MacOS/capture-helper"
codesign --force --sign "$flow_identity" --options runtime --timestamp --entitlements src/native/release.entitlements --identifier com.mewamew.flow-insight "$flow_bundle"
codesign --verify --deep --strict --verbose=2 "$flow_bundle"
ln -s /Applications "$flow_stage/image/Applications"
flow_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$flow_bundle/Contents/Info.plist")"
flow_dmg="$flow_stage/Flow-Insight-$flow_version-$flow_arch.dmg"
hdiutil create -volname 'Flow Insight' -srcfolder "$flow_stage/image" -ov -format UDZO "$flow_dmg"
codesign --sign "$flow_identity" --timestamp "$flow_dmg"
codesign --verify --strict "$flow_dmg"
echo "已签名但尚未公证，暂不对外发布：$flow_dmg"
echo '下一步：./scripts/release-macos.sh notarize <上述 DMG 的绝对路径>'
