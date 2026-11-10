#!/usr/bin/env python3
"""Parse kanripo/KR2e0003 (四部叢刊本《戰國策校注》) into 注文 blocks + 正文.

The scan prints large-type 正文 with small-type interlineal 注; the digitisation
flattens every 注 into parenthesised text, closes each printed line with ¶,
marks each 葉 (folio) with <pb:KR2e0003_SBCK_XXX-Ya>, and keeps the original
two-column fold inside a block as /.

Measured on the 11 upstream files (see stats in _build_stats.json):
  * every 葉 is paren-balanced on its own — the transcriber closes a 注 at the
    line/葉 end and re-opens it on the next line, so a logical 注 is a RUN of
    adjacent parenthesised blocks separated by nothing but whitespace/葉码;
  * therefore a line-based parser sees 10,365 short blocks where there are
    fewer logical notes, and it can split a marker in half ("(…補)¶(曰…)").

Hence the two rules from docs/PLAN.md 第七节:
  1. glue the 葉 back together FIRST (page breaks are not content), and join a
     parenthesised block to the next one when only whitespace / a 葉码 lies
     between them — that is what "跨页拼接" means here, and the number of joins
     that straddle a 葉码 is reported separately;
  2. never use paren balance as the validator (it only holds per-葉 by
     construction, so "balanced" proves nothing).
"""
from __future__ import annotations

import re
import sys
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CACHE = ROOT / "_cache" / "kanripo"
BASE_URL = "https://raw.githubusercontent.com/kanripo/KR2e0003/master/"
FILES = [f"KR2e0003_{i:03d}.txt" for i in range(11)]
UA = {"User-Agent": "Mozilla/5.0 (compatible; zhanguoce-m1b-builder/1.0)"}

# 第二見證本（WYG = 四庫全書文淵閣本）。同一部《戰國策校注》、同樣的
# 大口正文 + 括注版式，故共用一套解析；差別全在卷首噪聲：
#   * 000 檔是提要／劉向序／曾鞏序／鮑彪序／吳師道序／目錄／校正凡例，正文自 001 起；
#   * 每卷卷首三行「欽定四庫全書」「戰國䇿校注卷第N …宋 鮑彪 原注」「…元 吳師道補正」
#     是版式題名，解析前必須剔除，否則會混進正文、干擾章節定位。
WYG_BASE_URL = "https://raw.githubusercontent.com/kanripo/KR2e0005/master/"
WYG_FILES = [f"KR2e0005_{i:03d}.txt" for i in range(11)]
WYG_SKIP = {0}
WYG_NOISE_LINE_RE = re.compile(
    r"^(?:欽定四庫全書.*|戰國[䇿策]校注卷[一二三四五六七八九十]+.*|元　吳師道補正)$")

PB_RE = re.compile(r"<pb:[^>]*>")
ENTITY_RE = re.compile(r"&[A-Za-z0-9#]+;")
PAGE_SENTINEL = "\x01"
GAP_OK = set(" \t\n\u00a0\u3000" + PAGE_SENTINEL)
LAYOUT = {ord(c): None for c in "/\u3000 \t\u00a0" + PAGE_SENTINEL}
PUNCT = {ord(c): None for c in
         "　 ·—‘’“”…、。《》「」『』【】！（），：；？［］～｜－○〇"}


def ensure_cache(files=None, base_url=None) -> None:
    CACHE.mkdir(parents=True, exist_ok=True)
    for name in (files or FILES):
        path = CACHE / name
        if path.is_file() and path.stat().st_size > 0:
            continue
        req = urllib.request.Request((base_url or BASE_URL) + name, headers=UA)
        with urllib.request.urlopen(req, timeout=60) as resp:
            path.write_bytes(resp.read())


def _text(files=None, skip=(), drop_noise=False) -> str:
    """Files glued; page anchors -> sentinel, ¶ (printed line end) -> newline.

    drop_noise removes the WYG 卷首 題名 lines *before* gluing, so neither 正文
    positions nor note blocks ever see 欽定四庫全書 / 卷N headings.
    """
    chunks = []
    for i, name in enumerate(files or FILES):
        if i in skip:
            continue
        raw = (CACHE / name).read_text(encoding="utf-8")
        lines = [ln for ln in raw.split("\n") if not ln.startswith("#")]
        if drop_noise:
            lines = [ln for ln in lines
                     if not WYG_NOISE_LINE_RE.match(ln.rstrip("¶").strip())]
        chunks.append("\n".join(lines))
    return PB_RE.sub(PAGE_SENTINEL, "\n".join(chunks)).replace("¶", "\n")


