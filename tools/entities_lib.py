#!/usr/bin/env python3
"""Shared helpers for the M3a entity layer.

Only data/helpers live here: the traditional->simplified character table used
to derive nameSimp / simplified aliases, and the corpus loader.  The scanner
lives in build_entities.py and is re-implemented independently in
validate_entities.py, so neither script can hide a bug behind the other.
"""
from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "data" / "zhanguoce"
VOLUMES = DATA / "volumes"
ENTITIES = DATA / "entities"

# 逐字繁->简表。只收录本层实体名里实际出现的繁体专用字；
# 校验器要求 nameSimp 不含本表的任何键（即简体名里不得残留繁体专用字）。
T2S_PAIRS = """
蘇苏 張张 儀仪 陳陈 軫轸 孫孙 厲厉 羅罗 壽寿 馬马 錯错 應应 澤泽 呂吕 韋韦
禮礼 賈贾 鄭郑 齕龁 衞卫 韓韩 規规 倉仓 嬰婴 嘗尝 單单 臏膑 龐庞 鄒邹 顏颜
齊齐 門门 吳吴 竇窦 屢屡 縮缩 長长 樓楼 緩缓 兌兑 陽阳 義义 趙赵 靈灵 莊庄
馮冯 頗颇 觸触 讋詟 師师 龍龙 紹绍 東东 宮宫 魚鱼 獻献 蓋盖 鯉鲤 齒齿 華华
渾浑 黃黄 懷怀 頃顷 荊荆 樂乐 間间 噲哙 慶庆 劇剧 軻轲 漸渐 離离 陰阴 藍蓝
諸诸 舉举 輸输 聶聂 嚴严 讓让 談谈 駟驷 傾倾 勁劲 臨临 黨党 內内 漢汉 薊蓟
棗枣 於于 藺蔺 雲云 陸陆 濟济 剛刚 鄴邺 闕阙 滎荥 遼辽 蒼苍 晉晋 鄲郸 國国
閔闵 釐厘 讋詟 顯显 諼谖 勝胜 須须 魯鲁 連连 關关
""".split()

T2S: dict[str, str] = {}
for pair in T2S_PAIRS:
    if len(pair) != 2:
        raise SystemExit(f"bad T2S pair: {pair!r}")
    T2S[pair[0]] = pair[1]

TRAD_ONLY = frozenset(T2S)

# 国别/城的固定简称：单字地名只在这些字形上做排除（见 build_entities）。
SINGLE_CHAR_PLACES = ("秦", "齊", "楚", "趙", "魏", "韓", "燕", "宋", "衞")


def to_simp(text: str) -> str:
    return "".join(T2S.get(ch, ch) for ch in text)


def load_paragraphs() -> list[tuple[str, str]]:
    """Return [(paragraph_id, text)] in canonical book order."""
    book = json.loads((DATA / "book.json").read_text(encoding="utf-8"))
    out: list[tuple[str, str]] = []
    for vol_id in book["volumeOrder"]:
        vol = json.loads((VOLUMES / f"{vol_id}.json").read_text(encoding="utf-8"))
        for chapter in vol["chapters"]:
            for para in chapter["paragraphs"]:
                out.append((para["id"], para["text"]))
    return out


def load_chapter_titles() -> set[str]:
    book = json.loads((DATA / "book.json").read_text(encoding="utf-8"))
    titles: set[str] = set()
    for vol_id in book["volumeOrder"]:
        vol = json.loads((VOLUMES / f"{vol_id}.json").read_text(encoding="utf-8"))
        for chapter in vol["chapters"]:
            titles.add(chapter["title"])
    return titles


def expanded_forms(name: str, aliases: list[str]) -> list[str]:
    """name + aliases, each with its simplified twin inserted right after it.

    Order is fixed so reruns are byte-identical.
    """
    forms: list[str] = []

    def push(form: str) -> None:
        if form and form not in forms:
            forms.append(form)

    push(name)
    push(to_simp(name))
    for alias in aliases:
        push(alias)
        push(to_simp(alias))
    return forms
