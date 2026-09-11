#!/usr/bin/env python3
"""Import a qwen.txt model configuration into local settings without printing the key."""
import json, os, re, sys
from pathlib import Path
source = Path(sys.argv[1]).read_text()
url = re.search(r'https://[^\s]+', source)
key = re.search(r'(?im)^\s*key\s*[：:]\s*(\S+)', source)
if not url or not key:
    raise SystemExit('qwen.txt 需包含 HTTPS 地址和 key: 字段')
root = Path(os.environ.get('FLOW_INSIGHT_DATA_DIR', str(Path(__file__).resolve().parents[1] / 'data/runtime')))
root.mkdir(parents=True, exist_ok=True); root.chmod(0o700)
p = root / 'settings.json'
s = json.loads(p.read_text()) if p.exists() else {}
s.update(base_url=url.group(0), model='qwen3.8-flash', api_key=key.group(1))
s.setdefault('interval_seconds',60); s.setdefault('task',''); s.setdefault('report_hour',20)
fd = os.open(p, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
with os.fdopen(fd, 'w') as f: json.dump(s,f,ensure_ascii=False,indent=2)
p.chmod(0o600)
print('已导入 qwen3.8-flash 配置；密钥未输出。')