def build() -> dict:
    """Parse the SBCK witness (KR2e0003)."""
    return _parse(_text())


def build_wyg() -> dict:
    """Parse the WYG witness (KR2e0005, 四庫全書文淵閣本)."""
    return _parse(_text(WYG_FILES, WYG_SKIP, True))


def _parse(text: str) -> dict:
    """-> {text, body, notes, stats}.

    text   全書，葉已拼接、¶ 已去；正文与注文混排（注文带括号）
    body   text 去掉注文后的正文（保留下标可用 bodyPos 反查）
    notes  [{start, end, raw, blocks, crossPage, bodyPos, drop}]
           start/end 为合并后注块的 text 下标；bodyPos = 注块之前已出现的正文字数
    """
    n = len(text)

    raw_blocks = []                      # (open, close_inclusive)
    i = 0
    unterminated = 0
    while i < n:
        if text[i] != "(":
            i += 1
            continue
        j = text.find(")", i + 1)
        if j < 0:
            unterminated += 1
            break
        raw_blocks.append((i, j))
        i = j + 1

    # glue a block to the next one when only whitespace / 葉码 separates them
    groups = []                          # list of lists of (open, close)
    joins = cross_page_joins = 0
    for block in raw_blocks:
        if groups and all(c in GAP_OK for c in text[groups[-1][-1][1] + 1:block[0]]):
            gap = text[groups[-1][-1][1] + 1:block[0]]
            groups[-1].append(block)
            joins += 1
            if PAGE_SENTINEL in gap:
                cross_page_joins += 1
        else:
            groups.append([block])

    notes = []
    body_count = 0
    cursor = 0
    for grp in groups:
        start = grp[0][0]
        end = grp[-1][1] + 1
        body_count += sum(1 for k in range(cursor, start) if text[k] != PAGE_SENTINEL)
        raw = "".join(text[a + 1:b] for a, b in grp).translate(LAYOUT)
        note = {"start": start, "end": end, "raw": raw,
                "blocks": len(grp), "bodyPos": body_count,
                "crossPage": any(PAGE_SENTINEL in text[grp[k - 1][1] + 1:grp[k][0]]
                                 for k in range(1, len(grp)))}
        if ENTITY_RE.search(raw):
            note["drop"] = "entity"
        elif len(raw) <= 1:
            note["drop"] = "one-char"
        notes.append(note)
        cursor = end

    # body = text with every note span removed (葉码 sentinel also removed)
    parts = []
    cursor = 0
    for note in notes:
        parts.append(text[cursor:note["start"]])
        cursor = note["end"]
    parts.append(text[cursor:])
    body = "".join(parts).replace(PAGE_SENTINEL, "")

    pages = text.split(PAGE_SENTINEL)
    stats = {
        "files": len(FILES), "chars": n, "rawBlocks": len(raw_blocks),
        "pages": len(pages),
        "pagesUnbalanced": sum(1 for pg in pages if pg.count("(") != pg.count(")")),
        "notes": len(notes), "joinedBlocks": joins,
        "joinedAcrossPage": cross_page_joins,
        "notesDroppedEntity": sum(1 for x in notes if x.get("drop") == "entity"),
        "notesDroppedOneChar": sum(1 for x in notes if x.get("drop") == "one-char"),
        "notesCrossPage": sum(1 for x in notes if x["crossPage"]),
        "unterminated": unterminated,
        "bodyChars": len(body), "noteChars": sum(len(x["raw"]) for x in notes),
        "markers": {m: sum(x["raw"].count(m) for x in notes)
                    for m in ("補曰", "𥙷曰", "正曰", "彪謂", "彪曰")},
    }
    return {"text": text, "body": body, "notes": notes, "stats": stats}


def body_plain(body: str):
    """-> (plain, orig_index_of_each_plain_char) for alignment."""
    kept, idx = [], []
    for i, ch in enumerate(body):
        if ord(ch) in PUNCT or ch.isspace():
            continue
        kept.append(ch)
        idx.append(i)
    return "".join(kept), idx


if __name__ == "__main__":
    import json
    print(json.dumps(build()["stats"], ensure_ascii=False, indent=2))
