#!/usr/bin/env python3
"""Build data/zhanguoce/events/tongjian.json from the cached 資治通鑑 segments.

Source layout (docs/EVENT_SOURCES.md §1.2):

    <div class="yt" id="y0403"><p class="strong" id="p3">
      <span data-head-description="威烈王" data-head-title="二十三年">二十三年</span>
      <span class="biaozhusup" data-rel="">（戊寅、前四○三年）</span>
      <span class="jiazhu">上距春秋獲麟七十八年…</span></p></div>
    <p class="em2 strong" id="p4">…初命晉大夫魏斯、趙籍、韓虔爲諸侯。…</p>

The div.yt is a *year marker*; the numbered paragraphs that follow it (up to the
next div.yt) are that year's events. Every event paragraph is one event.

Six self-checks run before anything is written; the first failure aborts the
build with a non-zero exit code. Output is deterministic (idempotent).
"""

from __future__ import annotations

import html
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CACHE = ROOT / "tools" / "_cache" / "tongjian"
OUT = ROOT / "data" / "zhanguoce" / "events" / "tongjian.json"

# file → 卷名, in the book's volume order (the file numbers are NOT in order).
JUAN = {
    "5282": "周紀一",
    "5381": "周紀二",
    "5382": "周紀三",
    "5383": "周紀四",
    "5384": "周紀五",
    "5385": "秦紀一",
    "5386": "秦紀二",
}
FILES = list(JUAN)

DIV = re.compile(r'<div class="yt" id="(y\d{4})">')
HEAD_ATTRS = re.compile(
    r'data-head-description="([^"]*)"[^>]*data-head-title="([^"]*)"'
)
BIAOZHU = re.compile(r'<span class="biaozhusup" data-rel="">([^<]*)</span>')
EVENT_P = re.compile(r'<p class="em2 strong" id="(p\d+)">')
YEAR_P = re.compile(r'<p class="[^"]*" id="(p\d+)">')
TAG = re.compile(r'<(/?)([a-zA-Z0-9]+)([^>]*)>')

DIGITS = {
    "○": 0, "〇": 0, "零": 0,
    "一": 1, "二": 2, "三": 3, "四": 4, "五": 5,
    "六": 6, "七": 7, "八": 8, "九": 9,
}
# （戊寅、前四○三年） → ("戊寅", 403)
LABEL = re.compile(r"^（(.)(.)、前([○〇零一二三四五六七八九]+)年）$")

# Span classes that move text out of the main column.
DROP_CLASSES = {"yinwen"}  # the entry ordinal is not part of the text


def fail(message: str) -> None:
    print(f"FAIL {message}", file=sys.stderr)
    raise SystemExit(1)


def parse_label(label: str) -> tuple[str, int]:
    match = LABEL.match(label)
    if not match:
        fail(f"無法解析年份標籤：{label!r}")
    year = int("".join(str(DIGITS[ch]) for ch in match.group(3)))
    return match.group(1) + match.group(2), year


def strip_markup(segment: str) -> tuple[str, list[str], list[str]]:
    """Split one <p> body into (main text, 注文 list, 校記 list)."""
    text: list[str] = []
    notes: list[str] = []
    collations: list[str] = []
    stack: list[str] = []
    cursor = 0

    def target() -> tuple[str, list[str] | None]:
        # Transparent spans (shu-ming, yinhao, pass) keep the enclosing sink, so
        # walk down to the nearest frame that actually redirects text.
        for frame in reversed(stack):
            if frame == "note":
                return "note", notes
            if frame == "coll":
                return "coll", collations
            if frame == "drop":
                return "drop", None
        return "text", text

    for match in TAG.finditer(segment):
        kind, sinks = target()
        if sinks is not None and match.start() > cursor:
            sinks.append(html.unescape(segment[cursor:match.start()]))
        cursor = match.end()
        closing, name, attrs = match.group(1), match.group(2).lower(), match.group(3)
        if name == "a":
            continue
        if name != "span":
            continue
        if closing:
            if stack:
                stack.pop()
            continue
        span_class = ""
        found = re.search(r'class="([^"]*)"', attrs)
        if found:
            span_class = found.group(1)
        classes = set(span_class.split())
        if "jiazhu" in classes and "jiaozhusup" not in classes:
            stack.append("note")
        elif "jiaozhusup" in classes:
            stack.append("coll")
        elif classes & DROP_CLASSES:
            stack.append("drop")
        else:
            stack.append("pass")

    kind, sinks = target()
    if sinks is not None and cursor < len(segment):
        sinks.append(html.unescape(segment[cursor:]))

    def clean(chunks: list[str]) -> str:
        joined = "".join(chunks)
        joined = re.sub(r"[\r\n\t]+", "", joined)
        return joined.strip()

    return clean(text), [clean([note]) for note in notes if clean([note])], [
        clean([coll]) for coll in collations if clean([coll])
    ]


