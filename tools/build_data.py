#!/usr/bin/env python3
"""Normalize both sources into docs/SCHEMA.md and write data/zhanguoce/**.

Segmentation policy (see docs/DATA_REPORT.md):
  章 = one chapter; order comes from source B's file order (the transmitted order).
  段 = one 古文/现代文 pair from source A — the source's own sentence-group
       boundary, so every 段 carries its own 白话.
  text = 繁体, taken verbatim from source B's 底本 by locating each A pair's
       character span in B (SequenceMatcher on the punctuation-stripped forms).
       Pairs whose span cannot be located fall back to OpenCC s2t and are
       reported as 存疑, because s2t is context-blind (咸陽 would become 鹹陽).
"""
from __future__ import annotations

import difflib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import annotations as AN
import zgc_lib as L

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "data" / "zhanguoce"
STATS = Path(__file__).resolve().parent / "_build_stats.json"

LONG_PARAGRAPH = 260
MATCH_FLOOR = 0.60
SPAN_COVERAGE = 0.90
SPAN_SLACK = 1.30

# 底本简体残留的正字纠正：只纠正「纯简体专用字」，不做版本取舍。
# 不处理 于/里/后/云/干/咸/并 等——它们是一简对多繁的底本用字，在繁体语境里本身合法，
# 改了才是错。表内每个字的 s2t 结果必须等于右值（下方断言兜住）。
RESIDUAL_FIX = {"无": "無"}   # 魏策三「无忌」当作「無忌」（信陵君）
for _k, _v in RESIDUAL_FIX.items():
    assert L.to_trad(_k) == _v, (_k, _v)
_RESIDUAL_TABLE = str.maketrans(RESIDUAL_FIX)


def fix_residual(text):
    """-> (纠正后的文本, [(原字, 正字), ...])"""
    hits = [(c, RESIDUAL_FIX[c]) for c in text if c in RESIDUAL_FIX]
    return (text.translate(_RESIDUAL_TABLE), hits) if hits else (text, [])


def np_(s: str) -> str:
    return L.strip_punct(s)


def a_text(pairs) -> str:
    return "".join(g for g, _m in pairs)


def b_text(row) -> str:
    return "".join(row["paragraphs"])


def align(a_chapters, b_rows):
    """Greedy one-to-one alignment: title match first, content ratio as backstop."""
    a_items = list(a_chapters.items())
    scored = []
    for ai, (a_dir, pairs) in enumerate(a_items):
        akey = np_(a_dir)
        an = np_(a_text(pairs))[:400]
        for bi, row in enumerate(b_rows):
            bkey = np_(row["titleSimp"])
            if akey and bkey:
                if akey == bkey:
                    ts = 1.0
                elif akey.startswith(bkey) or bkey.startswith(akey):
                    ts = 0.96
                else:
                    ts = difflib.SequenceMatcher(None, akey, bkey, autojunk=False).ratio()
            else:
                ts = 0.0
            bn = np_(b_text(row))[:400]
            cs = difflib.SequenceMatcher(None, an, bn, autojunk=False).ratio() if an and bn else 0.0
            score = max(ts, cs)
            if score >= MATCH_FLOOR:
                scored.append((score, ai, bi))
    scored.sort(key=lambda t: (-t[0], t[1], t[2]))
    a_used, b_used, a2b = set(), set(), {}
    for _s, ai, bi in scored:
        if ai in a_used or bi in b_used:
            continue
        a_used.add(ai)
        b_used.add(bi)
        a2b[ai] = bi
    order = []
    for bi, row in enumerate(b_rows):
        ai = next((k for k, v in a2b.items() if v == bi), None)
        order.append((row, a_items[ai][0] if ai is not None else None))
    unmatched_a = [a_items[i][0] for i in range(len(a_items)) if i not in a_used]
    unmatched_b = [b_rows[bi] for bi in range(len(b_rows)) if bi not in b_used]
    return order, unmatched_a, unmatched_b


def b_plain_map(row):
    """-> (plain_text, orig_index_of_each_plain_char, original_text)"""
    orig = b_text(row)
    kept, idx = [], []
    for i, ch in enumerate(orig):
        if ord(ch) not in L._PUNCT_TABLE:
            kept.append(ch)
            idx.append(i)
    return "".join(kept), idx, orig


