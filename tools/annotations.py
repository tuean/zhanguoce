#!/usr/bin/env python3
"""M1b · mount 鮑彪注 / 吳師道補正 (kanripo KR2e0003) onto the 6663 paragraphs.

对齐绝不按卷/条序号：鮑彪「改定其次第」且改定章题（实测目录作「魏鞅亡魏入秦」，
通行本作「衞鞅亡魏入秦」），故只能用内容定位（docs/PLAN.md 第七节 硬约束 2）。

链条：
  1. 章：用「章首句片段」在 kanripo 正文（去注、去句读）里投票定位。取章内若干
     短 n-gram（6 字，隔 4 字取一个），每个命中位置减去它在章内的偏移后投票，
     得票最多者为章首。短 gram 是为了扛异体/异文（諼/驩、衞/衛 这类）。
  2. 章内：把本库整章正文与 kanripo 以该位置起、长 2 倍章长 + 60 的窗口做字符级
     对齐（SequenceMatcher），得「body 下标 -> 本库字序」映射，并把章的有效区间
     裁到真正对齐上的首尾块；覆盖率不足者判对齐失败，不挂任何注。
  3. 段：每个注块按其所附原文片段的位置（注紧跟被注正文之后）经映射落到章内字序，
     再按段偏移归属到段。落不进任何已对齐区间、或落点在映射空洞里的注块不挂载。
"""
from __future__ import annotations

import bisect
import difflib
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kanripo_lib as K
import zgc_lib as L

PUNC = {ord(c): None for c in
        "　 ·—‘’“”…、。《》「」『』【】（）〔〕！（），：；？［］～｜－○〇\"'"
        " \t\n\r"}

GRAM = 6                 # 投票用 n-gram 长度
GRAM_STEP = 4            # 取样步长
MIN_VOTES = 3            # 至少几张票才算定位到（短章 2 票，仍须过覆盖率）
SHORT_CHAPTER = 150
MIN_COVERAGE = 0.45      # 章级最低：低于此判「对不上」，一个注都不挂
FULL_COVERAGE = 0.80     # 达到此值记「完整对齐」，否则记「部分对齐」
MIN_MATCHED = 50         # 对齐上的字数下限
ANCHOR_TOL = 3           # 注位置可落在对齐块末尾之后几个字
ANCHOR_RATIO = 0.60      # 注前 6 字的相似度下限，低于此判该处错位
FALLBACK_GAP = 300       # 整卷兜底：相距小于此值的命中块并作一簇（body 字数）

SOURCE_BASE = "鲍彪"
SOURCE_BU = "吴师道补"
SOURCE_ZHENG = "吴师道正"

MARK_BU = re.compile(r"[補𥙷][曰注]")
MARK_ZHENG = re.compile(r"正曰")
MARK_BIAO = re.compile(r"彪[謂曰]")


def plain(s: str) -> str:
    return L.canon_simp(s.translate(PUNC))


def _ratio(a: str, b: str) -> float:
    return difflib.SequenceMatcher(None, a, b, autojunk=False).ratio()


def _occ(hay: str, needle: str, limit: int = 60) -> list[int]:
    out, i = [], hay.find(needle)
    while i >= 0 and len(out) < limit:
        out.append(i)
        i = hay.find(needle, i + 1)
    return out


class Corpus:
    """kanripo 正文（去注、去句读）与注块，坐标统一为 plain 下标。"""

    def __init__(self, parsed: dict | None = None) -> None:
        parsed = parsed if parsed is not None else K.build()
        self.stats = parsed["stats"]
        self.notes = [n for n in parsed["notes"] if not n.get("drop")]
        text, plain_idx = K.body_plain(parsed["body"])
        text = L.canon_simp(text)
        # 跳过 序 / 目錄 / 提要：正文自「西周」卷首（篇题「西周」+ 首句「安王」）起
        self.offset = max(0, text.find("西周安王"))
        self.up = text[self.offset:]
        self.by_pos: dict[int, list[dict]] = {}
        for n in self.notes:
            p = bisect.bisect_left(plain_idx, n["bodyPos"]) - self.offset
            n["plainPos"] = p
            self.by_pos.setdefault(p, []).append(n)
        self.positions = sorted(self.by_pos)


