#!/usr/bin/env python3
"""Opt-in multi-image check using saved model settings and synthetic images only."""
from pathlib import Path
import argparse
import base64
import json
import os
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--settings', type=Path, default=root / 'data/runtime/settings.json')
parser.add_argument('--report', type=Path, default=root / 'data/validation/multiscreen-model-validation.json')
args = parser.parse_args()
settings = json.loads(args.settings.read_text())
settings.update(task='验证两张合成图片的读取。图中没有真实工作内容，无法判断投入，应返回 unknown。', reminders=False, report_hour=23)
with socket.socket() as listener:
    listener.bind(('127.0.0.1', 0))
    port = listener.getsockname()[1]
with tempfile.TemporaryDirectory(prefix='multiscreen-model-') as directory:
    config = Path(directory) / 'settings.json'
    config.write_text(json.dumps(settings))
    config.chmod(0o600)
    env = dict(os.environ, FLOW_INSIGHT_PORT=str(port), FLOW_INSIGHT_DATA_DIR=directory)
    process = subprocess.Popen([str(root / 'target/debug/flow-insight')], env=env,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    def api(path, body=None):
        request = urllib.request.Request(f'http://127.0.0.1:{port}' + path,
            headers={'Content-Type':'application/json', 'x-flow-insight-client':'web'},
            data=None if body is None else json.dumps(body).encode())
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.load(response)
    try:
        for _ in range(80):
            if process.poll() is not None:
                raise RuntimeError('隔离服务启动失败')
            try:
                status = api('/api/status')
                break
            except urllib.error.URLError:
                time.sleep(.1)
        else:
            raise RuntimeError('隔离服务未就绪')
        session = api('/api/sessions', {})
        image = 'data:image/png;base64,' + base64.b64encode((root / 'tests/fixtures/synthetic-frame.png').read_bytes()).decode()
        sample_id = str(uuid.uuid4())
        at = int(time.time() * 1000)
        api('/api/captures', {'id':sample_id, 'session_id':session['id'], 'captured_at':at,
            'capture_source':'两张合成屏幕图验证', 'activity':{'foreground_display_id':1},
            'screens':[{'display_id':i, 'display_name':f'合成测试屏幕 {i}', 'captured_at':at, 'image':image} for i in [1,5]]})
        deadline = time.monotonic() + 100
        while time.monotonic() < deadline:
            view = api('/api/flow?date=' + status['today'] + '&mode=live')
            sample = next((s['sample'] for s in view['day']['segments'] if s['sample']['id'] == sample_id), None)
            if sample and sample['state'] == 'done':
                break
            if sample and sample['state'] == 'error':
                raise RuntimeError(sample['error'])
            time.sleep(.5)
        else:
            raise RuntimeError('双图分析未在限定时间内完成')
        assert len(sample['screens']) == 2 and sample['camera'] is None
        api('/api/sessions/' + session['id'] + '/stop', {})
        outcome = {'ok':True, 'model':settings['model'], 'samples':1, 'screen_images':2,
                   'camera_images':0, 'state':sample['state'], 'category':sample['analysis']['category'],
                   'real_screen_or_camera_uploaded':False,
                   'scope':'验证真实模型接受同一请求中的两张合成图，不证明日常工作状态判断准确率'}
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(json.dumps(outcome, ensure_ascii=False, indent=2) + '\n')
        print(json.dumps(outcome, ensure_ascii=False), flush=True)
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