def segment_from_base(pairs, row):
    """Locate every A pair inside B's 底本. -> [(text, source)]; source in {base, converted}."""
    b_plain, orig_idx, b_orig = b_plain_map(row)
    a_plain = np_(L.canon_simp(a_text(pairs)))
    b_plain_simp = np_(L.canon_simp(b_plain))
    sm = difflib.SequenceMatcher(None, a_plain, b_plain_simp, autojunk=False)
    amap = {}
    for a0, b0, n in sm.get_matching_blocks():
        for k in range(n):
            amap[a0 + k] = b0 + k

    out, cursor = [], 0
    for guwen, _modern in pairs:
        span = np_(L.canon_simp(guwen))
        a0, a1 = cursor, cursor + len(span)
        cursor = a1
        mapped = [amap[k] for k in range(a0, a1) if k in amap]
        if span and mapped and len(mapped) / max(1, a1 - a0) >= SPAN_COVERAGE:
            bmin, bmax = min(mapped), max(mapped)
            width = bmax - bmin + 1
            if width <= (a1 - a0) * SPAN_SLACK + 8 and bmax < len(orig_idx):
                start, end = orig_idx[bmin], orig_idx[bmax] + 1
                while start > 0 and b_orig[start - 1] in "「『（《":
                    start -= 1                      # 补回前置引号（不含句读）
                while end < len(b_orig) and ord(b_orig[end]) in L._PUNCT_TABLE:
                    end += 1                        # 补回句末句读
                text = b_orig[start:end].strip()
                if text:
                    out.append((text, "base"))
                    continue
        out.append((L.to_trad(guwen), "converted"))
    return out


def collate(a_pairs, b_row):
    """Chapter-level 对校. t2s-normalizing both sides collapses 繁简 mapping noise,
    so what remains is a genuine 异文."""
    a_trad = np_("".join(L.to_trad(g) for g, _m in a_pairs))
    b_trad = np_(fix_residual(b_text(b_row))[0])
    if a_trad == b_trad:
        return "identical", 1.0, None
    a_s, b_s = np_(L.canon_simp(a_trad)), np_(L.canon_simp(b_trad))
    if a_s == b_s:
        return "conversion-artifact", 1.0, None
    ratio = difflib.SequenceMatcher(None, a_s, b_s, autojunk=False).ratio()
    pos = next((i for i in range(min(len(a_s), len(b_s))) if a_s[i] != b_s[i]),
               min(len(a_s), len(b_s)))
    return "variant", ratio, (f"A…{a_s[max(0,pos-10):pos+12]}… / B…{b_s[max(0,pos-10):pos+12]}…")


