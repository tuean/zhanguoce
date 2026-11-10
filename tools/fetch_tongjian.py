#!/usr/bin/env python3
"""Fetch the 戰國 segment of 資治通鑑 into tools/_cache/tongjian/.

raw.githubusercontent.com times out in batches (docs/EVENT_SOURCES.md §5.1), so
this goes through the GitHub contents API, which returns base64. Requests are
serial: the unauthenticated quota is 60/h and seven files cost seven calls.

Idempotent: an already-cached file is never re-fetched, so repeat builds are
fully offline.
"""

from __future__ import annotations

import base64
import json
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = "Ancient-China-Books/zizhitongjian"
REF = "master"
# 周紀一/二/三/四/五、秦紀一/二 —— filenames are NOT in reading order.
FILES = ["5282", "5381", "5382", "5383", "5384", "5385", "5386"]
CACHE = Path(__file__).resolve().parent / "_cache" / "tongjian"
API = "https://api.github.com/repos/{repo}/contents/OEBPS/Text/{name}.html?ref={ref}"


def fetch_one(name: str, attempts: int = 3) -> bytes:
    url = API.format(repo=REPO, name=name, ref=REF)
    last = None
    for attempt in range(1, attempts + 1):
        try:
            request = urllib.request.Request(
                url,
                headers={
                    "Accept": "application/vnd.github+json",
                    "User-Agent": "zgc-tools/1.0",
                },
            )
            with urllib.request.urlopen(request, timeout=20) as response:
                payload = json.loads(response.read())
            if "content" not in payload:
                raise RuntimeError(f"{name}: no content field in API response")
            return base64.b64decode(payload["content"])
        except (urllib.error.URLError, TimeoutError, OSError, RuntimeError) as error:
            last = error
            if attempt < attempts:
                time.sleep(2 * attempt)
    raise SystemExit(f"下載 {name}.html 失敗（{attempts} 次）：{last}")


def main() -> int:
    CACHE.mkdir(parents=True, exist_ok=True)
    fetched = 0
    for name in FILES:
        path = CACHE / f"{name}.html"
        if path.exists() and path.stat().st_size > 0:
            print(f"cache  {name}.html  {path.stat().st_size} B")
            continue
        data = fetch_one(name)
        path.write_bytes(data)
        fetched += 1
        chars = len(data.decode("utf-8"))
        print(f"下載   {name}.html  {len(data)} B / {chars} 字")
        time.sleep(0.5)
    print(f"完成：{len(FILES)} 個文件，{fetched} 次網路請求，緩存於 {CACHE}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
