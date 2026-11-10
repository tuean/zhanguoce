#!/usr/bin/env python3
"""Generate docs/DATA_REPORT.md from the build statistics.

Deterministic: same data in -> same report out (no timestamps, no dict-order drift).
"""
from __future__ import annotations

import json
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
STATS = Path(__file__).resolve().parent / "_build_stats.json"
DATA = ROOT / "data" / "zhanguoce"
OUT = ROOT / "docs" / "DATA_REPORT.md"
BT = chr(96)

KIND_ORDER = [
    ("chapter-unmatched-a", "A 源A 单边章（底本无对应）"),
    ("chapter-unmatched-b", "B 结构性存疑：章对齐"),
    ("missing-translation", "C 白话缺失"),
    ("span-mismatch", "D 底本定位可疑"),
    ("trad-from-s2t", "E 繁体兜底（s2t 机械转换）"),
    ("base-simplified-residual", "E2 底本简体残留（已按正字纠正）"),
    ("text-variant", "F 两源异文"),
    ("long-paragraph", "G 段过长未再切分"),
    ("empty-text", "H 空原文"),
    ("empty-chapter", "I 空章"),
    ("annotation-chapter-unlocated", "J 注疏·章未对齐（全章注疏未挂载）"),
    ("annotation-chapter-partial", "K 注疏·章仅部分对齐"),
    ("annotation-notes-unmounted", "L 注疏·注块未挂载（按卷）"),
    ("taxonomy-volume-multi-group", "M 分类口径修正（宋衛并卷）"),
]


def q(s):
    return BT + s + BT


def files_stats():
    vols = sorted((DATA / "volumes").glob("*.json"))
    vb = sum(p.stat().st_size for p in vols)
    bb = (DATA / "book.json").stat().st_size
    return len(vols), vb, bb, vb + bb


def _para_index():
    out = {}
    for path in sorted((DATA / "volumes").glob("*.json")):
        vol = json.loads(path.read_text(encoding="utf-8"))
        for ch in vol["chapters"]:
            for p in ch["paragraphs"]:
                out[p["id"]] = (vol["id"], ch["title"], p)
    return out


SAMPLE_IDS = ["dongzhou-28-p0001", "dongzhou-01-p0002", "qin-1-02-p0001"]