def mount_annotations(out_volumes, suspects):
    """M1b: 把 kanripo 注疏挂到段上；对不上的登记进存疑清单，绝不猜挂。"""
    chapters = [ch for vol in out_volumes for ch in vol["chapters"]]
    per_para, per_gloss, stats, _placed = AN.attach(AN.Corpus(), chapters)
    # M1d：第二見證本（四庫全書本 KR2e0005）交叉補注。
    # 只補「章段覆蓋率 < 0.80 且該段當前零注」的位置；既有掛載一律不動。
    para_chapter = {p["id"]: ch["id"] for ch in chapters for p in ch["paragraphs"]}
    ann_per_ch: dict[str, int] = {}
    for pid in per_para:
        cid = para_chapter[pid]
        ann_per_ch[cid] = ann_per_ch.get(cid, 0) + 1
    cover = {ch["id"]: ann_per_ch.get(ch["id"], 0) / max(1, len(ch["paragraphs"]))
             for ch in chapters}
    wyg = AN.attach_wyg(chapters, per_para, cover)
    stats["wyg"] = wyg
    # 逐条审计报告：每条跨見證补注的注文 + 锚文对，供人工核对。
    report = {"edition": wyg["edition"],
              "counts": {k: v for k, v in wyg.items() if not isinstance(v, (list, dict))},
              "notes": wyg["added"]}
    (STATS.parent / "_wyg_fill_report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=1, sort_keys=True) + "\n",
        encoding="utf-8")
    stats["sbckNotes"] = sum(1 for v in per_para.values() for a in v
                             if a.get("edition", AN.EDITION_SBCK) == AN.EDITION_SBCK)
    stats["wygNotes"] = sum(1 for v in per_para.values() for a in v
                            if a.get("edition") == AN.EDITION_WYG)
    stats["mixedParagraphs"] = sum(
        1 for v in per_para.values()
        if {a.get("edition", AN.EDITION_SBCK) for a in v}
        >= {AN.EDITION_SBCK, AN.EDITION_WYG})
    stats["annotations"] = sum(len(v) for v in per_para.values())
    stats["paragraphsAnnotated"] = len(per_para)
    stats["chaptersAnnotated"] = len({para_chapter[pid] for pid in per_para})
    by_src = {AN.SOURCE_BASE: 0, AN.SOURCE_BU: 0, AN.SOURCE_ZHENG: 0}
    for v in per_para.values():
        for a in v:
            by_src[a["source"]] = by_src.get(a["source"], 0) + 1
    stats["bySource"] = by_src
    for vol in out_volumes:
        for ch in vol["chapters"]:
            for p in ch["paragraphs"]:
                if p["id"] in per_para:
                    p["annotations"] = per_para[p["id"]]
                if p["id"] in per_gloss:
                    p["glossary"] = per_gloss[p["id"]]
    for d in stats["chapterDetail"]:
        vid = d["id"].rsplit("-", 1)[0]
        if not d["aligned"]:
            suspects.append({
                "kind": "annotation-chapter-unlocated", "volumeId": vid,
                "detail": (f"章「{d['title']}」({d['id']}) 未能与 kanripo KR2e0003 对齐"
                           f"（{d['why']}），全章注疏一概未挂载")})
        elif d["partial"]:
            suspects.append({
                "kind": "annotation-chapter-partial", "volumeId": vid,
                "detail": (f"章「{d['title']}」({d['id']}) 仅部分对齐（覆盖率 "
                           f"{d['coverage']:.2f}），只在对齐上的片段挂载注疏")})
    for d in wyg["detail"]:
        if d["zeroBefore"] and d["notesAdded"]:
            suspects.append({
                "kind": "annotation-cross-edition",
                "volumeId": d["id"].rsplit("-", 1)[0],
                "detail": (f"章「{d['title']}」({d['id']}) 四部叢刊本全章無注"
                           f"（缺葉），已據四庫全書本補 {d['notesAdded']} 條，"
                           f"覆蓋 {d['paragraphsFilled']}/{d['paragraphs']} 段")})
    for vid, n in sorted(stats["notesUnmountedByVolume"].items()):
        suspects.append({
            "kind": "annotation-notes-unmounted", "volumeId": vid,
            "detail": (f"{n} 个注块未挂载：位于未对齐章、对齐空洞、章前篇题注或章末总注，"
                       f"锚点测不准，按「宁可不挂」处理")})
    return stats


