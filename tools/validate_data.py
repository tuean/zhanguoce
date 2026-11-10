#!/usr/bin/env python3
"""Validate data/zhanguoce/** against docs/SCHEMA.md.

Every rule from SCHEMA.md 硬性约束 is checked mechanically, plus the
cross-file invariants (global id uniqueness, book<->volume consistency,
group/volume membership, index ordering) and the formatting/idempotency
contract (no BOM, UTF-8, LF, 2-space indent, byte-identical rerun).

Exit code 0 = all checks pass.
"""
from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "data" / "zhanguoce"
VOLS = DATA / "volumes"
sys.path.insert(0, str(Path(__file__).resolve().parent))
import zgc_lib as L
import kanripo_lib as K

TAG_RE = re.compile(r"<[^>]+>")
# M1b 注疏（kanripo 四部丛刊本）落地契约；M1d 追加四庫全書本第二見證
ANN_SOURCES = ("鲍彪", "吴师道补", "吴师道正")
ANN_EDITIONS = ("四部叢刊", "四庫全書")
SBCK_NOTES_BASELINE = 8006   # M1d 之前的成品注數，禁止漂移
ANN_RESIDUE_RE = re.compile(r"[¶／/<>]|\u3000|\s|&[A-Za-z0-9#]+;")
GLOSS_TERM_BAN = set("不也而其此之所爲以於乎焉則且乃夫何若使可無有為")
CH_ID_RE = re.compile(r"^(?P<vol>[a-z]+(?:-[a-z]+)*-\d+)-\d{2}$")
VOL_ID_RE = re.compile(r"^(?P<group>[a-z]+(?:-[a-z]+)*)(?:-(?P<n>\d+))?$")

failures: list[str] = []
checks = 0

# 繁体判定：以底本实际用字为准。OpenCC s2t 是逐字映射，对 咸陽/于/里 这类
# 一简对多繁的字会误转，因此不能拿 to_trad(text)==text 当判据。
# 这里把「底本用字表」当作繁体字库：凡出现在底本里的字，按繁体保留不算错。
def base_traditional_vocab() -> set:
    vocab = set()
    try:
        for row in L.load_b():
            for para in row.get("paragraphs", []):
                vocab.update(para)
    except Exception:
        return set()
    return vocab


TRAD_VOCAB = base_traditional_vocab()



def check(cond: bool, label: str) -> bool:
    global checks
    checks += 1
    if cond:
        print(f"PASS  {label}")
    else:
        print(f"FAIL  {label}")
        failures.append(label)
    return cond


def kanripo_flat(prefix: str = "KR2e0003", skip_names: tuple[str, ...] = (),
                 noise_re=None) -> str:
    """見證本去版式后的连续文 stream：該本注文应逐字落在其中。

    只去掉版式层：叶码、¶、原双行折行 /、全角空格、换行、以及承载注文边界的括号。
    WYG 本另需剔掉卷首四庫署名行（prefix=KR2e0005）。
    """
    import re as _re
    cache = Path(__file__).resolve().parent / "_cache" / "kanripo"
    chunks = []
    for path in sorted(cache.glob(f"{prefix}_0*.txt")):
        if any(path.name.startswith(s) for s in skip_names):
            continue
        lines = [ln for ln in path.read_text(encoding="utf-8").split("\n")
                 if not ln.startswith("#") and not (noise_re and noise_re.match(ln))]
        chunks.append("\n".join(lines))
    flat = _re.sub(r"<pb:[^>]*>", "", "\n".join(chunks))
    return flat.translate({ord(c): None for c in "/\u3000 \t\n()¶"})


def wyg_flat() -> str:
    return kanripo_flat("KR2e0005",
                        tuple(f"KR2e0005_{i:03d}" for i in sorted(K.WYG_SKIP)),
                        K.WYG_NOISE_LINE_RE)


def raw_bytes(path: Path) -> bytes:
    return path.read_bytes()