def paragraph_body(segment: str, start: int) -> str:
    """Text between a <p …> start tag and its closing </p>."""
    open_end = segment.find(">", start) + 1
    close = segment.find("</p>", open_end)
    if close < 0:
        fail(f"段未閉合：{segment[start:start + 60]!r}")
    return segment[open_end:close]


def parse_file(name: str) -> tuple[str, list[dict], list[dict]]:
    source = (CACHE / f"{name}.html").read_text(encoding="utf-8")
    juan = JUAN[name]

    heading = re.search(
        r'<h1>.*?data-head-description="([^"]*)"[^>]*data-head-title="([^"]*)"',
        source,
        re.S,
    )
    if not heading:
        fail(f"{name}.html 找不到 <h1> 卷名")
    declared = "".join(
        ch for ch in html.unescape(heading.group(2)) if "\u4e00" <= ch <= "\u9fff"
    )
    if declared != juan:
        fail(f"{name}.html 卷名不符：文件寫「{declared}」，表為「{juan}」")

    divs = [(m.start(), m.group(1)) for m in DIV.finditer(source)]
    if not divs:
        fail(f"{name}.html 沒有 div.yt")

    years: list[dict] = []
    events: list[dict] = []
    for index, (start, div_id) in enumerate(divs):
        stop = divs[index + 1][0] if index + 1 < len(divs) else len(source)
        div_end = source.find("</div>", start)
        header = source[start:div_end]

        attrs = HEAD_ATTRS.findall(header)
        if len(attrs) != 1:
            fail(f"{div_id} 的 data-head-* 不是唯一一組：{attrs}")
        ruler, ruler_year = attrs[0]
        labels = BIAOZHU.findall(header)
        if len(labels) != 1:
            fail(f"{div_id} 的 biaozhusup 不是唯一一條：{labels}")
        label = labels[0]
        ganzhi, label_year = parse_label(label)

        year_paragraph = YEAR_P.search(header)
        if not year_paragraph:
            fail(f"{div_id} 內沒有年頭 <p>")
        year_anchor = year_paragraph.group(1)
        _, year_notes, _ = strip_markup(paragraph_body(header, year_paragraph.start()))

        year = {
            "yearBc": int(div_id[1:]),
            "div": div_id,
            "label": label,
            "ganzhi": ganzhi,
            "ruler": ruler,
            "rulerYear": ruler_year,
            "juan": juan,
            "sourceFile": name,
            "anchor": year_anchor,
            "note": "".join(year_notes) if year_notes else None,
        }
        years.append(year)

        for match in EVENT_P.finditer(source, start, stop):
            anchor = match.group(1)
            text, notes, collations = strip_markup(
                paragraph_body(source, match.start())
            )
            events.append(
                {
                    "id": f"tj-{div_id}-{anchor}",
                    "yearBc": year["yearBc"],
                    "ruler": ruler,
                    "rulerYear": ruler_year,
                    "ganzhi": ganzhi,
                    "juan": juan,
                    "sourceFile": name,
                    "anchor": anchor,
                    "text": text,
                    "note": "".join(notes) if notes else None,
                    "collation": "".join(collations) if collations else None,
                }
            )
    return juan, years, events


def check(condition: bool, label: str, detail: str) -> None:
    if not condition:
        fail(f"{label} — {detail}")
    print(f"PASS {label} {detail}")


