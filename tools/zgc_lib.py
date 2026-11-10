#!/usr/bin/env python3
"""Parsing + taxonomy for the two upstream sources of 戰國策.

Source A (crazyxiaoqiang/Classical-Modern): <卷>/<章>/bitext.txt, simplified,
one blank-line separated block per record, lines 古文：… / 现代文：…
Source B (Ancient-China-Books/zhanguoce): OEBPS/Text/NNN.htm, traditional,
numbered in book order; volume label in span.style9, chapter title in p.style6,
body paragraphs in plain p.
"""
from __future__ import annotations

import html
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CACHE = ROOT / "_cache"
VENDOR = ROOT / "_vendor"

# ---------------------------------------------------------------- taxonomy ---
# (卷目录名, volumeId, groupId, 繁体卷题, 简体卷题)
CANON = [
    ("东周", "dongzhou", "dongzhou", "東周策", "东周策"),
    ("西周", "xizhou", "xizhou", "西周策", "西周策"),
    ("秦一", "qin-1", "qin", "秦策一", "秦策一"),
    ("秦二", "qin-2", "qin", "秦策二", "秦策二"),
    ("秦三", "qin-3", "qin", "秦策三", "秦策三"),
    ("秦四", "qin-4", "qin", "秦策四", "秦策四"),
    ("秦五", "qin-5", "qin", "秦策五", "秦策五"),
    ("齐一", "qi-1", "qi", "齊策一", "齐策一"),
    ("齐二", "qi-2", "qi", "齊策二", "齐策二"),
    ("齐三", "qi-3", "qi", "齊策三", "齐策三"),
    ("齐四", "qi-4", "qi", "齊策四", "齐策四"),
    ("齐五", "qi-5", "qi", "齊策五", "齐策五"),
    ("齐六", "qi-6", "qi", "齊策六", "齐策六"),
    ("楚一", "chu-1", "chu", "楚策一", "楚策一"),
    ("楚二", "chu-2", "chu", "楚策二", "楚策二"),
    ("楚三", "chu-3", "chu", "楚策三", "楚策三"),
    ("楚四", "chu-4", "chu", "楚策四", "楚策四"),
    ("赵一", "zhao-1", "zhao", "趙策一", "赵策一"),
    ("赵二", "zhao-2", "zhao", "趙策二", "赵策二"),
    ("赵三", "zhao-3", "zhao", "趙策三", "赵策三"),
    ("赵四", "zhao-4", "zhao", "趙策四", "赵策四"),
    ("魏一", "wei-1", "wei", "魏策一", "魏策一"),
    ("魏二", "wei-2", "wei", "魏策二", "魏策二"),
    ("魏三", "wei-3", "wei", "魏策三", "魏策三"),
    ("魏四", "wei-4", "wei", "魏策四", "魏策四"),
    ("韩一", "han-1", "han", "韓策一", "韩策一"),
    ("韩二", "han-2", "han", "韓策二", "韩策二"),
    ("韩三", "han-3", "han", "韓策三", "韩策三"),
    ("燕一", "yan-1", "yan", "燕策一", "燕策一"),
    ("燕二", "yan-2", "yan", "燕策二", "燕策二"),
    ("燕三", "yan-3", "yan", "燕策三", "燕策三"),
    # 宋、衞 shared one 卷 in the transmitted text (卷三十二): one volume, two groups.
    ("宋卫", "song-wei", "song-wei", "宋衛策", "宋卫策"),
    ("中山", "zhongshan", "zhongshan", "中山策", "中山策"),
]

# 一卷只归一个 group。通行分法是 11 国策，宋、衛同卷（卷三十二），故该卷与所属组同名
# song-wei，不得再被第二个 group 引用：一旦一个卷被两组同时引用，阅读器的目录树就会把
# 同一卷展开两次（卷位 34、章链接重复）。

GROUP_TITLES = [
    ("dongzhou", "東周", "东周"),
    ("xizhou", "西周", "西周"),
    ("qin", "秦策", "秦策"),
    ("qi", "齊策", "齐策"),
    ("chu", "楚策", "楚策"),
    ("zhao", "趙策", "赵策"),
    ("wei", "魏策", "魏策"),
    ("han", "韓策", "韩策"),
    ("yan", "燕策", "燕策"),
    ("song-wei", "宋衛策", "宋卫策"),
    ("zhongshan", "中山策", "中山策"),
]

DIR2VOL = {d: v for d, v, _g, _t, _s in CANON}
VOL2DIR = {v: d for d, v, _g, _t, _s in CANON}
VOL_ORDER = [v for _d, v, _g, _t, _s in CANON]
VOL_META = {v: {"dir": d, "group": g, "title": t, "titleSimp": s}
            for d, v, g, t, s in CANON}

# ------------------------------------------------------------------ opencc ---
def _load_opencc():
    if VENDOR.is_dir() and str(VENDOR) not in sys.path:
        sys.path.insert(0, str(VENDOR))
    from opencc import OpenCC
    return OpenCC("s2t"), OpenCC("t2s")


S2T, T2S = _load_opencc()


def to_trad(s):
    return S2T.convert(s)


def to_simp(s):
    return T2S.convert(s)


