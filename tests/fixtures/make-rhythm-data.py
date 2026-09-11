#!/usr/bin/env python3
"""Create an isolated, clearly labeled synthetic history for browser verification.

Usage: python3 tests/fixtures/make-rhythm-data.py
Prints a new temporary directory. Never accepts or writes an existing data directory.
"""
import datetime as dt
import json
from pathlib import Path
import sqlite3
import tempfile

root = Path(tempfile.mkdtemp(prefix="flow-insight-rhythm-"))
(root / "captures").mkdir()
(root / "settings.json").write_text(json.dumps({
    "task": "隔离合成测试数据，不代表真实工作记录", "api_key": "", "retention_days": 90,
    "reminders": False,
}, ensure_ascii=False))
db = sqlite3.connect(root / "flow-insight.sqlite3")
db.execute("CREATE TABLE objects(kind TEXT NOT NULL,id TEXT NOT NULL,stamp INTEGER NOT NULL,mode TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(kind,id))")

def put(kind, obj, stamp):
    db.execute("INSERT INTO objects VALUES(?,?,?,?,?)", (kind,obj["id"],stamp,"live",json.dumps(obj,ensure_ascii=False)))

today = dt.date.today()
for offset in range(21):
    date = today - dt.timedelta(days=offset)
    if offset in (3, 5, 11, 15, 19):
        continue
    start = int(dt.datetime.combine(date, dt.time(8)).timestamp() * 1000)
    session_id = f"synthetic-session-{date}"
    # All synthetic sessions are closed. No real recorder or model is used.
    put("session", {"id":session_id,"started_at":start,"ended_at":start+10*3600000,
        "interval_seconds":60,"task":"隔离合成测试","mode":"live"}, start)
    for minute in range(600):
        hour = 8 + minute // 60
        within = minute % 60
        if hour == 12 or (hour == 8 and within < 15) or (hour == 17 and within > 30):
            continue
        category = "work"
        if hour == 8:
            category = "unknown" if within < 30 else "work"
        elif hour == 9 and within in range(22+offset%4, 28+offset%4):
            category = "distracted"
        elif hour == 10 and within in range(35, 42):
            category = "away"
        elif hour == 13 and within < 20+offset%8:
            category = "distracted"
        elif hour == 15:
            category = "distracted" if within % 15 < 5 else "work"
        elif hour == 16 and within > 48:
            category = "unknown"
        t = start + minute * 60000
        sample = {"id":f"synthetic-{t}","session_id":session_id,"captured_at":t,
            "interval_seconds":60,"mode":"live","task":"隔离合成测试数据，不代表真实工作记录",
            "screen":None,"camera":None,"capture_source":"synthetic-validation",
            "state":"done","error":None,"correction":None,"correction_note":"",
            "analysis":{"category":category,"confidence":0.9,"app_name":"合成测试场景",
                "screen_activity":"验证工作节奏与回放联动","camera_state":"未采集",
                "summary":"这是自动生成的隔离测试数据，不代表真实工作情况。","evidence":["合成测试数据"]}}
        put("sample", sample, t)
db.commit()
db.close()
print(root)
