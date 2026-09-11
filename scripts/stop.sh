#!/bin/zsh
set -euo pipefail
flow_pid="$(lsof -nP -t -iTCP:"${FLOW_INSIGHT_PORT:-17901}" -sTCP:LISTEN 2>/dev/null || true)"
[[ -n "$flow_pid" ]] || { echo '服务未运行'; exit 0; }
flow_comm="$(ps -p "$flow_pid" -o comm=)"
case "$flow_comm" in
  */flow-insight) kill -TERM "$flow_pid" ;;
  *) echo '端口不是 Flow Insight，未停止它'; exit 1 ;;
esac
for attempt in {1..60}; do
  if ! kill -0 "$flow_pid" 2>/dev/null; then echo '已停止后台，摄像头已释放'; exit 0; fi
  sleep 0.2
done
echo '退出中'; exit 1