def _votes(up: str, cp: str) -> dict[int, int]:
    votes: dict[int, int] = {}
    for i in range(0, max(1, len(cp) - GRAM), GRAM_STEP):
        gram = cp[i:i + GRAM]
        if len(gram) < GRAM:
            break
        for p in _occ(up, gram, limit=40):
            votes[p - i] = votes.get(p - i, 0) + 1
    return votes


def _align(up: str, cp: str, pos: int):
    """-> (mapping, span, coverage)。span 为对齐上的 body 区间（up 下标）。"""
    window = up[max(0, pos):min(len(up), max(0, pos) + len(cp) * 2 + 60)]
    sm = difflib.SequenceMatcher(None, cp, window, autojunk=False)
    blocks = [b for b in sm.get_matching_blocks() if b.size]
    if not blocks:
        return None, None, 0.0
    coverage = sum(b.size for b in blocks) / max(1, len(cp))
    mapping = {}
    for b in blocks:
        for k in range(b.size):
            mapping[max(0, pos) + b.b + k] = b.a + k
    span = (max(0, pos) + blocks[0].b, max(0, pos) + blocks[-1].b + blocks[-1].size)
    return mapping, span, coverage


def _fallback_align(up: str, cp: str):
    """整卷兜底：先在全书正文里找本章最长的成片命中，再在该处局部重对齐。

    章首投票要求若干 n-gram 落在同一相位；若本章开头恰在缺叶/异文处，投票会散掉，
    但章中后段仍可能整片存在于底本。这里不投票，直接看命中块的分布：
    把相距 < FALLBACK_GAP 的命中块聚成一簇，取总长最大的一簇，再按 _align 的窗口
    规则重对齐一次（仍要过 MIN_COVERAGE / MIN_MATCHED 两道闸，不放宽）。
    """
    sm = difflib.SequenceMatcher(None, cp, up, autojunk=False)
    blocks = [b for b in sm.get_matching_blocks() if b.size >= GRAM]
    if not blocks:
        return None, None, 0.0
    clusters: list[list] = []
    for b in blocks:
        if clusters and b.b - clusters[-1][-1].b < FALLBACK_GAP:
            clusters[-1].append(b)
        else:
            clusters.append([b])
    best = max(clusters, key=lambda cl: sum(x.size for x in cl))
    lo = max(0, best[0].b - 60)
    return _align(up, cp, lo)


def place_chapters(corpus: Corpus, chapters: list[dict]) -> dict[str, dict]:
    up = corpus.up
    placed: dict[str, dict] = {}
    for ch in chapters:
        cp = ch["plain"]
        if len(cp) < GRAM * 3:
            placed[ch["id"]] = {"ok": False, "reason": "chapter-too-short"}
            continue
        votes = _votes(up, cp)
        need = 2 if len(cp) < SHORT_CHAPTER else MIN_VOTES
        cands = [pos for pos, n in sorted(votes.items(), key=lambda kv: (-kv[1], kv[0]))
                 if n >= need][:4]
        best = None
        for pos in cands:
            mapping, span, cov = _align(up, cp, pos)
            if mapping and cov >= FULL_COVERAGE:
                best = (mapping, span, cov)
                break
            if mapping and (best is None or cov > best[2]):
                best = (mapping, span, cov)
        matched = len(best[0]) if best else 0
        floor = min(MIN_MATCHED, int(len(cp) * 0.80))
        method = "opening-vote"
        fb = None
        if not (best and best[2] >= MIN_COVERAGE and matched >= floor):
            # 章首投票不成：改走整卷兜底（同一覆盖率闸门，不因兜底而放宽）
            fb = _fallback_align(up, cp)
            fb_matched = len(fb[0]) if fb[0] else 0
            method = "volume-fallback"
            if fb[0] and fb[2] >= MIN_COVERAGE and fb_matched >= floor:
                best, matched = fb, fb_matched
            else:
                method = None
        if method and best:
            placed[ch["id"]] = {"ok": True, "mapping": best[0], "span": best[1],
                                "coverage": best[2], "matched": matched,
                                "votes": max(votes.values()), "method": method,
                                "partial": best[2] < FULL_COVERAGE}
        else:
            placed[ch["id"]] = {
                "ok": False,
                "reason": (f"chapter-unlocated:votes={max(votes.values()) if votes else 0}"
                           if not votes or max(votes.values()) < need else
                           f"chapter-low-coverage:{best[2] if best else 0:.2f}"
                           f"/matched={matched}/{len(cp)}"),
                "coverage": best[2] if best else 0.0,
                "fallbackCoverage": fb[2] if fb and fb[0] else 0.0,
                "fallbackMatched": len(fb[0]) if fb and fb[0] else 0}
    return placed


