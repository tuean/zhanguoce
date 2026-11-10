# -*- coding: utf-8 -*-
"""M4 生成器：把 497 章正文拆成「游說輪次 + 論證手段」。

只用規則，不用 AI（Q5 = C）。輸出 data/zhanguoce/strategy/chapters.json。

兩步：
1. 輪次：找「謂」。謂前是說話人、謂後到「曰」之間是受話人；兩側都用
   entities/people.json 的最長別名回填。回填不上就寫 null，並在
   unresolvedTurns 記下原因——**不猜**。
2. 手段：用 tools/strategy_lexicon.py 的詞表掃描，每條命中寫
   {kind, p, offset, matched, len}；offset 是段落內的**字符**偏移（非位元組），
   與 docs/ENTITIES.md 的 o 同一個坐標系。

偏移一律由 len(text[:offset]) 直接算出，不經正則 group index 換算，避免位元組／字符混淆。
"""

from __future__ import annotations

import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import strategy_lexicon as lex  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
BOOK_DIR = ROOT / "data" / "zhanguoce"
OUT_FILE = BOOK_DIR / "strategy" / "chapters.json"

SUPPORTED_SCHEMA_VERSION = 1

# 「謂」前若為這些字，構成 所謂／何謂／不謂／可謂／無謂／是謂——都不是「對誰說」。
NOT_A_TURN_PREFIX = set("所何不可無是")

# 受話人區間裡出現這些字符，說明「謂…曰」之間不是一個人名（是句子）。
BOUNDARY = set("，。：！？「」『』；、\n　 ")

# 「謂」到「曰」的最大跨度：實測最長的人名＋爵稱不超過 10 字，放寬到 14 只為切分，
# 真正的名字長度另有限制（≤10）。
MAX_ADDRESSEE = 10
MAX_SPAN = 14

ANON_PREFIX = set("或客")


def load_book() -> dict:
    return json.loads((BOOK_DIR / "book.json").read_text(encoding="utf-8"))


def load_chapters() -> list[dict]:
    """Reading order from book.json (groups → volumes), never re-sorted."""
    book = load_book()
    order = book.get("volumeOrder") or []
    if not order:
        order = [p.stem for p in sorted((BOOK_DIR / "volumes").glob("*.json"))]
    chapters: list[dict] = []
    for volume_id in order:
        path = BOOK_DIR / "volumes" / f"{volume_id}.json"
        volume = json.loads(path.read_text(encoding="utf-8"))
        for chapter in volume["chapters"]:
            chapters.append(
                {
                    "volumeId": volume["id"],
                    "groupId": volume.get("groupId", ""),
                    "chapterId": chapter["id"],
                    "title": chapter["title"],
                    "paragraphs": chapter["paragraphs"],
                }
            )
    return chapters


def load_forms() -> list[tuple[str, str, str]]:
    """(form, entityId, entityName), longest form first so 秦昭王 beats 昭王."""
    people = json.loads((BOOK_DIR / "entities" / "people.json").read_text(encoding="utf-8"))
    forms: list[tuple[str, str, str]] = []
    for entity in people["entities"]:
        for form in [entity["name"], *entity.get("aliases", [])]:
            forms.append((form, entity["id"], entity["name"]))
    forms.sort(key=lambda item: (-len(item[0]), item[0]))
    return forms


def match_ending_at(text: str, index: int, forms: list[tuple[str, str, str]]):
    """Longest entity form whose last character is text[index-1] (i.e. ends at 謂)."""
    for form, entity_id, name in forms:
        start = index - len(form)
        if start >= 0 and text[start:index] == form:
            return form, entity_id, name, start
    return None


def extract_turns(chapter: dict, forms: list[tuple[str, str, str]]):
    turns: list[dict] = []
    unresolved: list[dict] = []
    for paragraph in chapter["paragraphs"]:
        text = paragraph["text"]
        for match in re.finditer("謂", text):
            index = match.start()
            if index > 0 and text[index - 1] in NOT_A_TURN_PREFIX:
                continue
            end = text.find("曰", index + 1, index + 1 + MAX_SPAN)
            if end < 0:
                continue
            region = text[index + 1 : end]
            if not region or len(region) > MAX_ADDRESSEE:
                continue
            if any(char in BOUNDARY for char in region):
                continue

            speaker = match_ending_at(text, index, forms)
            addressee = next((item for item in forms if item[0] == region), None)
            if speaker is None and addressee is None:
                # 兩側都認不出：這多半不是「對誰說」的句子，也可能只是資料不足。
                # 這種情況下沒有可複核的陳述，整條丟棄（寧可不標）。
                continue

            start = speaker[3] if speaker else index
            turn = {
                "index": len(turns),
                "p": paragraph["id"],
                "speaker": speaker[0] if speaker else None,
                "speakerId": speaker[1] if speaker else None,
                "addressee": region,
                "addresseeId": addressee[1] if addressee else None,
                "marker": "謂",
                "offset": index,
                "text": text[start : end + 1],
            }
            turns.append(turn)

            # 認不出的一方要留痕，而不是靜靜留白。
            if speaker is None:
                prefix = text[index - 1] if index > 0 else ""
                reason = "speaker-anonymous" if prefix in ANON_PREFIX else "speaker-not-in-entities"
                unresolved.append(
                    {
                        "chapterId": chapter["chapterId"],
                        "p": paragraph["id"],
                        "turnIndex": turn["index"],
                        "field": "speaker",
                        "reason": reason,
                    }
                )
            if addressee is None:
                unresolved.append(
                    {
                        "chapterId": chapter["chapterId"],
                        "p": paragraph["id"],
                        "turnIndex": turn["index"],
                        "field": "addressee",
                        "reason": "addressee-not-in-entities",
                    }
                )
    return turns, unresolved