def main() -> int:
    book_path = DATA / "book.json"
    if not check(book_path.is_file(), "book.json 存在"):
        return 1
    book_raw = raw_bytes(book_path)
    book = json.loads(book_raw.decode("utf-8"))

    # ---- formatting contract (SCHEMA 硬性约束 5) ----
    check(not book_raw.startswith(b"\xef\xbb\xbf"), "book.json 无 BOM")
    check(b"\r\n" not in book_raw, "book.json 为 LF 换行")
    check(book_raw.decode("utf-8").rstrip("\n") + "\n" ==
          json.dumps(book, ensure_ascii=False, indent=2) + "\n",
          "book.json 为 2 空格缩进且键序稳定")

    # ---- book fields ----
    check(book.get("schemaVersion") == 1, "book.schemaVersion == 1")
    check(book.get("id") == "zhanguoce", "book.id == zhanguoce")
    for key in ("title", "titleSimp", "compiler", "era", "form"):
        check(bool(book.get(key)), f"book.{key} 非空")
    check(book.get("title") == "戰國策", "book.title == 戰國策（繁体）")
    check(book.get("form") == "國別體", "book.form == 國別體")
    check(isinstance(book.get("sources"), list) and book["sources"], "book.sources 非空")

    groups = book.get("groups") or []
    order = book.get("volumeOrder") or []
    check(len(groups) == 11, f"11 个 group（实际 {len(groups)}）")
    check(len(order) == 33, f"33 卷在 volumeOrder（实际 {len(order)}）")
    check(len(set(order)) == len(order), "volumeOrder 无重复")
    check(len(set(g["id"] for g in groups)) == len(groups), "group.id 唯一")

    group_ids = {g["id"] for g in groups}
    group_of = {g["id"]: g["volumes"] for g in groups}
    listed = [v for g in groups for v in g["volumes"]]
    check(set(listed) == set(order), "所有 group.volumes 均出现在 volumeOrder 且无遗漏")
    check(all(v in order for v in listed), "group.volumes 引用均存在于 volumeOrder")
    owners: dict[str, int] = {}
    for g in groups:
        for v in g["volumes"]:
            owners[v] = owners.get(v, 0) + 1
    shared = sorted(v for v, n in owners.items() if n > 1)
    check(not shared,
          f"任何卷不得被多个 group 引用（重复归属 {len(shared)} 卷：{' '.join(shared) or '无'}）")

    files = sorted(p.stem for p in VOLS.glob("*.json"))
    check(len(files) == 33, f"卷文件 33 个（实际 {len(files)}）")
    check(set(files) == set(order), "卷文件名集合与 volumeOrder 完全一致")

    # ---- load + per-file checks ----
    all_chapter_ids: dict[str, str] = {}
    all_para_ids: dict[str, str] = {}
    stats = {"chapters": 0, "paragraphs": 0, "textChars": 0, "transChars": 0,
             "withTrans": 0, "baseConfirmed": 0, "ann": 0, "annPara": 0,
             "annChapter": 0, "gloss": 0, "glossPara": 0}
    trailing = []
    loaded = []
    for vid in order:
        path = VOLS / f"{vid}.json"
        raw = raw_bytes(path)
        vol = json.loads(raw.decode("utf-8"))
        loaded.append(vol)
        if raw.startswith(b"\xef\xbb\xbf") or b"\r\n" in raw or \
                raw.decode("utf-8").rstrip("\n") + "\n" != json.dumps(vol, ensure_ascii=False, indent=2) + "\n":
            trailing.append(vid)
        if vol.get("schemaVersion") != 1:
            failures.append(f"{vid}: schemaVersion")
        if vol.get("id") != vid:
            failures.append(f"{vid}: id 与文件名不符")
        if vol.get("bookId") != "zhanguoce":
            failures.append(f"{vid}: bookId")
        if vol.get("index") != order.index(vid) + 1:
            failures.append(f"{vid}: index 与 volumeOrder 位置不符")
        if vol.get("groupId") not in group_ids:
            failures.append(f"{vid}: groupId 不在 book.groups 中")
        elif vid not in group_of.get(vol.get("groupId"), []):
            failures.append(f"{vid}: groupId 与 book.groups[*].volumes 不一致")
        if not vol.get("title") or not vol.get("titleSimp"):
            failures.append(f"{vid}: title/titleSimp 为空")
        if L.to_trad(vol.get("titleSimp", "")) != vol.get("title", ""):
            failures.append(f"{vid}: title 非 titleSimp 的繁体对应")
        chapters = vol.get("chapters") or []
        if not chapters:
            failures.append(f"{vid}: chapters 为空")
        for ci, ch in enumerate(chapters, start=1):
            stats["chapters"] += 1
            cid = ch.get("id", "")
            if cid != f"{vid}-{ci:02d}":
                failures.append(f"{vid}: 章 id 序号异常 {cid}")
            if cid in all_chapter_ids:
                failures.append(f"章 id 重复: {cid}")
            all_chapter_ids[cid] = vid
            if not ch.get("title"):
                failures.append(f"{cid}: title 为空")
            if TAG_RE.search(ch.get("title", "")):
                failures.append(f"{cid}: title 含 HTML 标签")
            paras = ch.get("paragraphs") or []
            if not paras:
                failures.append(f"{cid}: paragraphs 为空")
            if any(p_.get("annotations") for p_ in paras):
                stats["annChapter"] += 1
            for pi, p in enumerate(paras, start=1):
                stats["paragraphs"] += 1
                pid = p.get("id", "")
                if pid != f"{cid}-p{pi:04d}":
                    failures.append(f"{cid}: 段 id 序号异常 {pid}")
                if pid in all_para_ids:
                    failures.append(f"段 id 重复: {pid}")
                all_para_ids[pid] = cid
                text = p.get("text")
                if not isinstance(text, str) or not text.strip():
                    failures.append(f"{pid}: text 为空")
                    continue
                stats["textChars"] += len(text)
                if TAG_RE.search(text):
                    failures.append(f"{pid}: text 含 HTML 标签")
                if TRAD_VOCAB:
                    simp_chars = {c for c in text
                                  if L.to_trad(c) != c and c not in TRAD_VOCAB}
                    if simp_chars:
                        failures.append(
                            f"{pid}: text 含简体专用字 {' '.join(sorted(simp_chars))}")
                else:
                    changed = sum(1 for c in text if L.to_trad(c) != c)
                    if changed / max(1, len(text)) > 0.05:
                        failures.append(f"{pid}: text 疑似非繁体（{changed} 字可转）")
                tr = p.get("translation")
                if tr is not None:
                    if not isinstance(tr, str) or not tr.strip():
                        failures.append(f"{pid}: translation 为空串（应为 null）")
                    else:
                        stats["withTrans"] += 1
                        stats["transChars"] += len(tr)
                        if TAG_RE.search(tr):
                            failures.append(f"{pid}: translation 含 HTML 标签")
                        if L.to_simp(tr) != tr:
                            failures.append(f"{pid}: translation 非简体")
                anns = p.get("annotations")
                gloss = p.get("glossary")
                for key, val in (("annotations", anns), ("glossary", gloss)):
                    if not isinstance(val, list):
                        failures.append(f"{pid}: {key} 不是数组")
                    elif any((not isinstance(x, dict)) or not x for x in val):
                        failures.append(f"{pid}: {key} 含空对象")
                if isinstance(anns, list):
                    for a in anns:
                        stats["ann"] += 1
                        if not isinstance(a, dict) or \
                                set(a) not in ({"source", "text"},
                                               {"source", "text", "edition"}):
                            failures.append(
                                f"{pid}: annotation 字段应恰为 source/text[/edition]")
                            continue
                        ed = a.get("edition", "四部叢刊")
                        if ed not in ANN_EDITIONS:
                            failures.append(f"{pid}: annotation.edition 非法 {ed!r}")
                        src, txt = a.get("source"), a.get("text", "")
                        if src not in ANN_SOURCES:
                            failures.append(f"{pid}: annotation.source 非法 {src!r}")
                        if not isinstance(txt, str) or not txt.strip():
                            failures.append(f"{pid}: annotation.text 为空")
                        elif txt != txt.strip():
                            failures.append(f"{pid}: annotation.text 含首尾空白")
                        elif TAG_RE.search(txt) or ANN_RESIDUE_RE.search(txt):
                            failures.append(
                                f"{pid}: annotation.text 含版式残留或 HTML：{txt[:16]}")
                    if anns:
                        stats["annPara"] += 1
                if isinstance(gloss, list):
                    for g in gloss:
                        stats["gloss"] += 1
                        if not all(k in g for k in ("term", "pinyin", "gloss")):
                            failures.append(f"{pid}: glossary 缺字段")
                            continue
                        term, gl = g.get("term"), g.get("gloss")
                        if not isinstance(term, str) or not (1 <= len(term) <= 2):
                            failures.append(f"{pid}: glossary.term 应为 1-2 字 {term!r}")
                        elif term in GLOSS_TERM_BAN:
                            failures.append(f"{pid}: glossary.term 是虚词 {term!r}")
                        if not isinstance(gl, str) or not gl.endswith("也") or len(gl) > 8:
                            failures.append(f"{pid}: glossary.gloss 应形如「…也」 {gl!r}")
                        if g.get("pinyin") is not None and not isinstance(g.get("pinyin"), str):
                            failures.append(f"{pid}: glossary.pinyin 应为 null 或字符串")
                        if not anns:
                            failures.append(f"{pid}: glossary 所在段没有注疏，无法溯源")
                    if gloss:
                        stats["glossPara"] += 1
                if anns:
                    stats["annChapter"] = stats["annChapter"]  # 章级计数在下一层
                for key in p:
                    if key not in ("id", "text", "translation", "annotations", "glossary"):
                        failures.append(f"{pid}: 多余字段 {key}")

    check(not trailing, f"全部卷文件 BOM/LF/缩进/键序合规（异常 {len(trailing)}）")
    check(all(p["text"].strip() for vol in loaded
              for ch in vol["chapters"] for p in ch["paragraphs"]),
          "全部段 text 非空")
    check(not any(("古文：" in p["text"] or "现代文：" in p["text"] or "古文:" in p["text"])
                  for vol in loaded for ch in vol["chapters"] for p in ch["paragraphs"]),
          "段 text 不含源格式残留标记")
    dup = 0
    for vol in loaded:
        for ch in vol["chapters"]:
            texts = [p["text"] for p in ch["paragraphs"]]
            dup += sum(1 for a, b in zip(texts, texts[1:]) if a == b)
    check(dup == 0, f"章内无重复相邻段（重复 {dup}）")
    try:
        import build_data
        residual = build_data.RESIDUAL_FIX
    except Exception:
        residual = {}
    hits = [(p["id"], c) for vol in loaded for ch in vol["chapters"]
            for p in ch["paragraphs"] for c in p["text"] if c in residual]
    check(not hits, f"无底本简体残留（命中 {len(hits)}：{hits[:3]}）")
    check(all([c["id"] for c in vol["chapters"]] ==
              [f"{vol['id']}-{i:02d}" for i in range(1, len(vol["chapters"]) + 1)]
              for vol in loaded), "各卷章 id 连续且有序")
    check(all([p["id"] for p in ch["paragraphs"]] ==
              [f"{ch['id']}-p{i:04d}" for i in range(1, len(ch["paragraphs"]) + 1)]
              for vol in loaded for ch in vol["chapters"]), "各章段 id 连续且有序")
    check(all(vol["index"] == i for vol in loaded for i in [order.index(vol["id"]) + 1]),
          "各卷 index 与 volumeOrder 位置一致")
    check(all(group_of[vol["groupId"]] for vol in loaded), "每卷都能在所属 group 中找到")
    check(len(all_para_ids) == stats["paragraphs"], "段 id 全局唯一")
    check(len(all_chapter_ids) == stats["chapters"], "章 id 全局唯一")
    check(not any(f.startswith(("dongzhou-0",)) and "章 id 序号" in f for f in failures),
          "章 id 形如 <vol>-NN")
    check(stats["ann"] > 0, f"注疏已入库（{stats['ann']} 条）")
    check(stats["gloss"] > 0, f"训释已入库（{stats['gloss']} 条）")
    check(stats["glossPara"] <= stats["annPara"], "训释只落在带注路段上")

    # ---- M1b: 注文逐字可回底本（防编造） ----
    flat = kanripo_flat()
    all_anns = [a for vol in loaded for ch in vol["chapters"]
                for p in ch["paragraphs"] for a in (p.get("annotations") or [])]
    ann_texts = [a["text"] for a in all_anns
                 if a.get("edition", "四部叢刊") == "四部叢刊"]
    wyg_texts = [a["text"] for a in all_anns if a.get("edition") == "四庫全書"]
    missing = [t for t in ann_texts if t not in flat]
    check(bool(ann_texts) and not missing,
          f"四部叢刊注文可在 KR2e0003 逐字命中（{len(ann_texts)} 条，未命中 {len(missing)}）")
    wmissing = [t for t in wyg_texts if t not in wyg_flat()]
    check(bool(wyg_texts) and not wmissing,
          f"四庫全書注文可在 KR2e0005 逐字命中（{len(wyg_texts)} 条，未命中 {len(wmissing)}）")

    # ---- M1c: 注疏计数自洽（防统计与成品漂移） ----
    ann_stats = json.loads((ROOT / "tools" / "_build_stats.json")
                           .read_text(encoding="utf-8"))["annotations"]
    check(len(all_anns) == ann_stats["annotations"],
          f"注疏条数与构建统计一致（{len(all_anns)} 条）")
    by_src = {}
    for vol in loaded:
        for ch in vol["chapters"]:
            for p in ch["paragraphs"]:
                for a in (p.get("annotations") or []):
                    by_src[a["source"]] = by_src.get(a["source"], 0) + 1
    check(by_src == ann_stats["bySource"],
          "注疏来源计数与构建统计一致（" +
          " / ".join(f"{k} {v}" for k, v in sorted(by_src.items())) + "）")
    check(stats["annPara"] == ann_stats["paragraphsAnnotated"],
          f"带注段计数与构建统计一致（{stats['annPara']} 段）")
    check(stats["annChapter"] == ann_stats["chaptersAnnotated"],
          f"带注章计数与构建统计一致（{stats['annChapter']} 章）")
    detail_ann = sum(c["annotated"] for c in ann_stats["chapterDetail"])
    check(detail_ann == ann_stats["notesAttached"],
          f"章级注块明细求和与挂载数一致（{detail_ann} = {ann_stats['notesAttached']}）")
    check(ann_stats["notesAttached"] + ann_stats["notesUnplaced"]
          + ann_stats["notesOutsideAligned"] == ann_stats["notes"],
          "注块守恒：挂载 + 未挂载 + 区间外 = 解析总数"
          f"（{ann_stats['notesAttached']} + {ann_stats['notesUnplaced']} + "
          f"{ann_stats['notesOutsideAligned']} = {ann_stats['notes']}）")
    check(ann_stats["notesDuplicateMount"] == 0,
          f"无注块被挂载两次（重复 {ann_stats['notesDuplicateMount']}）")
    # 短注（如「未詳」「見下」）同段出现两次是两个独立注块，故只查 >=10 字的长注
    dup_ann = []
    for vol in loaded:
        for ch in vol["chapters"]:
            for p in ch["paragraphs"]:
                seen = set()
                for a in (p.get("annotations") or []):
                    key = (a["source"], a["text"])
                    if len(a["text"]) >= 10 and key in seen:
                        dup_ann.append(p["id"])
                    seen.add(key)
    check(not dup_ann, f"同段内无重复长注（重复 {len(dup_ann)}：{dup_ann[:3]}）")
    unaligned_ann = [c["id"] for c in ann_stats["chapterDetail"]
                     if c["annotated"] and not c["aligned"]]
    check(not unaligned_ann,
          f"带注章均已完成定位（未定位却有注 {len(unaligned_ann)}：{unaligned_ann[:3]}）")

    # ---- M1d: 跨見證本補注契約（四庫全書本只補零注段） ----
    check(ann_stats["sbckNotes"] == SBCK_NOTES_BASELINE,
          f"四部叢刊注数保持 {SBCK_NOTES_BASELINE}（实际 {ann_stats['sbckNotes']}）")
    check(len(ann_texts) == ann_stats["sbckNotes"],
          f"成品四部叢刊注数 = 统计（{len(ann_texts)}）")
    check(len(wyg_texts) == ann_stats["wygNotes"] == ann_stats["wyg"]["notesAdded"],
          f"成品四庫全書注数 = 统计（{len(wyg_texts)}）")
    check(stats["ann"] == ann_stats["sbckNotes"] + ann_stats["wygNotes"],
          f"注数合计 = 两见证之和（{stats['ann']}）")
    check(ann_stats["mixedParagraphs"] == 0,
          f"无同段混排两见证（混排 {ann_stats['mixedParagraphs']} 段）")
    mixed = [p["id"] for vol in loaded for ch in vol["chapters"] for p in ch["paragraphs"]
             if {a.get("edition", "四部叢刊") for a in (p.get("annotations") or [])}
             >= {"四部叢刊", "四庫全書"}]
    check(not mixed, f"成品无混排段（{len(mixed)}）")
    wyg_detail = {d["id"]: d for d in ann_stats["wyg"]["detail"]}
    wyg_where = [ch["id"] for vol in loaded for ch in vol["chapters"]
                 for p in ch["paragraphs"]
                 if any(a.get("edition") == "四庫全書"
                        for a in (p.get("annotations") or []))]
    bad_scope = sorted({cid for cid in wyg_where
                        if wyg_detail.get(cid, {}).get("inScope") is not True})
    check(not bad_scope,
          f"四庫全書注只落在 in-scope 章（越界 {len(bad_scope)}：{bad_scope[:3]}）")
    zero_left = [d["id"] for d in ann_stats["wyg"]["detail"]
                 if d["zeroBefore"] and not d["notesAdded"]]
    check(not zero_left,
          f"四部叢刊本全章无注的章均已补注（未补 {len(zero_left)}：{zero_left[:3]}）")
    report = json.loads((ROOT / "tools" / "_wyg_fill_report.json").read_text(encoding="utf-8"))
    rep_notes = report["notes"]
    weak = [n for n in rep_notes if n["anchorRatio"] < 0.60
            or not n["editionAnchor"] or not n["corpusAnchor"]]
    check(len(rep_notes) == ann_stats["wygNotes"] and not weak,
          f"补注审计报告逐条覆盖 {ann_stats['wygNotes']} 条、锚文成对且相似度 >= 0.60"
          f"（记录 {len(rep_notes)}，不合格 {len(weak)}）")

    # ---- idempotency: rerun the pipeline, compare bytes ----
    before = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
              for p in [book_path, *sorted(VOLS.glob("*.json"))]}
    r = subprocess.run([sys.executable, str(ROOT / "tools" / "build_data.py")],
                       capture_output=True, text=True, cwd=str(ROOT))
    after = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
             for p in [book_path, *sorted(VOLS.glob("*.json"))]}
    check(r.returncode == 0, "重建管线退出码 0")
    check(before == after, "幂等：重建后全部文件字节一致")

    print()
    print(f"CHECKED  {checks} 条断言；FAIL {len(failures)} 条")
    print(f"TOTALS   卷 {len(order)} / 组 {len(groups)} / 章 {stats['chapters']} / "
          f"段 {stats['paragraphs']}")
    print(f"CHARS    原文 {stats['textChars']} 字 / 白话 {stats['transChars']} 字")
    cov = 100.0 * stats["withTrans"] / stats["paragraphs"] if stats["paragraphs"] else 0
    print(f"COVERAGE 白话覆盖 {stats['withTrans']}/{stats['paragraphs']} = {cov:.4f}%")
    acov = 100.0 * stats["annChapter"] / stats["chapters"] if stats["chapters"] else 0
    print(f"ANNOT    注疏 {stats['ann']} 条 / 带注段 {stats['annPara']} / "
          f"带注章 {stats['annChapter']} = {acov:.4f}% / 训释 {stats['gloss']} 条")
    if failures:
        print("\nFAILURES:")
        seen = set()
        for f in failures:
            if f in seen:
                continue
            seen.add(f)
            print("  -", f)
        return 1
    print("\nALL CHECKS PASSED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