def split_note(raw: str) -> list[tuple[str, str]]:
    """注块正文 -> [(source, text)]。无标记的前段=鮑彪；補曰/正曰/彪謂 各自起段。"""
    marks = []
    for rx, src in ((MARK_BU, SOURCE_BU), (MARK_ZHENG, SOURCE_ZHENG),
                    (MARK_BIAO, SOURCE_BASE)):
        for m in rx.finditer(raw):
            marks.append((m.start(), m.end(), src))
    marks.sort()
    if not marks:
        return [(SOURCE_BASE, raw)] if raw else []
    out = []
    if marks[0][0] > 0:
        out.append((SOURCE_BASE, raw[:marks[0][0]]))
    for i, (_s, e, src) in enumerate(marks):
        end = marks[i + 1][0] if i + 1 < len(marks) else len(raw)
        out.append((src, raw[e:end]))
    return out


# 「X，Y也」型紧注：注文首字即被注字，Y 是训释本体。
# 宁缺毋滥：Y 超过 4 字、或含 爲/所/此/者/其/亦/而/以/於/之/乎 的，是句子不是训释。
GLOSS_RE = re.compile(r"^(?P<term>[\u4e00-\u9fff])(?P<def>[\u4e00-\u9fff]{1,4})也$")
GLOSS_BAN = set("爲所此者其亦而以於之乎也不")
GLOSS_TERM_BAN = set("不也而其此之所爲以於乎焉則且乃夫何若使可無有為")


def extract_glossary(body_before: str, note_raw: str) -> list[dict]:
    """反抽训释；term 必须就出现在注块紧邻的正文里（注文首字=被注字）。"""
    mg = GLOSS_RE.match(note_raw)
    if not mg:
        return []
    if GLOSS_BAN & set(mg.group("def")):
        return []
    term = mg.group("term")
    if term in GLOSS_TERM_BAN or term not in body_before[-6:]:
        return []
    return [{"term": term, "pinyin": None, "gloss": mg.group("def") + "也"}]


def _locate(up: str, cp: str, mapping: dict, pos: int):
    """注块位置 -> (章内字序 a, 状态)。状态 ∈ ok / nomap / anchor。"""
    a = mapping.get(pos)
    if a is None:
        for d in range(1, ANCHOR_TOL + 1):     # 注紧跟被注词，末字常被异文吃掉
            a = mapping.get(pos - d)
            if a is not None:
                a += d
                break
    if a is None:
        return None, "nomap"
    # 锚点复核：注紧跟被注词，故注前的正文片段必须与本库同位置文字一致。
    # 对不上的说明上面那一小段发生了错位（本库漏句/异文吞句），宁可不挂。
    n_b = min(6, a or 0, pos)
    if n_b >= 4 and _ratio(up[pos - n_b:pos], cp[a - n_b:a]) < ANCHOR_RATIO:
        return None, "anchor"
    return a, "ok"


