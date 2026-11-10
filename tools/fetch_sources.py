#!/usr/bin/env python3
"""Fetch both upstream sources into a local cache.

Source A: crazyxiaoqiang/Classical-Modern  -> 双语数据/战国策/<卷>/<章>/bitext.txt
          (simplified 古文 + 现代文 pairs, one blank-line separated record per pair)
Source B: Ancient-China-Books/zhanguoce    -> OEBPS/Text/NNN.htm (traditional, per-章)

Design: one recursive git-tree call per repo (avoids the 60/h contents API limit),
then raw.githubusercontent.com for every blob. Cache is content-addressed by path;
existing non-empty files are never re-fetched, so re-running is cheap and idempotent.
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CACHE = ROOT / "_cache"
REPOS = {
    "a": ("crazyxiaoqiang/Classical-Modern", "main"),
    "b": ("Ancient-China-Books/zhanguoce", "master"),
}
UA = {"User-Agent": "Mozilla/5.0 (compatible; zhanguoce-m1-builder/1.0)"}


def _get(url: str, timeout: int = 40, tries: int = 3) -> bytes:
    last: Exception | None = None
    for attempt in range(tries):
        try:
            req = urllib.request.Request(url, headers=UA)
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                return resp.read()
        except Exception as exc:  # noqa: BLE001 - network flake is expected
            last = exc
            time.sleep(0.6 * (attempt + 1))
    raise last  # type: ignore[misc]


def tree(repo: str, branch: str) -> list[dict]:
    url = f"https://api.github.com/repos/{repo}/git/trees/{branch}?recursive=1"
    data = json.loads(_get(url))
    if data.get("truncated"):
        print(f"WARN {repo}: tree truncated", file=sys.stderr)
    return [e for e in data["tree"] if e["type"] == "blob"]


def raw_url(repo: str, branch: str, path: str) -> str:
    return (f"https://raw.githubusercontent.com/{repo}/{branch}/"
            + urllib.parse.quote(path))


def fetch_all(repo: str, branch: str, blobs: list[dict], dest: Path,
              workers: int = 8) -> tuple[int, list[str]]:
    dest.mkdir(parents=True, exist_ok=True)
    failed: list[str] = []
    done = 0

    def one(blob: dict) -> bool:
        out = dest / blob["path"]
        if out.exists() and out.stat().st_size == blob["size"] and blob["size"] > 0:
            return True
        out.parent.mkdir(parents=True, exist_ok=True)
        try:
            data = _get(raw_url(repo, branch, blob["path"]))
        except Exception as exc:  # noqa: BLE001
            failed.append(f"{blob['path']} :: {exc}")
            return False
        if len(data) != blob["size"]:
            print(f"WARN size mismatch {blob['path']}: {len(data)} != {blob['size']}",
                  file=sys.stderr)
        out.write_bytes(data)
        return True

    with ThreadPoolExecutor(max_workers=workers) as pool:
        for ok in pool.map(one, blobs):
            done += 1 if ok else 0
    return done, failed


def main() -> int:
    manifest: dict = {"sources": {}, "fetchedAt": None}
    for key, (repo, branch) in REPOS.items():
        blobs = tree(repo, branch)
        if key == "a":
            prefix = "双语数据/战国策/"
            sel = [b for b in blobs if b["path"].startswith(prefix)
                   and b["path"].endswith("/bitext.txt")]
            # keep the full tree too:它是章/卷清单的唯一权威来源
            index = [b for b in blobs if b["path"].startswith(prefix)]
        else:
            sel = [b for b in blobs
                   if b["path"].startswith("OEBPS/Text/") and b["path"].endswith(".htm")]
            index = [b for b in blobs if b["path"] in
                     ("OEBPS/content.opf", "OEBPS/toc.ncx", "index.html")]
        dest = CACHE / key
        ok, failed = fetch_all(repo, branch, sel + [b for b in index if b not in sel], dest)
        total_bytes = sum(p.stat().st_size for p in dest.rglob("*") if p.is_file())
        print(f"[{key}] {repo}@{branch}: {len(sel)} payload blobs, "
              f"{ok} ok, {len(failed)} failed, {total_bytes} bytes on disk")
        for f in failed[:10]:
            print("   FAIL", f)
        manifest["sources"][key] = {
            "repo": repo, "branch": branch,
            "payloadFiles": len(sel), "ok": ok, "failed": failed[:50],
            "bytes": total_bytes,
        }
    manifest["fetchedAt"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    (CACHE / "manifest.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print("manifest ->", CACHE / "manifest.json")
    return 0 if all(not s["failed"] for s in manifest["sources"].values()) else 1


if __name__ == "__main__":
    raise SystemExit(main())
