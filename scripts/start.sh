#!/bin/zsh
set -euo pipefail
cd "$(dirname "$0")/.."
flow_port="${FLOW_INSIGHT_PORT:-17901}"
flow_url="http://127.0.0.1:$flow_port"
if lsof -nP -iTCP:"$flow_port" -sTCP:LISTEN >/dev/null 2>&1; then
  if curl --fail --silent "$flow_url/api/status" | python3 -c 'import json,sys; assert json.load(sys.stdin).get("name")=="Flow Insight · 心流洞察"' 2>/dev/null; then
    open "$flow_url"; exit 0
  fi
  echo "端口 $flow_port 被其他程序占用" >&2; exit 1
fi
flow_bundle="$PWD/target/Flow Insight.app"
# Reuse the authorized bundle; re-sign only on an explicit rebuild.
if [[ ! -x "$flow_bundle/Contents/MacOS/flow-insight" || ! -x "$flow_bundle/Contents/MacOS/capture-helper" || ! -f "$flow_bundle/Contents/Resources/web/index.html" ]]; then
  ./scripts/build-macos.sh
fi
open -a "$PWD/target/Flow Insight.app" --env "FLOW_INSIGHT_PORT=$flow_port" --env "FLOW_INSIGHT_DATA_DIR=${FLOW_INSIGHT_DATA_DIR:-$PWD/data/runtime}"
for attempt in {1..60}; do
  if curl --fail --silent "$flow_url/api/status" >/dev/null; then open "$flow_url"; echo "已启动 $flow_url"; exit 0; fi
  sleep 0.2
done
echo '后台未启动，请直接运行 target/Flow Insight.app/Contents/MacOS/flow-insight 查看错误' >&2
exit 1