def attach(corpus: Corpus, chapters: list[dict]):
    """-> (per-para annotations, per-para glossary, stats, placement)

    挂载分两轮。第一轮沿用旧规则（注块归覆盖它的最小有效区间），逐字保持既有挂载；
    第二轮只处理第一轮挂不上的注块（映射空洞 / 锚点复核不过），在同时覆盖该位置的
    其它章里按对齐覆盖率从高到低再复核一次。两轮都以「映射 + 注前 N 字锚点」为准，
    绝不按章序或块序就近猜挂；一个位置至多挂一次。
    """
    for ch in chapters:
        ch["plain"] = "".join(plain(p["text"]) for p in ch["paragraphs"])
    placed = place_chapters(corpus, chapters)
    up = corpus.up

    # 章的有效区间（防重叠）：注块若同时落在两章区间，归区间最小者
    spans = sorted((v["span"][0], v["span"][1], ch_id)
                   for ch_id, v in placed.items() if v["ok"])
    # 第二轮用：覆盖区间 + 对齐质量，覆盖率高的优先复核
    cover = sorted(((v["span"][0], v["span"][1], cid, v["coverage"])
                    for cid, v in placed.items() if v["ok"]),
                   key=lambda t: (-t[3], t[1] - t[0], t[2]))
    per_para: dict[str, list] = {}
    per_gloss: dict[str, list] = {}
    by_chapter = {ch["id"]: ch for ch in chapters}
    stats = {"chapters": len(chapters), "chaptersAligned": 0, "chaptersPartial": 0,
             "chaptersAnnotated": 0, "chaptersFallback": 0,
             "notes": len(corpus.notes), "notesAttached": 0, "notesUnplaced": 0,
             "notesNoAnchor": 0, "notesEmpty": 0, "glossary": 0,
             "notesRecoveredByNeighbour": 0, "notesDuplicateMount": 0,
             "bySource": {SOURCE_BASE: 0, SOURCE_BU: 0, SOURCE_ZHENG: 0},
             "unaligned": [], "unplacedSamples": [],
             "chapterDetail": [], "glossaryTrace": [], "recoveredSamples": [],
             "notesUnmountedByVolume": {}}
    counts = {ch["id"]: {"annotated": 0, "unmounted": 0, "recovered": 0}
              for ch in chapters}
    offs_of: dict[str, list] = {}
    for ch in chapters:
        if not placed.get(ch["id"], {}).get("ok"):
            continue
        offs, acc = [], 0
        for p in ch["paragraphs"]:
            n = len(plain(p["text"]))
            offs.append((acc, acc + n))
            acc += n
        offs_of[ch["id"]] = offs

    def emit(cid: str, pos: int, a: int, notes: list) -> str | None:
        if pos in mounted:                    # 同一注块不得挂两次
            stats["notesDuplicateMount"] += 1
            return None
        mounted.add(pos)
        offs = offs_of[cid]
        # 注紧跟被注词之后：落点 a 恰在段界时归前一段（注的是前一段的末字）
        idx = next((k for k, (s, e) in enumerate(offs)
                    if s < a <= max(e, s + 1)), None)
        if idx is None:
            idx = 0 if a == 0 else len(offs) - 1
        para = by_chapter[cid]["paragraphs"][idx]
        body_before = up[max(0, pos - 8):pos]
        attached = 0
        for n in notes:
            pieces = [(src, t) for src, t in split_note(n["raw"]) if t]
            if not pieces:
                stats["notesEmpty"] += 1
                continue
            bucket = per_para.setdefault(para["id"], [])
            attached += 1
            stats["notesAttached"] += 1
            counts[cid]["annotated"] += 1
            for src, text in pieces:
                bucket.append({"source": src, "text": text})
                stats["bySource"][src] += 1
            for src, text in pieces:
                for g in extract_glossary(body_before, text):
                    per_gloss.setdefault(para["id"], []).append(g)
                    stats["glossary"] += 1
                    stats["glossaryTrace"].append(
                        {"paraId": para["id"], "term": g["term"],
                         "gloss": g["gloss"], "source": src, "note": text})
        return para["id"] if attached else None

    mounted: set[int] = set()
    for ch in chapters:
        info = placed.get(ch["id"], {"ok": False, "reason": "chapter-unlocated"})
        if not info["ok"]:
            continue
        mapping, (span_s, span_e) = info["mapping"], info["span"]
        cp = ch["plain"]
        # pos == span_s 的注块注的是章前之物（篇题/王名），不是本章正文，排除
        lo = bisect.bisect_right(corpus.positions, span_s)
        hi = bisect.bisect_right(corpus.positions, span_e)
        for pos in corpus.positions[lo:hi]:
            if _owner(spans, pos) != ch["id"]:
                continue                      # 该位置属另一章的有效区间
            a, why = _locate(up, cp, mapping, pos)
            notes = corpus.by_pos[pos]
            if why == "anchor":
                stats["notesAnchorReject"] = stats.get("notesAnchorReject", 0) + len(notes)
            if a is None:
                stats["notesUnplaced"] += len(notes)
                counts[ch["id"]]["unmounted"] += len(notes)
                if len(stats["unplacedSamples"]) < 40:
                    stats["unplacedSamples"].append(
                        {"chapterId": ch["id"], "pos": pos, "text": notes[0]["raw"][:20]})
                continue
            emit(ch["id"], pos, a, notes)

    # 第二轮：第一轮挂不上的，换同时覆盖该位置的其它章复核
    for pos in corpus.positions:
        if pos in mounted:
            continue
        own = _owner(spans, pos)
        if own is None:
            continue                          # 不在任何已对齐区间内：本就不挂
        own_s, own_e = placed[own]["span"]
        if not (own_s < pos <= own_e):
            continue                          # 与第一轮同口径：章首那一位注的是篇题，不挂
        notes = corpus.by_pos[pos]
        for s, e, cid, _cov in cover:
            if cid == own or not (s <= pos <= e):
                continue
            a, _why = _locate(up, by_chapter[cid]["plain"], placed[cid]["mapping"], pos)
            if a is None:
                continue
            pid = emit(cid, pos, a, notes)
            if pid and len(stats["recoveredSamples"]) < 60:
                stats["recoveredSamples"].append(
                    {"chapterId": cid, "ownChapter": own, "paraId": pid,
                     "note": notes[0]["raw"][:80],
                     "anchorEdition": up[max(0, pos - 6):pos],
                     "anchorCorpus": by_chapter[cid]["plain"][max(0, a - 6):a]})
            counts[cid]["recovered"] += len(notes)
            counts[own]["unmounted"] -= len(notes)
            stats["notesRecoveredByNeighbour"] += len(notes)
            break

    for ch in chapters:
        info = placed.get(ch["id"], {"ok": False, "reason": "chapter-unlocated"})
        c = counts[ch["id"]]
        if not info["ok"]:
            stats["unaligned"].append({"id": ch["id"], "title": ch["title"],
                                       "why": info.get("reason", "chapter-unlocated"),
                                       "paragraphs": len(ch["paragraphs"]),
                                       "chars": len(ch["plain"]),
                                       "fallbackCoverage": info.get("fallbackCoverage", 0.0),
                                       "fallbackMatched": info.get("fallbackMatched", 0)})
            stats["chapterDetail"].append({
                "id": ch["id"], "title": ch["title"], "aligned": False, "method": None,
                "partial": False, "coverage": 0.0, "annotated": 0,
                "recovered": 0, "unmounted": 0,
                "fallbackCoverage": info.get("fallbackCoverage", 0.0),
                "fallbackMatched": info.get("fallbackMatched", 0),
                "why": info.get("reason", "chapter-unlocated")})
            continue
        stats["chaptersAligned"] += 1
        if info.get("partial"):
            stats["chaptersPartial"] += 1
        if info.get("method") == "volume-fallback":
            stats["chaptersFallback"] += 1
        if c["annotated"]:
            stats["chaptersAnnotated"] += 1
        stats["chapterDetail"].append({
            "id": ch["id"], "title": ch["title"], "aligned": True,
            "method": info.get("method", "opening-vote"),
            "partial": bool(info.get("partial")), "coverage": info["coverage"],
            "annotated": c["annotated"], "recovered": c["recovered"],
            "unmounted": c["unmounted"], "why": None})
    for ch in chapters:
        ch.pop("plain", None)
    stats["notesUnplaced"] = max(0, stats["notesUnplaced"] - stats["notesRecoveredByNeighbour"])
    stats["notesOutsideAligned"] = (stats["notes"] - stats["notesAttached"]
                                    - stats["notesUnplaced"])
    # 未挂载注块按「最近的已对齐章」落到卷，便于逐卷统计（仅为报表归属，不改数据）
    for pos, notes in corpus.by_pos.items():
        if pos in mounted:
            continue
        if _owner(spans, pos) is None:
            cid = _nearest(spans, pos)
            vid = cid.rsplit("-", 1)[0] if cid else "?"
            bucket = stats["notesUnmountedByVolume"]
            bucket[vid] = bucket.get(vid, 0) + len(notes)
    stats["parse"] = corpus.stats
    stats["constants"] = {"gram": GRAM, "gramStep": GRAM_STEP, "minVotes": MIN_VOTES,
                          "minCoverage": MIN_COVERAGE, "fullCoverage": FULL_COVERAGE,
                          "minMatched": MIN_MATCHED, "anchorTol": ANCHOR_TOL,
                          "anchorRatio": ANCHOR_RATIO,
                          "fallbackGap": FALLBACK_GAP}
    stats["paragraphs"] = sum(len(c["paragraphs"]) for c in chapters)
    stats["paragraphsAnnotated"] = len(per_para)
    stats["annotations"] = sum(len(v) for v in per_para.values())
    return per_para, per_gloss, stats, placed