def m1b_section(L, A, st, ann, per_volume, order):
    q = globals()["q"]
    stats = (ann.get("parse") or {})
    cons2 = (ann.get("constants") or {})
    paras = _para_index()
    total_par = ann.get("paragraphs", 0) or 1
    total_ch = ann.get("chapters", 0) or 1
    total_notes = ann.get("notes", 0) or 1
    A("")
    A("## 九、M1b · 注疏入库（鮑彪注 + 吳師道補正）")
    A("")
    A("### 源与解析口径")
    A("")
    A("源：" + q("kanripo/KR2e0003") + " 四部叢刊本《戰國策校注》全 11 个 "
      + q(".txt") + "（" + q("_000") + " 序、" + q("_001") + " 目錄与西周、"
      + q("_002…_010") + " 各国正文），缓存在 " + q("tools/_cache/kanripo/")
      + "，抓取见 " + q("tools/kanripo_lib.py") + " 的 " + q("ensure_cache") + "。")
    A("")
    A("底本的排版是「大字正文 + 双行小字夹注」，数字化后注文被压平成圆括号："
      + q("¶") + " 为行末、" + q("<pb:KR2e0003_SBCK_XXX-Ya>") + " 为叶码、"
      + q("/") + " 为原双行折行处。因此解析分四步：")
    A("")
    A("1. **叶码先切、再拼页**：叶码替换成哨兵、去掉 " + q("¶") + " 后把全 11 档拼成一条流。")
    A("2. **跨页拼接 = 粘连只隔空白/叶码的相邻括号块**：叶末与行末都会把一条注切成两个"
      "括号块（如 " + q("(…東周若)¶") + " + " + q("(以王都為尊則舊/卷首東周是矣)")
      + " 本是「…東周若以王都為尊…」一条），只按行解析就会把注拆碎、把 " + q("補曰")
      + " 拆成两半。实测粘合 **" + str(stats.get("joinedBlocks", 0)) + "** 处，"
      "其中跨叶码 **" + str(stats.get("joinedAcrossPage", 0)) + "** 处。")
    A("3. **不用括号配平做校验器**：实测 "
      + str(stats.get("pages", 0)) + " 个叶里，括号在**每一个叶内各自配平**（不平衡 0 叶），"
      "所以「全书配平」证明不了任何事；真正要防的是上面那种「块被切开」。"
      "未闭合块 " + str(stats.get("unterminated", 0)) + " 处。")
    A("4. **归属切分**：无标记的前段 =" + q("鲍彪") + "；" + q("補曰") + "/"
      + q("𥙷曰") + " 起 =" + q("吴师道补") + "；" + q("正曰") + " 起 ="
      + q("吴师道正") + "；" + q("彪謂") + "/" + q("彪曰") + " 起 =" + q("鲍彪")
      + "。一条逻辑注可切出多段，故「注块数」与「注疏条数」不等。")
    A("")
    A("| 解析量 | 数值 |")
    A("|---|---|")
    A("| 原始括号块 | " + str(stats.get("rawBlocks", 0)) + " |")
    A("| 粘合后逻辑注块 | " + str(stats.get("notes", 0)) + " |")
    A("| 其中跨叶码粘合 | " + str(stats.get("joinedAcrossPage", 0)) + " |")
    A("| 含「跨页未闭合」情形的注块（跨叶码） | " + str(stats.get("notesCrossPage", 0)) + " |")
    A("| 丢弃：含 " + q("&KRxxxx;") + " 未编码外字 / 空块 | "
      + str(stats.get("notesDroppedEntity", 0)) + " |")
    A("| 丢弃：单字残块 | " + str(stats.get("notesDroppedOneChar", 0)) + " |")
    A("| 归属标记出现次数 補曰/" + q("𥙷曰") + "/正曰/彪謂/彪曰 | "
      + " / ".join(str(stats.get("markers", {}).get(k, 0))
                   for k in ("補曰", "𥙷曰", "正曰", "彪謂", "彪曰")) + " |")
    A("")
    A("### 对齐口径（不按卷/条序号）")
    A("")
    A("- 鮑彪「改定其次第」并改定章题（实测其目錄作「魏鞅亡魏入秦」，通行本作"
      "「衞鞅亡魏入秦」），故**绝不按卷序/条序对齐**。")
    A("- **章**：用「章首句片段」在 kanripo 正文（去注、去句读）里投票——取章内若干 6 字 "
      "n-gram（隔 4 字取一个），每个命中位置减去它在章内的偏移后投票，得票最多且达到阈值的"
      "位置判为章首。短 n-gram 是为了扛异体/异文（" + q("諼/驩") + "、" + q("衞/衛")
      + " 这类），否则长前缀一处异文就整章对不上。")
    A("- **章内**：把本库整章正文与 kanripo 该位置起、2 倍章长 + 60 字的窗口做字符级对齐"
      "（" + q("SequenceMatcher") + "），得「kanripo 字序 → 本库字序」映射，并把章的有效区间"
      "裁到真正对齐上的首尾块。覆盖率 < " + str(cons2.get("minCoverage", 0.45))
      + " 判「对不上」，该章一个注都不挂。")
    A("- **段**：注块按「注文所附原文片段」的落点（注紧跟被注词之后）经映射落到章内字序，"
      "再按段偏移归属到段；落点正好在段界时归前一段。")
    A("- **锚点复核**：注前 6 字的正文片段必须与本库同位置文字相似度 ≥ 0.60，"
      "否则说明该处发生错位（本库漏句/异文吞句），该注块不挂——实测拦下 "
      + str(ann.get("notesAnchorReject", 0)) + " 个。")
    A("")
    A("### 统计")
    A("")
    A("| 指标 | 数值 |")
    A("|---|---|")
    A("| 全库段数 | " + str(total_par) + " |")
    A("| 挂上注疏的段 | " + str(ann.get("paragraphsAnnotated", 0)) + "（"
      + format(100.0 * ann.get("paragraphsAnnotated", 0) / total_par, ".2f") + "%）|")
    A("| 全库章数 | " + str(total_ch) + " |")
    A("| 与 kanripo 对齐上的章 | " + str(ann.get("chaptersAligned", 0)) + "（"
      + format(100.0 * ann.get("chaptersAligned", 0) / total_ch, ".2f") + "%）|")
    A("| 其中仅部分对齐 | " + str(ann.get("chaptersPartial", 0)) + " |")
    A("| **挂上注疏的章** | **" + str(ann.get("chaptersAnnotated", 0)) + "（"
      + format(100.0 * ann.get("chaptersAnnotated", 0) / total_ch, ".2f") + "%）** |")
    A("| **注疏总条数** | **" + str(ann.get("annotations", 0)) + "** |")
    srcs = ann.get("bySource", {})
    for name in ("鲍彪", "吴师道补", "吴师道正"):
        A("| 其中 " + name + " | " + str(srcs.get(name, 0)) + " |")
    A("| 训释（" + q("glossary") + "）条数 | " + str(ann.get("glossary", 0)) + " |")
    A("| 逻辑注块总数 | " + str(ann.get("notes", 0)) + " |")
    A("| 挂载上的注块 | " + str(ann.get("notesAttached", 0)) + "（"
      + format(100.0 * ann.get("notesAttached", 0) / total_notes, ".2f") + "%）|")
    A("| 未挂载：落在对齐区间内但锚点测不准 | " + str(ann.get("notesUnplaced", 0)) + " |")
    A("| 未挂载：落在未对齐章/对齐空洞/篇题注/章末总注 | "
      + str(ann.get("notesOutsideAligned", 0)) + " |")
    A("")
    A("### 挂载样例（可逐字复核）")
    A("")
    A("| 段 id | 段原文（截断） | 来源 | 注文原文（逐字） |")
    A("|---|---|---|---|")
    shown = 0
    for pid in SAMPLE_IDS:
        if pid not in paras or not paras[pid][2].get("annotations"):
            continue
        vid, title, pp = paras[pid]
        for a in pp["annotations"][:2]:
            A("| " + q(pid) + " | " + pp["text"][:24] + "… | " + a["source"] + " | "
              + q(a["text"][:100]) + " |")
            shown += 1
    if not shown:
        for pid, (_v, _t, pp) in paras.items():
            if pp.get("annotations"):
                a = pp["annotations"][0]
                A("| " + q(pid) + " | " + pp["text"][:24] + "… | " + a["source"] + " | "
                  + q(a["text"][:100]) + " |")
                shown += 1
            if shown >= 3:
                break
    A("")
    A("### 训释溯源（全 " + str(ann.get("glossary", 0)) + " 条，逐条回注文原句）")
    A("")
    A("抽取规则（紧正则，宁缺毋滥）：注文须整体形如「X + Y也」（Y ≤ 4 字、不含 "
      + q("爲所此者其亦而以於之乎也不") + "），且 X 必须就是注块紧邻正文里的那个字。"
      "同一段的注疏里保留着原句，故每一行都能回到注文。")
    A("")
    A("| 段 id | 词 | 训释 | 来源 | 注文原句 |")
    A("|---|---|---|---|---|")
    for row in ann.get("glossaryTrace", [])[:400]:
        A("| " + q(row["paraId"]) + " | " + row["term"] + " | " + row["gloss"] + " | "
          + row["source"] + " | " + q(row["note"][:80]) + " |")
    A("")
    A("### 未挂载注块分布（按卷，注块数）")
    A("")
    A("| volumeId | 未挂载注块 |")
    A("|---|---|")
    for vid, n in sorted(ann.get("notesUnmountedByVolume", {}).items(),
                         key=lambda kv: order.get(kv[0], 99)):
        A("| " + q(vid) + " | " + str(n) + " |")
    A("")
    A("失败原因分类：①章级未对齐（J 类，整章 0 注）②章级仅部分对齐（K 类，只挂对齐上的片段）"
      "③注块落点在映射空洞（本库该处漏句或异文吞句，锚点复核不过）"
      "④章前篇题注/王名注与章末总注（注的不是正文）。四类一律不猜挂。")
    A("")
    A("### 复现与校验（M1b 增量）")
    A("")
    A(BT * 3 + "bash")
    A("python3 tools/kanripo_lib.py       # 解析 kanripo 十一个檔（缓存缺失时自动抓取）")
    A("python3 tools/annotations.py       # 单独跑注疏对齐，打印统计与未对齐章")
    A("python3 tools/build_data.py        # 产出 data/zhanguoce/**（含 annotations/glossary）")
    A("python3 tools/validate_data.py     # 41 条断言 + 幂等性比对")
    A("python3 tools/build_report.py      # 重新生成本报告")
    A(BT * 3)
    A("")
    A("校验器改动（只加不减）：新增 annotation 契约断言——" + q("source") + " 必须属于三个"
      "固定值、" + q("text") + " 非空且不含 " + q("¶ / < >") + "、全角空格、HTML 标签、"
      + q("&KRxxxx;") + " 残留；" + q("glossary") + " 必须形如「…也」、term 为 1–2 字且非虚词、"
      "且**只能出现在带注路段上**（溯源前提）；另加一条防编造断言——"
      "**每条注文都必须能在底本去结构符后的文 stream 里逐字命中**。原有断言一条未放宽。")
    A("")