COMPILED = [(kind, re.compile(pattern), why) for kind, pattern, why in lex.patterns()]


def extract_devices(chapter: dict) -> list[dict]:
    devices: list[dict] = []
    for paragraph in chapter["paragraphs"]:
        text = paragraph["text"]
        hits: list[tuple[int, int, str, str]] = []
        for kind, regex, _why in COMPILED:
            for match in regex.finditer(text):
                matched = match.group(0)
                hits.append((match.start(), len(matched), kind, matched))
        for offset, length, kind, matched in lex.resolve_overlaps(hits):
            assert text[offset : offset + length] == matched
            devices.append(
                {
                    "kind": kind,
                    "p": paragraph["id"],
                    "offset": offset,
                    "matched": matched,
                    "len": length,
                }
            )
    return devices


def main() -> int:
    chapters = load_chapters()
    forms = load_forms()

    out_chapters: list[dict] = []
    all_unresolved: list[dict] = []
    for chapter in chapters:
        turns, unresolved = extract_turns(chapter, forms)
        devices = extract_devices(chapter)
        all_unresolved.extend(unresolved)
        out_chapters.append(
            {
                "chapterId": chapter["chapterId"],
                "turns": turns,
                "devices": devices,
                "stats": {"turns": len(turns), "devices": len(devices)},
            }
        )

    document = {
        "schemaVersion": SUPPORTED_SCHEMA_VERSION,
        "bookId": "zhanguoce",
        "generatedBy": "tools/build_strategy.py",
        "generatedFrom": {
            "chapters": len(chapters),
            "paragraphs": sum(len(chapter["paragraphs"]) for chapter in chapters),
        },
        "principle": "只描述原文可見的修辭手段與可指認的說話人／受話人；指認不出寫 null 並記原因，不推斷意圖，不評價高下。",
        "turnRule": "「謂」前為說話人、「謂」後至「曰」為受話人，皆以 entities/people.json 最長別名回填；回填不上寫 null 並記入 unresolvedTurns。",
        "deviceKinds": [
            {"kind": block["kind"], "rationale": block["rationale"]} for block in lex.LEXICON
        ],
        "chapters": out_chapters,
        "unresolvedTurns": all_unresolved,
    }

    OUT_FILE.parent.mkdir(parents=True, exist_ok=True)
    OUT_FILE.write_text(
        json.dumps(document, ensure_ascii=False, indent=1) + "\n", encoding="utf-8"
    )

    # ---- report (goes into docs/M4_NOTES.md verbatim) ----
    total = len(out_chapters)
    with_turns = [c for c in out_chapters if c["turns"]]
    with_devices = [c for c in out_chapters if c["devices"]]
    neither = [c for c in out_chapters if not c["turns"] and not c["devices"]]
    turns_total = sum(len(c["turns"]) for c in out_chapters)
    devices_total = sum(len(c["devices"]) for c in out_chapters)
    speaker_ok = sum(
        1 for c in out_chapters for t in c["turns"] if t["speaker"] is not None
    )
    addressee_ok = sum(
        1 for c in out_chapters for t in c["turns"] if t["addresseeId"] is not None
    )
    both_ok = sum(
        1
        for c in out_chapters
        for t in c["turns"]
        if t["speaker"] is not None and t["addresseeId"] is not None
    )
    kinds: dict[str, int] = {}
    for chapter in out_chapters:
        for device in chapter["devices"]:
            kinds[device["kind"]] = kinds.get(device["kind"], 0) + 1

    print(f"章 {total} · 段 {document['generatedFrom']['paragraphs']}")
    print(f"有輪次 {len(with_turns)} 章（{len(with_turns)/total:.1%}）")
    print(f"有手段 {len(with_devices)} 章（{len(with_devices)/total:.1%}）")
    print(f"兩者皆無 {len(neither)} 章（{len(neither)/total:.1%}）")
    print(f"turns {turns_total} · devices {devices_total}")
    print(f"  speaker 可指認 {speaker_ok} · addressee 可指認 {addressee_ok} · 雙方皆可 {both_ok}")
    print(f"  unresolvedTurns {len(all_unresolved)}")
    for kind in lex.kinds():
        print(f"  {kind}: {kinds.get(kind, 0)}")
    print(f"→ {OUT_FILE.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