# ---- M1d：第二見證本（四庫全書本 KR2e0005）交叉補注 ----------------------
# 只往「SBCK 一概無注」之處補：章內段覆蓋率 < 0.80 的章，且只補當前零注的段。
# 既有掛載一律不動——本函數只向 per_para 追加新條目，從不改寫已有條目。
# 人（source）不變，所據本子（edition）不同，故新條目帶 edition 標記。
EDITION_SBCK = "四部叢刊"
EDITION_WYG = "四庫全書"
FILL_COVERAGE = 0.80       # 章段覆蓋率達此值即視為「已有注疏」，不再補
DEDUPE_RATIO = 0.90        # 同段去重的前綴相似度門檻
_MARKER_CHARS = set("補𥙷曰正注彪謂姚云劉本")
_STRIP_CHARS = "¶\u3000\n\t\r/　 "


def _norm_note(text: str) -> str:
    """去标点、去空白的比较用形。"""
    return "".join(c for c in text.translate(PUNC) if not c.isspace())


def _same_prefix(a: str, b: str) -> bool:
    """完全相同，或较短者与另一條同長度前綴 >=0.90 相似。"""
    n = min(len(a), len(b))
    if not n:
        return False
    return a[:n] == b[:n] or _ratio(a[:n], b[:n]) >= DEDUPE_RATIO


