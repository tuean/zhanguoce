#!/usr/bin/env python3
"""Read-only format probe: what do the two sources actually look like?

Prints aggregates only (never full texts) so the findings can go into DATA_REPORT.md.
Also resolves the authoritative file list for source A via its subtree SHA, because
the repo-wide recursive tree is truncated by GitHub.
"""
from __future__ import annotations
import json, re, sys, urllib.parse, urllib.request
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CACHE = ROOT / "_cache"
UA = {"User-Agent": "Mozilla/5.0 (compatible; zhanguoce-m1-builder/1.0)"}
A_PREFIX = "双语数据/战国策/"


def get(url, timeout=40, tries=3):
    last = None
    for _ in range(tries):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=timeout) as r:
                return r.read()
        except Exception as exc:
            last = exc
    raise last


def subtree_sha(repo: str, branch: str, path: str) -> str:
    """Walk one path segment at a time with the contents API to get a tree sha."""
    parent, _, leaf = path.rstrip("/").rpartition("/")
    url = (f"https://api.github.com/repos/{repo}/contents/"
           f"{urllib.parse.quote(parent)}?ref={branch}")
    entries = json.loads(get(url))
    assert isinstance(entries, list), f"expected listing for {parent}"
    for entry in entries:
        if entry["name"] == leaf:
            return entry["sha"]
    raise KeyError(f"{leaf} not found under {parent}")


def authoritative_a() -> list[str]:
    sha = subtree_sha("crazyxiaoqiang/Classical-Modern", "main", "双语数据/战国策")
    url = f"https://api.github.com/repos/crazyxiaoqiang/Classical-Modern/git/trees/{sha}?recursive=1"
    tree = json.loads(get(url))["tree"]
    out = []
    for e in tree:
        if e["type"] == "blob" and e["path"].endswith("/bitext.txt"):
            # path is relative to the 战国策 subtree: <卷>/<章>/bitext.txt
            out.append(A_PREFIX + e["path"])
    return sorted(out)


def main() -> int:
    auth = authoritative_a()
    cached = sorted(str(p.relative_to(CACHE / "a")).replace("\\", "/")
                    for p in (CACHE / "a").rglob("bitext.txt"))
    print(f"[A] authoritative chapter files: {len(auth)}")
    print(f"[A] cached chapter files      : {len(cached)}")
    print(f"[A] missing from cache        : {len(set(auth) - set(cached))}")
    for m in sorted(set(auth) - set(cached))[:20]:
        print("    MISSING", m)
    vols = Counter(p[len(A_PREFIX):].split("/")[0] for p in auth)
    print(f"[A] volumes: {len(vols)} -> " + ", ".join(f"{k}:{v}" for k, v in vols.items()))
    (CACHE / "a_manifest.json").write_text(
        json.dumps({"chapters": auth, "volumes": vols}, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8")

    # ---- format stats ----
    rec_re = re.compile(r"^\s*(古文|现代文)\s*[：:]\s*(.*)$")
    stats = Counter()
    odd_lines: Counter = Counter()
    for path in (CACHE / "a").rglob("bitext.txt"):
        text = path.read_text(encoding="utf-8-sig")
        for block in re.split(r"\n\s*\n", text.strip()):
            if not block.strip():
                continue
            stats["records"] += 1
            lines = [l for l in block.split("\n") if l.strip()]
            kinds = []
            for line in lines:
                m = rec_re.match(line)
                if m:
                    kinds.append(m.group(1))
                else:
                    odd_lines[line[:40]] += 1
            if "古文" in kinds and "现代文" in kinds:
                stats["pairs"] += 1
            elif "古文" in kinds:
                stats["guwen_only"] += 1
            elif "现代文" in kinds:
                stats["modern_only"] += 1
            if len(kinds) != len(set(kinds)):
                stats["dup_key_in_block"] += 1
            if len(lines) > 2:
                stats["blocks_gt2lines"] += 1
    print("[A] record stats:", dict(stats))
    print("[A] non-key lines (top 10):", odd_lines.most_common(10))

    # ---- source B ----
    htm = sorted((CACHE / "b" / "OEBPS" / "Text").glob("*.htm"))
    print(f"[B] cached htm: {len(htm)}")
    span_re = re.compile(r'<span class="style9">(.*?)</span>', re.S)
    title_re = re.compile(r'<p class="style6">(.*?)</p>', re.S)
    tag_re = re.compile(r"<[^>]+>")
    vol_counter: Counter = Counter()
    no_span = no_title = 0
    for p in htm:
        t = p.read_text(encoding="utf-8-sig")
        m = span_re.search(t)
        if m:
            vol_counter[tag_re.sub("", m.group(1)).strip()] += 1
        else:
            no_span += 1
        if not title_re.search(t):
            no_title += 1
    print(f"[B] no volume span: {no_span}, no chapter title: {no_title}")
    print(f"[B] volume labels: {len(vol_counter)}")
    for k, v in vol_counter.items():
        print(f"    {k!r}: {v}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
