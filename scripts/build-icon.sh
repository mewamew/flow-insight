#!/bin/zsh
# Convert the approved artwork to macOS icon sizes without changing the source.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $# == 1 ]] || { echo '用法：build-icon.sh /绝对路径/FlowInsight.icns'; exit 1; }
flow_output="$1"
[[ "$flow_output" == /* ]] || { echo '输出必须是绝对路径'; exit 1; }
flow_icon_source="$PWD/src/native/assets/flow-insight-icon.png"
[[ -f "$flow_icon_source" ]] || { echo '缺少已选定的应用图标'; exit 1; }
flow_icon_temp="$(mktemp -d)"
trap 'rm -rf -- "$flow_icon_temp"' EXIT
mkdir -p "$flow_icon_temp/FlowInsight.iconset" "$(dirname "$flow_output")"
for flow_size in 16 32 128 256 512; do
  sips -z "$flow_size" "$flow_size" "$flow_icon_source" --out "$flow_icon_temp/FlowInsight.iconset/icon_${flow_size}x${flow_size}.png" >/dev/null
  flow_double=$((flow_size * 2))
  sips -z "$flow_double" "$flow_double" "$flow_icon_source" --out "$flow_icon_temp/FlowInsight.iconset/icon_${flow_size}x${flow_size}@2x.png" >/dev/null
done
iconutil -c icns "$flow_icon_temp/FlowInsight.iconset" -o "$flow_output"