def _clean_note(raw: str) -> str:
    """去掉版式殘留（¶/折行/全角空格）與純標記殘片（如黏連產生的「補」）。"""
    t = raw
    for c in _STRIP_CHARS:
        t = t.replace(c, "")
    t = t.strip()
    norm = _norm_note(t)
    if len(norm) < 2 or all(c in _MARKER_CHARS for c in norm):
        return ""
    return t


def attach_wyg(chapters: list[dict], per_para: dict[str, list],
               sbck_cover: dict[str, float]) -> dict:
    """把 WYG 注補進零注段；就地擴充 per_para，返回可審計報告。"""
    corpus = Corpus(K.build_wyg())
    up = corpus.up
    for ch in chapters:
        ch["plain"] = "".join(plain(p["text"]) for p in ch["paragraphs"])
    placed = place_chapters(corpus, chapters)
    spans = sorted((v["span"][0], v["span"][1], cid)
                   for cid, v in placed.items() if v["ok"])
    mounted: set[int] = set()
    pre_annotated = set(per_para)          # 進函數時已有注的段：規則 1(b) 一律不補
    stats = {"edition": EDITION_WYG, "blocksParsed": corpus.stats["notes"],
             "blocksUsable": len(corpus.notes), "chaptersAligned": 0,
             "chaptersInScope": 0, "chaptersFilled": 0, "paragraphsFilled": 0,
             "notesAdded": 0, "notesDroppedDedupe": 0, "notesDroppedEmpty": 0,
             "notesAnchorReject": 0, "notesUnplaced": 0,
             "notesOnAnnotatedParagraph": 0, "notesOwnerRelaxed": 0,
             "bySource": {SOURCE_BASE: 0, SOURCE_BU: 0, SOURCE_ZHENG: 0},
             "detail": [], "samples": [], "added": [], "anchorRatioMin": 1.0}
    for ch in chapters:
        info = placed.get(ch["id"], {"ok": False})
        cov = sbck_cover.get(ch["id"], 0.0)
        rec = {"id": ch["id"], "title": ch["title"], "paragraphs": len(ch["paragraphs"]),
               "sbckCoverage": round(cov, 4), "zeroBefore": cov <= 0.0,
               "inScope": False, "wygCoverage": None, "notesAdded": 0,
               "paragraphsFilled": 0, "dropped": 0, "samples": []}
        stats["detail"].append(rec)
        if not info.get("ok"):
            rec["reason"] = info.get("reason", "chapter-unlocated")
            continue
        stats["chaptersAligned"] += 1
        rec["wygCoverage"] = round(info["coverage"], 4)
        if cov >= FILL_COVERAGE:
            continue
        rec["inScope"] = True
        stats["chaptersInScope"] += 1
        cp = ch["plain"]
        mapping, (span_s, span_e) = info["mapping"], info["span"]
        offs, acc = [], 0
        for p in ch["paragraphs"]:
            n = len(plain(p["text"]))
            offs.append((acc, acc + n))
            acc += n
        filled_here: set[str] = set()
        lo = bisect.bisect_right(corpus.positions, span_s)
        hi = bisect.bisect_right(corpus.positions, span_e)
        for pos in corpus.positions[lo:hi]:
            if pos in mounted:
                continue
            relaxed = False
            owner = _owner(spans, pos)
            if owner != ch["id"]:
                # 章末總注恰好落在本章 span_e，也是下一章 span_s；下一章規則本就
                # 排除 span_s 的注（篇題注），故此塊只可能屬本章末尾。
                nxt = placed.get(owner) if owner else None
                if not (rec["zeroBefore"] and pos == span_e and nxt
                        and nxt.get("ok") and nxt["span"][0] == pos):
                    continue
                relaxed = True
            a, _why = _locate(up, cp, mapping, pos)
            blocks = corpus.by_pos[pos]
            if a is None:
                stats["notesUnplaced" if _why == "nomap" else "notesAnchorReject"] += len(blocks)
                continue
            idx = next((k for k, (s, e) in enumerate(offs)
                        if s < a <= max(e, s + 1)), None)
            if idx is None:
                idx = 0 if a == 0 else len(offs) - 1
            para = ch["paragraphs"][idx]
            if para["id"] in pre_annotated:  # 規則 1(b)：只補零注段
                stats["notesOnAnnotatedParagraph"] += len(blocks)
                continue
            bucket = per_para.get(para["id"], [])
            keep = []
            for n in blocks:
                for src, text in split_note(n["raw"]):
                    text = _clean_note(text)
                    if not text:
                        stats["notesDroppedEmpty"] += 1
                        continue
                    norm = _norm_note(text)
                    if any(_same_prefix(norm, _norm_note(x["text"]))
                           for x in bucket + keep):
                        stats["notesDroppedDedupe"] += 1
                        rec["dropped"] += 1
                        continue
                    keep.append({"source": src, "text": text,
                                 "edition": EDITION_WYG})
            if not keep:
                continue
            mounted.add(pos)
            per_para.setdefault(para["id"], []).extend(keep)
            if relaxed:
                stats["notesOwnerRelaxed"] += len(keep)
            ratio = 1.0
            if a >= 4:
                ratio = _ratio(up[max(0, pos - 6):pos], cp[max(0, a - 6):a])
            stats["anchorRatioMin"] = min(stats["anchorRatioMin"], ratio)
            rec["notesAdded"] += len(keep)
            stats["notesAdded"] += len(keep)
            if para["id"] not in filled_here:
                filled_here.add(para["id"])
                rec["paragraphsFilled"] += 1
                stats["paragraphsFilled"] += 1
            for k in keep:
                stats["bySource"][k["source"]] += 1
                # 逐条审计记录：每一条补入的注都带自己的锚文对（见 tools/_wyg_fill_report.json）。
                stats["added"].append({
                    "chapterId": ch["id"], "title": ch["title"],
                    "paraId": para["id"], "source": k["source"], "text": k["text"],
                    "editionAnchor": up[max(0, pos - 6):pos],
                    "corpusAnchor": cp[max(0, a - 6):a],
                    "anchorRatio": round(ratio, 4), "relaxedOwner": relaxed})
            sample = {"chapterId": ch["id"], "title": ch["title"],
                      "paraId": para["id"], "source": keep[0]["source"],
                      "text": keep[0]["text"][:120],
                      "editionAnchor": up[max(0, pos - 6):pos],
                      "corpusAnchor": cp[max(0, a - 6):a],
                      "anchorRatio": round(ratio, 4),
                      "paragraph": para["text"][:60]}
            if len(rec["samples"]) < 3:
                rec["samples"].append(sample)
            if len(stats["samples"]) < 60:
                stats["samples"].append(sample)
    for ch in chapters:
        ch.pop("plain", None)
    stats["blocksMounted"] = len(mounted)
    stats["chaptersFilled"] = sum(1 for d in stats["detail"] if d["notesAdded"])
    stats["anchorRatioMin"] = round(stats["anchorRatioMin"], 4)
    return stats