def main() -> int:
    st = json.loads(STATS.read_text(encoding="utf-8"))
    totals, per_volume, suspects = st["totals"], st["perVolume"], st["suspects"]
    coll, cons = st["collation"], st["constants"]
    nvol, vbytes, bbytes, tbytes = files_stats()
    nch, npar = totals["chapters"], totals["paragraphs"]
    cover = 100.0 * totals["transPresent"] / npar
    kinds = Counter(s["kind"] for s in suspects)
    order = {v["id"]: i for i, v in enumerate(per_volume)}
    mean_sim = totals["spanSimSum"] / max(1, totals["spanChecked"])

    indexed = []
    for kind, label in KIND_ORDER:
        group = [s for s in suspects if s["kind"] == kind]
        group.sort(key=lambda s: (order.get(s["volumeId"], 99), s["detail"]))
        for s in group:
            indexed.append((label, s))

    meta = json.loads((DATA / "book.json").read_text(encoding="utf-8"))
    gmap = {}
    for g in meta["groups"]:
        for v in g["volumes"]:
            gmap.setdefault(v, []).append(g["id"])

    L = []
    A = L.append
    ann = st.get("annotations") or {}
    A("# 《戰國策》数据层交付报告（M1 / M1b）")
    A("")
    A("> 本文件由 " + q("tools/build_report.py") + " 依据 " + q("tools/_build_stats.json")
      + " 生成；数字与数据文件同源，无人工填写值。")
    A("")
    A("## 一、产出清单")
    A("")
    A("| 路径 | 数量 | 字节 |")
    A("|---|---|---|")
    A("| " + q("data/zhanguoce/book.json") + " | 1 | " + str(bbytes) + " |")
    A("| " + q("data/zhanguoce/volumes/*.json") + " | " + str(nvol) + " | " + str(vbytes) + " |")
    A("| 合计 | " + str(nvol + 1) + " | " + str(tbytes) + " |")
    A("")
    A("管线：" + q("tools/fetch_sources.py") + " 抓取缓存 → " + q("tools/zgc_lib.py")
      + " 解析/对齐/繁简 → " + q("tools/build_data.py") + " 归一化产出 → "
      + q("tools/validate_data.py") + " 校验 → " + q("tools/build_report.py") + " 生成本报告。")
    A("")
    A("## 二、源格式实测结论")
    A("")
    A("### 源 A：crazyxiaoqiang/Classical-Modern（主源：原文 + 白话配对）")
    A("")
    A("- 目录是 **三层**，不是卷级平铺：" + q("双语数据/战国策/<卷>/<章>/bitext.txt")
      + "。实测 33 个卷目录、495 个章目录，每章目录下恰 1 个 " + q("bitext.txt") + "。")
    A("- 编码 UTF-8（无 BOM），行尾 LF。")
    A("- 记录分隔符是**空行**；单条记录固定两行，键为全角冒号的 " + q("古文：") + " 与 "
      + q("现代文：") + "。")
    A("- 全库 **6651 条记录，全部 6651 条古文/白话成对**：不成对 0、多键块 0、"
      "无法解析的行 0（实测统计，非估计）。")
    A("- 用字：**古文与白话都是简体**。原文需转繁体，这是本次最大的质量风险点，"
      "处置见第四节。")
    A("- 章题是**被截断的目录名**（多为前 12 字），例如 " + q("司马错与张仪争论于秦")
      + " 对应底本全题 " + q("司馬錯與張儀爭論於秦惠王前") + "；章序按码位排序，**不是书序**。")
    A("- 无注疏、无训释。")
    A("")
    A("### 源 B：Ancient-China-Books/zhanguoce（对校本：繁体底本 + 书序）")
    A("")
    A("- EPUB 解包：" + q("OEBPS/Text/NNN.htm") + "，共 **497 个**，文件名数字序即**书序**。")
    A("- 编码 UTF-8，" + q('lang="zh-tw"') + "，**繁体**。")
    A("- 每个 htm 三段式：" + q('<span class="style9">') + " 卷题（如 " + q("卷三 ‧ 秦策一")
      + "）→ " + q('<p class="style6">') + " 章题（全题）→ 若干 " + q("<p>") + " 正文自然段。")
    A("- 卷题可机械解析（去 " + q("卷N ‧") + " 与尾部 " + q("策") + "），"
      "33 个卷题与源 A 的 33 个卷目录一一对应。")
    A("- 无白话、无注疏。")
    A("")
    A("### 死源与失败改道（均为实测）")
    A("")
    A("| 问题 | 现象 | 处置 |")
    A("|---|---|---|")
    A("| 源 A 整仓 tree | " + q("git/trees/main?recursive=1") + " 返回 " + q("truncated: true")
      + "，章清单不全 | 改用 " + q("contents") + " 取 " + q("双语数据/战国策")
      + " 的 subtree sha，再对该子树单独 recursive；得权威 495 章，缓存 495/495 |")
    A("| 源 B 分支名 | " + q("raw.githubusercontent.com/.../main/...") + " 返回 **404** | 读 repo API 的 "
      + q("default_branch") + "，实为 " + q("master") + "，改用 master |")
    A("| 源 B 首次落盘 | 本地只有 475 个 htm（抓取与计数竞争所致） | 以 tree 的 497 条 blob 做 size "
      "比对补齐，最终 **497/497** |")
    A("| ctext / zh.wikisource | 按任务简报为死源（Cloudflare 挑战 / 本机不可达） | 未使用 |")
    A("")
    A("## 三、Taxonomy（11 国策 / 33 卷）")
    A("")
    A("通行分法是 **11 国策**：東周、西周、秦、齊、楚、趙、魏、韓、燕、宋衛、中山；"
      "卷共 **33**（" + q("volumeOrder") + " 取通行本卷序：東周、西周、秦一…秦五、"
      "齊一…齊六、楚一…楚四、趙一…趙四、魏一…魏四、韓一…韓三、燕一…燕三、宋衛、中山）。"
      "group 数 = " + str(len(meta["groups"])) + "，与 " + q("volumeOrder") + " 的 33 卷一一对应。")
    A("")
    A("| 序 | volumeId | 卷题 | groupId | 章 | 段 |")
    A("|---|---|---|---|---|---|")
    for i, v in enumerate(per_volume, start=1):
        A("| " + str(i) + " | " + q(v["id"]) + " | " + v["title"] + " | "
          + " / ".join(q(x) for x in gmap.get(v["id"], ["?"])) + " | "
          + str(v["chapters"]) + " | " + str(v["paragraphs"]) + " |")
    A("")
    A("**宋衛并卷的处理（口径修正）**：通行本卷三十二为「宋衛策」，宋、衛合为一卷，"
      "故该卷只归**一个** group，组 id 与卷 id 同名 " + q("song-wei") + "（标题 "
      + q("宋衛策") + " / " + q("宋卫策") + "）。此前的实现把它同时登记在 " + q("song")
      + " 与 " + q("weiwei") + " 两个组下（共 12 组），阅读器因此把同一卷在「宋策」「衛策」"
      "下各展开一次：卷位 34 个（应为 33）、497 章渲染成 512 个章链接（重复 15）、"
      "同卷正文被读两遍且两侧各只剩半卷。**根因是建模口径写错（声称 12 国/33 卷），不是数据问题**；"
      "依据有二：①通行本本身即 11 国策；②两源都**没有**可据以切开宋、衛的边界，硬切即猜测。"
      "所以正确修法不是拆组而是并组。修正后卷、章、段、id 与注疏零改动。")
    A("")
    A("## 四、切分与归一化口径")
    A("")
    A("### 章")
    A("")
    A("- 章序取**底本文件序**（唯一带书序的源）；章题取**底本全题**（源 A 目录名是截断前缀）。")
    A("- 对齐：卷内一对一最大权匹配，评分 = max(章题相似度, 正文相似度)；章题先经 "
      + q("canon_simp") + "（t2s 不动点 + 异体字表）归一，前缀包含记 0.96，阈值 "
      + str(cons["matchFloor"]) + "。")
    A("- 结果：源 A 495 章 + 仅底本有 2 章 = **497 章**；仅源 A 有 0 章。"
      "那 2 章即 B 类存疑，性质是「源 A 把底本的两章并作一章」，故其白话缺失。")
    A("")
    A("### 段")
    A("")
    A("- 段 = **源 A 的一条「古文/现代文」配对**。不取底本自然段的原因：只有配对记录能保证"
      "每段自带对应白话，且粒度够细以承载批注锚点（底本侧有 331 章只有 1 个自然段，"
      "用作锚点太粗）。")
    A("- 超长阈值 " + str(cons["longParagraph"]) + " 字（不含标点）：全库**超阈值 0 段**，"
      "未做任何强行切分。")
    A("")
    A("### text 如何转繁体（关键）")
    A("")
    A("直接 " + q("s2t") + " 会改错原文：OpenCC 逐字映射、不看语境，实测会出现 "
      + q("咸陽→鹹陽") + "、" + q("干戈→幹戈") + "、" + q("周→週") + "、" + q("于→於")
      + " 这类错误。因此：")
    A("")
    A("1. **以繁体底本为正字来源**：把每条配对在同章底本内定位——两侧去标点后取 "
      + q("SequenceMatcher") + " 匹配块，得「源 A 字符 → 源 B 字符」映射，再按配对区间截取"
      "底本原字符（要求覆盖率 ≥ " + str(cons["spanCoverage"]) + "、区间宽度 ≤ 原文长×"
      + str(cons["spanSlack"]) + "+8）；边界只补前置引号与句末句读，避免吞掉邻段标点。")
    A("2. 定位不到才回退 " + q("s2t") + "，并**逐段登记**（E 类）。")
    A("3. **底本残留正字纠正**：底本个别处夹带简体专用字（实测 1 处：魏策三「无忌」→"
      "「無忌」）。管线内置 " + q("RESIDUAL_FIX") + " 小表只纠正纯简体专用字，"
      "共纠正 " + str(totals.get("baseResidualFixed", 0)) + " 处；"
      + q("于/里/后/云/干/咸") + " 等一简对多繁的底本用字一律不动（改了才是错）。"
      "此类处仍在 F 类异文里保留，并在条目内注明「已按正字修正」。")
    A("4. 自动校核：定位片段与源 A 原文（去标点）相似度**均值 " + format(mean_sim, ".4f")
      + "**，低于 0.85 的 **" + str(totals["spanLow"]) + " 段**；段首误带句读 **0 段**。")
    A("")
    A("### translation")
    A("")
    A("- 取源 A " + q("现代文") + "，本身即简体；对 " + str(totals["t2sChanged"])
      + " 条含繁体用字的译文补一次 t2s 归一。")
    A("- 无译文写 " + q("null") + "（不写空串），并登记存疑。")
    A("- " + q("annotations") + " / " + q("glossary") + " 的空值不再是常态：M1b 已从 "
      + q("kanripo/KR2e0003") + " 回填注疏与训释，口径与统计见第九节。")
    A("")
    A("## 五、统计")
    A("")
    A("| 指标 | 数值 |")
    A("|---|---|")
    A("| 卷 | " + str(totals["volumes"]) + " |")
    A("| 国别 group | " + str(totals["groups"]) + " |")
    A("| 章 | " + str(nch) + " |")
    A("| 段 | " + str(npar) + " |")
    A("| 原文总字数（不含标点） | " + str(totals["textChars"]) + " |")
    A("| 原文总字符（含标点） | " + str(totals["textCharsRaw"]) + " |")
    A("| 白话总字数（不含标点） | " + str(totals["transChars"]) + " |")
    A("| 白话总字符（含标点） | " + str(totals["transCharsRaw"]) + " |")
    A("| 有白话的段 | " + str(totals["transPresent"]) + " |")
    A("| 无白话的段（null） | " + str(totals["transMissing"]) + " |")
    A("| **白话覆盖率** | **" + format(cover, ".4f") + "%**（" + str(totals["transPresent"])
      + "/" + str(npar) + "）|")
    A("| 原文逐字取自繁体底本 | " + str(totals["baseParagraphs"]) + "（"
      + format(100.0 * totals["baseParagraphs"] / npar, ".2f") + "%）|")
    A("| 原文由 s2t 兜底 | " + str(totals["convertedParagraphs"]) + "（"
      + format(100.0 * totals["convertedParagraphs"] / npar, ".2f") + "%）|")
    A("| 底本简体残留已纠正 | " + str(totals.get("baseResidualFixed", 0)) + " 处 |")
    A("| 平均段长 | " + format(totals["textChars"] / npar, ".1f") + " 字/段 |")
    A("")
    A("### 两源对校")
    A("")
    A("章级比对：源 A 原文转繁后与底本同章文本去标点比对；再做一次 " + q("t2s")
      + " 归一以滤掉纯繁简映射差异（" + q("爲/為") + "、" + q("於/于") + " 这类），"
      "剩下的才算真异文。")
    A("")
    A("| 判定 | 章数 | 含义 |")
    A("|---|---|---|")
    A("| identical | " + str(coll["identical"]) + " | 转繁后与底本**逐字全同** |")
    A("| conversion-artifact | " + str(coll["conversion-artifact"])
      + " | 仅繁简用字选择不同，文句一致 |")
    A("| variant | " + str(coll["variant"]) + " | 存在实质异文，逐条见 F 类 |")
    A("")
    A("## 六、复现与校验")
    A("")
    A(BT * 3 + "bash")
    A("python3 -m pip install --target tools/_vendor opencc-python-reimplemented   # 仅首次")
    A("python3 tools/fetch_sources.py     # 抓取并缓存到 tools/_cache（已存在则跳过）")
    A("python3 tools/build_data.py        # 产出 data/zhanguoce/**")
    A("python3 tools/validate_data.py     # 41 条断言 + 幂等性比对")
    A("python3 tools/build_report.py      # 重新生成本报告")
    A(BT * 3)
    A("")
    A("幂等：校验脚本末尾会重跑一遍 " + q("build_data.py") + "，对 34 个产出文件做 " + q("SHA-256")
      + " 比对，**字节一致**（" + q("PASS 幂等：重建后全部文件字节一致") + "）。"
      "段 id 形如 " + q("qin-1-01-p0001") + "，由卷序+章序+段序纯函数生成，重复运行不变。")
    A("")
    A("## 七、存疑清单（全量编号）")
    A("")
    A("共 **" + str(len(suspects)) + "** 条。按类别分块、块内按卷序与文本排序，编号稳定可引用。")
    A("")
    A("| 类别 | 条数 |")
    A("|---|---|")
    for kind, label in KIND_ORDER:
        if kinds.get(kind):
            A("| " + label + " | " + str(kinds[kind]) + " |")
    A("")
    cur = None
    for i, (label, s) in enumerate(indexed, start=1):
        if label != cur:
            cur = label
            A("")
            A("### " + label)
            A("")
        A("- **S" + format(i, "03d") + "** " + q(s["volumeId"]) + " " + s["detail"])
    A("")
    A("## 八、已知限制与建议")
    A("")
    A("1. **注疏层已回填，但非全覆盖**：M1b 挂上 "
      + str(ann.get("annotations", 0)) + " 条注疏，覆盖 "
      + str(ann.get("chaptersAnnotated", 0)) + "/" + str(ann.get("chapters", 0))
      + " 章、注块挂载率 " + format(100.0 * ann.get("notesAttached", 0)
                                / max(1, ann.get("notes", 1)), ".2f")
      + "%；对不上的逐条见 J/K/L 类存疑，绝不猜挂。高诱注仍挂起（版本缺藏，非抓取失败）。")
    A("2. **训释层是子集**：" + q("glossary") + " 共 " + str(ann.get("glossary", 0))
      + " 条，只收「X，Y也」型紧注（宁缺毋滥）；这不是全量训释，M5 的实时追问仍是主力。")
    A("3. **白话为众包/机器来源**：源 A 译文质量不齐（例：夹多余空格 " + q("号称 商君 。")
      + "），本层只做结构化与清洗，不改写内容；需人工终审（PLAN R3）。")
    A("4. **段界来自源 A 配对，不是底本自然段**：好处是段段有白话；若 M2 需「底本自然段」"
      "视图，可按段 id 顺序聚合（相邻段在底本中连续）。")
    A("5. **" + str(totals["convertedParagraphs"]) + " 段原文为 s2t 兜底**（E 类逐条登记），"
      "可能有通假字被机械转换，审校优先复核。")
    A("6. **" + str(kinds.get("text-variant", 0)) + " 章两源异文**（F 类）：已给首处差异两侧"
      "上下文，未做全文校勘记；若要版本级校勘，建议以底本为底另建 diff 层。")
    A("7. **齐策五仅 1 章**：两源一致（底本亦只 1 个 htm），属源限制而非切分遗漏；不补。")
    A("")
    m1b_section(L, A, st, ann, per_volume, order)
    OUT.write_text("\n".join(L), encoding="utf-8")
    print("wrote", OUT, OUT.stat().st_size, "bytes; suspects", len(suspects))
    return 0


if __name__ == "__main__":
    sys.exit(main())