def main() -> int:
    years: list[dict] = []
    events: list[dict] = []
    for name in FILES:
        juan, file_years, file_events = parse_file(name)
        print(
            f"解析 {name}.html {juan}：{len(file_years)} 年 · {len(file_events)} 條事件"
        )
        years.extend(file_years)
        events.extend(file_events)

    # ---- check 1: div id yNNNN == yearBc -------------------------------
    for year in years:
        if int(year["div"][1:]) != year["yearBc"]:
            fail(f"檢查1 {year['div']}：id 與 yearBc {year['yearBc']} 不等")
    check(True, "檢查1 div-id 與 yearBc", f"{len(years)}/{len(years)} 相等")

    # ---- check 2: 括號內「前X年」 == yearBc ---------------------------
    for year in years:
        _, parsed = parse_label(year["label"])
        if parsed != year["yearBc"]:
            fail(f"檢查2 {year['div']}：標籤 {year['label']} 解析為 {parsed}")
    check(True, "檢查2 標籤年份與 yearBc", f"{len(years)}/{len(years)} 相等")

    # ---- check 3: ganzhi == 括號內干支 --------------------------------
    for year in years:
        ganzhi, _ = parse_label(year["label"])
        if ganzhi != year["ganzhi"]:
            fail(f"檢查3 {year['div']}：ganzhi {year['ganzhi']} ≠ {ganzhi}")
    check(True, "檢查3 干支與標籤", f"{len(years)}/{len(years)} 相等")

    # ---- check 4: ruler / rulerYear 非空且逐事件一致 -------------------
    by_year = {year["yearBc"]: year for year in years}
    for year in years:
        if not year["ruler"].strip() or not year["rulerYear"].strip():
            fail(f"檢查4 {year['div']}：ruler/rulerYear 為空")
    for event in events:
        year = by_year.get(event["yearBc"])
        if year is None:
            fail(f"檢查4 {event['id']}：年份 {event['yearBc']} 無年頭記錄")
        if (event["ruler"], event["rulerYear"], event["ganzhi"], event["juan"]) != (
            year["ruler"],
            year["rulerYear"],
            year["ganzhi"],
            year["juan"],
        ):
            fail(f"檢查4 {event['id']}：年信息與年頭記錄不一致")
    check(
        True,
        "檢查4 ruler/rulerYear",
        f"{len(years)} 個年頭、{len(events)} 條事件均取自 data-head-*",
    )

    # ---- check 5: id 唯一、anchor 真實存在於該年的源區間 --------------
    ids = set()
    for event in events:
        if event["id"] in ids:
            fail(f"檢查5 重複事件 id：{event['id']}")
        ids.add(event["id"])
        if not event["anchor"]:
            fail(f"檢查5 {event['id']}：anchor 為空")
    # Re-open every source file and prove each anchor occurs exactly once and
    # inside its own year's slice (div.yt … next div.yt).
    anchors_checked = 0
    cache: dict[str, str] = {}
    spans: dict[str, list[tuple[int, int]]] = {}
    for name in FILES:
        source = (CACHE / f"{name}.html").read_text(encoding="utf-8")
        cache[name] = source
        divs = [m.start() for m in DIV.finditer(source)]
        spans[name] = [
            (start, divs[i + 1] if i + 1 < len(divs) else len(source))
            for i, start in enumerate(divs)
        ]
    div_index = {
        year["yearBc"]: i for i in range(len(years)) for year in [years[i]]
    }
    per_file_year = {}
    for name in FILES:
        per_file_year[name] = [
            year for year in years if year["sourceFile"] == name
        ]
    for event in events:
        source = cache[event["sourceFile"]]
        if source.count(f'id="{event["anchor"]}"') != 1:
            fail(f"檢查5 {event['id']}：anchor 在源文件中不是唯一")
        position = source.find(f'id="{event["anchor"]}"')
        year_order = [y["yearBc"] for y in per_file_year[event["sourceFile"]]]
        index = year_order.index(event["yearBc"])
        lo, hi = spans[event["sourceFile"]][index]
        if not (lo <= position < hi):
            fail(f"檢查5 {event['id']}：anchor 不在 {event['yearBc']} 年的 div 區間內")
        anchors_checked += 1
    for year in years:
        source = cache[year["sourceFile"]]
        if source.count(f'id="{year["anchor"]}"') != 1:
            fail(f"檢查5 年頭 {year['div']}：anchor 在源文件中不是唯一")
    check(
        True,
        "檢查5 id/anchor",
        f"{len(ids)} 個唯一 id、{anchors_checked} 個 anchor 均在源 div 區間內",
    )

    # ---- check 6: 匯總 -------------------------------------------------
    year_numbers = sorted(year["yearBc"] for year in years)
    if len(years) != 180:
        fail(f"檢查6 div.yt 總數 {len(years)} ≠ 180")
    if len(set(year_numbers)) != len(year_numbers):
        fail("檢查6 有重複年份")
    low, high = 209, 403
    if year_numbers[0] != low or year_numbers[-1] != high:
        fail(f"檢查6 年份跨度 {year_numbers[0]}–{year_numbers[-1]} ≠ 前209–前403")
    missing = sorted(set(range(low, high + 1)) - set(year_numbers))
    if len(missing) != 15:
        fail(f"檢查6 無事件年份 {len(missing)} ≠ 15：{missing}")
    check(
        True,
        "檢查6 匯總",
        f"div.yt {len(years)} · 事件 {len(events)} · 前{high}–前{low} · 無事件年份 {len(missing)}",
    )

    document = {
        "schemaVersion": 1,
        "bookId": "zhanguoce",
        "source": "資治通鑑·周紀/秦紀（戰國段）",
        "generatedBy": "tools/build_events.py",
        "years": years,
        "events": events,
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(document, ensure_ascii=False, indent=2) + "\n"
    OUT.write_text(payload, encoding="utf-8")
    print(
        f"寫入 {OUT.relative_to(ROOT)}：{len(payload)} 字符 / "
        f"{OUT.stat().st_size} 字節 · {len(years)} 年 · {len(events)} 事件"
    )
    print(f"無事件年份（{len(missing)}）：{missing}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