def _owner(spans, pos: int):
    """(start, end, chapterId) 列表 -> 覆盖 pos 的最小间间的章 id"""
    best = None
    for s, e, cid in spans:
        if s <= pos <= e and (best is None or e - s < best[0]):
            best = (e - s, cid)
    return best[1] if best else None


def _nearest(spans, pos: int):
    """离 pos 最近的已对齐章（按区间距离）。"""
    best = None
    for s, e, cid in spans:
        d = s - pos if pos < s else (pos - e if pos > e else 0)
        if best is None or d < best[0]:
            best = (d, cid)
    return best[1] if best else None


if __name__ == "__main__":
    root = Path(__file__).resolve().parent.parent
    book = json.loads((root / "data/zhanguoce/book.json").read_text(encoding="utf-8"))
    chapters = []
    for vid in book["volumeOrder"]:
        v = json.loads((root / "data/zhanguoce/volumes" / f"{vid}.json")
                       .read_text(encoding="utf-8"))
        for ch in v["chapters"]:
            chapters.append({"id": ch["id"], "title": ch["title"],
                             "paragraphs": [dict(p) for p in ch["paragraphs"]]})
    corpus = Corpus()
    per_para, per_gloss, stats, placed = attach(corpus, chapters)
    brief = {k: v for k, v in stats.items()
             if k not in ("unplacedSamples",)}
    ua = brief.pop("unaligned")
    print(json.dumps(brief, ensure_ascii=False, indent=2))
    print("unaligned", len(ua))
    for u in ua:
        print("   ", u["id"], u["title"], u["why"], u["chars"])