# 异体字表：仅用于两源「定位对齐」，不参与输出文字。
# OpenCC 的 t2s 表不收这些异体，会让 衞鞅/衛鞅 这类同字异形对不上而丢字。
VARIANT_ALIAS = str.maketrans({
    "衞": "衛",  # 衞 -> 衛
    "汚": "汙",  # 汚 -> 汙
    "得": "得",
    "彷": "遍",  # 徧 -> 遍(繁 徧/遍 通)
    "雝": "雍",  # 雝 -> 雍
    "麤": "粗",  # 麤 -> 粗
    "鴈": "雁",  # 鴈 -> 雁
    "鼈": "鱉",  # 鼈 -> 鱉
    "醎": "鹹",  # 醎 -> 鹹
    "懽": "歡",  # 懽 -> 歡
    "愬": "訴",  # 愬 -> 訴
    "慴": "攝",  # 慴 -> 懾
    "箒": "帚",  # 箒 -> 帚
    "阨": "厄",  # 阨 -> 厄
    "闚": "窺",  # 闚 -> 窺
    "皐": "皋",  # 皐 -> 皋
    "巿": "市",  # 巿 -> 市
    "柰": "奈",  # 柰 -> 奈
    "拏": "拿",  # 拏 -> 拿
    "髠": "髡",  # 髠 -> 髡
    "崐": "崑",  # 崐 -> 崑
})


def canon_simp(s):
    """t2s 的不动点。衞→衛→卫 是多级映射，单遍 t2s 归一不了，会让对齐错位。"""
    for _ in range(4):
        t = to_simp(s.translate(VARIANT_ALIAS))
        if t == s:
            return t
        s = t
    return s


# ------------------------------------------------------------- source A ------
A_RE = re.compile(r"^\s*(古文|现代文)\s*[：:]\s*(.*)$")


def parse_a_chapter(path):
    """-> [(古文简体, 白话简体)], one entry per blank-line separated block."""
    text = path.read_text(encoding="utf-8-sig")
    pairs = []
    for block in re.split(r"\n\s*\n", text.strip()):
        if not block.strip():
            continue
        g = m = ""
        for line in block.split("\n"):
            mm = A_RE.match(line)
            if not mm:
                continue
            if mm.group(1) == "古文":
                g = mm.group(2).strip()
            else:
                m = mm.group(2).strip()
        pairs.append((g, m))
    return pairs


def load_a():
    """-> {volDir: {chapterDir: [(guwen, modern)]}}"""
    out = {d: {} for d, *_ in CANON}
    base = CACHE / "a" / "双语数据" / "战国策"
    for vol_dir in out:
        for ch_dir in sorted((base / vol_dir).iterdir()):
            bt = ch_dir / "bitext.txt"
            if bt.is_file():
                out[vol_dir][ch_dir.name] = parse_a_chapter(bt)
    return out


# ------------------------------------------------------------- source B ------
SPAN_RE = re.compile(r'<span class="style9">(.*?)</span>', re.S)
TITLE_RE = re.compile(r'<p class="style6">(.*?)</p>', re.S)
P_RE = re.compile(r"<p(?:\s[^>]*)?>(.*?)</p>", re.S)
TAG_RE = re.compile(r"<[^>]+>")
NUM_RE = re.compile(r"(\d+)")


def _clean(fragment):
    return html.unescape(TAG_RE.sub("", fragment)).replace("\u3000", " ").strip()


def norm_vol_label(label):
    """卷三十二 ‧ 宋、衞策 -> 宋卫 ; 卷三 ‧ 秦策一 -> 秦一"""
    body = label.split("‧")[-1].strip()
    body = re.sub(r"^卷[〇零一二三四五六七八九十百]+", "", body).strip()
    if body.startswith("宋") and len(body.replace("、", "")) <= 4:
        return "宋卫"
    body = body.replace("策", "").replace("、", "").replace(" ", "")
    return to_simp(body)


def load_b():
    """-> [{no, file, volLabel, volDir, title, titleSimp, paragraphs}] in book order."""
    base = CACHE / "b" / "OEBPS" / "Text"
    rows = []
    for path in sorted(base.glob("*.htm")):
        raw = path.read_text(encoding="utf-8-sig", errors="replace")
        span = SPAN_RE.search(raw)
        title_m = TITLE_RE.search(raw)
        no = int(NUM_RE.search(path.stem).group(1))
        if not span or not title_m:
            rows.append({"no": no, "file": path.name, "broken": True})
            continue
        label = _clean(span.group(1))
        title = _clean(title_m.group(1))
        body = raw[title_m.end():]
        paras = [p for p in (_clean(m.group(1)) for m in P_RE.finditer(body)) if p]
        rows.append({
            "no": no,
            "file": path.name,
            "volLabel": label,
            "volDir": norm_vol_label(label),
            "title": title,
            "titleSimp": to_simp(title),
            "paragraphs": paras,
        })
    rows.sort(key=lambda r: r.get("no", 0))
    return rows


# --------------------------------------------------------------- helpers -----
# --------------------------------------------------------------- helpers -----
_PUNCT = set(
    " ~!@#$%^&*()_+-=[]{}|;:'\",.<>/?"
    "\\　·—‘’“”…、。《》"
    "「」『』【】！（），：；？［］～｜－"
)
_PUNCT_TABLE = {ord(c): None for c in _PUNCT}


def strip_punct(s):
    return s.translate(_PUNCT_TABLE)