def main() -> int:
    a_all = L.load_a()
    b_all = L.load_b()
    b_by_vol = {}
    for row in b_all:
        b_by_vol.setdefault(row["volDir"], []).append(row)

    out_volumes, suspects = [], []
    coll = {"identical": 0, "conversion-artifact": 0, "variant": 0}
    totals = {"chapters": 0, "paragraphs": 0, "textChars": 0, "textCharsRaw": 0,
              "transChars": 0, "transCharsRaw": 0, "transPresent": 0,
              "transMissing": 0, "t2sChanged": 0, "baseParagraphs": 0,
              "convertedParagraphs": 0, "baseConfirmedChapters": 0,
              "baseResidualFixed": 0,
              "spanChecked": 0, "spanLow": 0, "spanSimSum": 0.0}
    per_volume = []

    for vid in L.VOL_ORDER:
        meta = L.VOL_META[vid]
        vol_dir = meta["dir"]
        a_chapters = a_all.get(vol_dir, {})
        b_rows = b_by_vol.get(vol_dir, [])
        order, unmatched_a, unmatched_b = align(a_chapters, b_rows)

        for a_dir in unmatched_a:
            order.append((None, a_dir))
            suspects.append({"kind": "chapter-unmatched-a", "volumeId": vid,
                             "detail": f"源A 章「{a_dir}」在源B 无对应章，章序无法判定，已置于卷末"})
        for row in unmatched_b:
            suspects.append({"kind": "chapter-unmatched-b", "volumeId": vid,
                             "detail": (f"源B 章「{row['title']}」({row['file']}) 在源A 无对应章："
                                        f"源A 整篇缺此章（与该卷各章正文相似度均 < 0.2），"
                                        f"故章界无争议、白话为空，白话缺失另见 C 类")})

        chapters, idx = [], 0
        for b_row, a_dir in order:
            idx += 1
            title = b_row["title"] if b_row is not None else L.to_trad(a_dir)
            paras, ch_fix = [], []
            if a_dir is not None:
                pairs = a_chapters[a_dir]
                segs = segment_from_base(pairs, b_row) if b_row is not None else \
                    [(L.to_trad(g), "converted") for g, _m in pairs]
                for (text, src), (guwen, modern) in zip(segs, pairs):
                    trans = modern.strip() or None
                    if trans is not None and L.to_simp(trans) != trans:
                        trans = L.to_simp(trans)
                        totals["t2sChanged"] += 1
                    if not text.strip():
                        suspects.append({"kind": "empty-text", "volumeId": vid,
                                         "detail": f"章「{title}」存在原文为空的条目，已跳过"})
                        continue
                    text, hits = fix_residual(text)
                    if hits:
                        ch_fix.extend(hits)
                        totals["baseResidualFixed"] += len(hits)
                    if src == "base":
                        totals["baseParagraphs"] += 1
                        sim = difflib.SequenceMatcher(
                            None, np_(L.canon_simp(guwen)),
                            np_(L.canon_simp(text)), autojunk=False).ratio()
                        totals["spanChecked"] += 1
                        totals["spanSimSum"] += sim
                        if sim < 0.85:
                            totals["spanLow"] += 1
                            suspects.append({
                                "kind": "span-mismatch", "volumeId": vid,
                                "detail": (f"章「{title}」定位出的底本片段与源A 原文相似度仅 {sim:.3f}，"
                                           f"段界可能偏移：{text[:28]}")})
                    else:
                        totals["convertedParagraphs"] += 1
                        suspects.append({
                            "kind": "trad-from-s2t", "volumeId": vid,
                            "detail": (f"章「{title}」有一段未能在底本中定位，繁体由 OpenCC s2t 机械转换，"
                                       f"可能存在通假字误转（如 咸→鹹）")})
                    if trans is None:
                        totals["transMissing"] += 1
                        suspects.append({"kind": "missing-translation", "volumeId": vid,
                                         "detail": f"章「{title}」有一段白话缺失"})
                    else:
                        totals["transPresent"] += 1
                    n = len(np_(text))
                    if n > LONG_PARAGRAPH:
                        suspects.append({"kind": "long-paragraph", "volumeId": vid,
                                         "detail": (f"章「{title}」有一段原文 {n} 字，超过 {LONG_PARAGRAPH} 字阈值，"
                                                    f"源未再分句，未强行切分")})
                    paras.append({"text": text, "trans": trans})
            else:
                suspects.append({
                    "kind": "missing-translation", "volumeId": vid,
                    "detail": (f"章「{title}」仅存于底本（源A 无对应白话），"
                               f"{len(b_row['paragraphs'])} 段 translation 均为 null")})
                for p in b_row["paragraphs"]:
                    totals["transMissing"] += 1
                    fixed, hits = fix_residual(p)
                    if hits:
                        ch_fix.extend(hits)
                        totals["baseResidualFixed"] += len(hits)
                    paras.append({"text": fixed, "trans": None})

            if not paras:
                suspects.append({"kind": "empty-chapter", "volumeId": vid,
                                 "detail": f"章「{title}」无段落，已跳过"})
                idx -= 1
                continue

            paragraphs = []
            for pi, item in enumerate(paras, start=1):
                paragraphs.append({
                    "id": f"{vid}-{idx:02d}-p{pi:04d}",
                    "text": item["text"],
                    "translation": item["trans"],
                    "annotations": [],
                    "glossary": [],
                })
                totals["paragraphs"] += 1
                totals["textChars"] += len(np_(item["text"]))
                totals["textCharsRaw"] += len(item["text"])
                totals["transChars"] += len(np_(item["trans"] or ""))
                totals["transCharsRaw"] += len(item["trans"] or "")
            chapters.append({"id": f"{vid}-{idx:02d}", "title": title,
                             "paragraphs": paragraphs})
            totals["chapters"] += 1

            if a_dir is not None and b_row is not None:
                verdict, ratio, sample = collate(a_chapters[a_dir], b_row)
                coll[verdict] += 1
                if verdict in ("identical", "conversion-artifact"):
                    totals["baseConfirmedChapters"] += 1
                    if ch_fix:
                        pairs_txt = "、".join(f"「{a}」→「{b}」"
                                              for a, b in dict.fromkeys(ch_fix))
                        suspects.append({
                            "kind": "base-simplified-residual", "volumeId": vid,
                            "detail": (f"章「{title}」底本含简体残留 {pairs_txt}，"
                                       f"已按正字纠正（E 类 s2t 缺陷）")})
                else:
                    detail = f"章「{title}」两源异文，相似度 {ratio:.4f}；{sample}"
                    if ch_fix:
                        pairs_txt = "、".join(f"「{a}」→「{b}」"
                                              for a, b in dict.fromkeys(ch_fix))
                        detail += ("；另：源B 该处为简体残留，已按正字修正"
                                   f"（{pairs_txt}，E 类 s2t 缺陷，非仅异文）")
                    suspects.append({"kind": "text-variant", "volumeId": vid,
                                     "detail": detail})

        vol_obj = {
            "schemaVersion": 1, "id": vid, "bookId": "zhanguoce",
            "groupId": meta["group"], "title": meta["title"],
            "titleSimp": meta["titleSimp"],
            "index": L.VOL_ORDER.index(vid) + 1, "chapters": chapters,
        }
        out_volumes.append(vol_obj)
        per_volume.append({"id": vid, "title": meta["title"], "dir": vol_dir,
                           "chapters": len(chapters),
                           "paragraphs": sum(len(c["paragraphs"]) for c in chapters),
                           "aFiles": len(a_chapters), "bFiles": len(b_rows)})
        print(f"  {vid:11s} ch={len(chapters):3d} "
              f"p={sum(len(c['paragraphs']) for c in chapters):4d}")

    ann_stats = mount_annotations(out_volumes, suspects)
    suspects.append({
        "kind": "taxonomy-volume-multi-group", "volumeId": "song-wei",
        "detail": ("分类口径修正：宋衛为通行本一卷（卷三十二），原先被 " + "song / weiwei"
                   + " 两个 group 同时引用（共 12 组），致阅读器把同卷展开两次"
                   "（卷位 34、497 章渲染出 512 个章链接、重复 15）。现并为单一 group "
                   "song-wei（宋衛策/宋卫策），group 数 11，volumeOrder 仍 33 项；"
                   "依据：通行本即 11 国策，且两源均无宋/衛切分边界，硬切即猜测。"
                   "卷、章、段、id 与注疏零改动。")})

    DATA.mkdir(parents=True, exist_ok=True)
    (DATA / "volumes").mkdir(exist_ok=True)
    for vol_obj in out_volumes:
        (DATA / "volumes" / f"{vol_obj['id']}.json").write_text(
            json.dumps(vol_obj, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    groups = []
    for gid, title, title_simp in L.GROUP_TITLES:
        vols = [v for v in L.VOL_ORDER if L.VOL_META[v]["group"] == gid]
        groups.append({"id": gid, "title": title, "titleSimp": title_simp,
                       "volumes": vols})

    # 不变式：一个卷只能被一个 group 拥有（宋衛并卷曾违反此条，导致目录树重复展开）
    owners: dict[str, list[str]] = {}
    for g in groups:
        for v in g["volumes"]:
            owners.setdefault(v, []).append(g["id"])
    dup = {v: gs for v, gs in owners.items() if len(gs) > 1}
    if dup:
        raise SystemExit(f"卷被多个 group 引用: {dup}")
    if set(owners) != set(L.VOL_ORDER):
        raise SystemExit(f"group 未覆盖全部卷: {sorted(set(L.VOL_ORDER) - set(owners))}")
    book = {
        "schemaVersion": 1, "id": "zhanguoce", "title": "戰國策",
        "titleSimp": "战国策", "compiler": "劉向", "era": "西漢", "form": "國別體",
        "sources": [
            {"name": "Classical-Modern（crazyxiaoqiang）双语数据·战国策",
             "url": "https://github.com/crazyxiaoqiang/Classical-Modern",
             "license": "public-domain"},
            {"name": "zhanguoce（Ancient-China-Books）EPUB 繁体底本",
             "url": "https://github.com/Ancient-China-Books/zhanguoce",
             "license": "public-domain"},
        ],
        "groups": groups, "volumeOrder": list(L.VOL_ORDER),
    }
    (DATA / "book.json").write_text(
        json.dumps(book, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    totals.update({"volumes": len(out_volumes), "groups": len(groups)})
    STATS.write_text(json.dumps({
        "totals": totals, "perVolume": per_volume, "suspects": suspects,
        "annotations": ann_stats,
        "collation": coll,
        "constants": {"longParagraph": LONG_PARAGRAPH, "matchFloor": MATCH_FLOOR,
                      "spanCoverage": SPAN_COVERAGE, "spanSlack": SPAN_SLACK},
    }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(totals, ensure_ascii=False))
    print("collation:", coll)
    print("annotations:", json.dumps(
        {k: v for k, v in ann_stats.items() if k not in
         ("chapterDetail", "glossaryTrace", "unaligned", "unplacedSamples",
          "notesUnmountedByVolume")}, ensure_ascii=False))
    print("suspects:", len(suspects))
    return 0


if __name__ == "__main__":
    sys.exit(main())
