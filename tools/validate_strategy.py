# -*- coding: utf-8 -*-
"""M4 校驗器：拿 volumes/ 的原文當唯一真值，逐條回驗 strategy/chapters.json。

設計原則與 entities/events 的校驗一致——**每條結論都要能在原文裡切片複核**。
所有偏移都是段落內的字符偏移（Unicode scalar），與 docs/ENTITIES.md 的 o 同一個坐標系。

用法：
    python3 tools/validate_strategy.py                 # 校驗預設檔案
    python3 tools/validate_strategy.py --strategy f.json
退出碼 0 = 全部通過；1 = 有一條斷言不成立（訊息含 chapterId/段 id/欄位）。
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BOOK_DIR = ROOT / "data" / "zhanguoce"
DEFAULT_STRATEGY = BOOK_DIR / "strategy" / "chapters.json"

SCHEMA_VERSION = 1
BOOK_ID = "zhanguoce"

REASONS = {
    "speaker-not-in-entities",
    "speaker-anonymous",
    "addressee-not-in-entities",
    "addressee-anonymous",
}


class Checker:
    def __init__(self) -> None:
        self.errors: list[str] = []

    def check(self, condition: bool, message: str) -> bool:
        if not condition:
            self.errors.append(message)
        return condition

    def fail(self, message: str) -> None:
        self.errors.append(message)


def load_json(path: pathlib.Path):
    return json.loads(path.read_text(encoding="utf-8"))


def load_truth():
    """章節順序、段 id、段文字、實體 id/form —— 全部取自原始資料。"""
    book = load_json(BOOK_DIR / "book.json")
    chapters: list[tuple[str, str, dict[str, str]]] = []
    seen_paragraphs: set[str] = set()
    for volume_id in book["volumeOrder"]:
        volume = load_json(BOOK_DIR / "volumes" / f"{volume_id}.json")
        for chapter in volume["chapters"]:
            paragraphs: dict[str, str] = {}
            for paragraph in chapter["paragraphs"]:
                assert paragraph["id"] not in seen_paragraphs, paragraph["id"]
                seen_paragraphs.add(paragraph["id"])
                paragraphs[paragraph["id"]] = paragraph["text"]
            chapters.append((chapter["id"], chapter["title"], paragraphs))
    people = load_json(BOOK_DIR / "entities" / "people.json")
    entity_ids: set[str] = set()
    entity_forms: set[str] = set()
    forms_by_id: dict[str, set[str]] = {}
    for entity in people["entities"]:
        entity_ids.add(entity["id"])
        forms = {entity["name"], *entity.get("aliases", [])}
        entity_forms |= forms
        forms_by_id.setdefault(entity["id"], set()).update(forms)
    return chapters, entity_ids, entity_forms, forms_by_id


def validate(strategy_path: pathlib.Path) -> list[str]:
    checker = Checker()
    chapters, entity_ids, entity_forms, forms_by_id = load_truth()
    chapter_order = [item[0] for item in chapters]
    chapter_titles = {item[0]: item[1] for item in chapters}
    chapter_paragraphs = {item[0]: item[2] for item in chapters}

    if not checker.check(strategy_path.is_file(), f"找不到檔案：{strategy_path}"):
        return checker.errors
    doc = load_json(strategy_path)

    checker.check(doc.get("schemaVersion") == SCHEMA_VERSION, f"schemaVersion 應為 {SCHEMA_VERSION}")
    checker.check(doc.get("bookId") == BOOK_ID, f"bookId 應為 {BOOK_ID}")

    declared_kinds = [item["kind"] for item in doc.get("deviceKinds") or []]
    checker.check(len(declared_kinds) == len(set(declared_kinds)), "deviceKinds 有重複")
    checker.check(len(declared_kinds) >= 6, "deviceKinds 少於六類")

    entries = doc.get("chapters")
    if not checker.check(isinstance(entries, list), "chapters 必須是陣列"):
        return checker.errors

    ids = [entry.get("chapterId") for entry in entries]
    checker.check(len(ids) == len(set(ids)), "chapterId 有重複")
    missing = set(chapter_order) - set(ids)
    unknown = set(ids) - set(chapter_order)
    checker.check(not missing, f"缺章：{sorted(missing)[:5]}")
    checker.check(not unknown, f"未知章：{sorted(unknown)[:5]}")
    checker.check(
        ids == [chapter_id for chapter_id in chapter_order if chapter_id in set(ids)],
        "chapters 必須依 book.json 的閱讀順序排列",
    )

    turn_total = 0
    device_total = 0
    unresolved_seen: list[tuple[str, int, str]] = []

    for entry in entries:
        chapter_id = entry.get("chapterId")
        where = f"{chapter_id}"
        if chapter_id not in chapter_paragraphs:
            continue
        paragraphs = chapter_paragraphs[chapter_id]
        paragraph_ids = list(paragraphs)

        turns = entry.get("turns") or []
        devices = entry.get("devices") or []
        stats = entry.get("stats") or {}
        checker.check(stats.get("turns") == len(turns), f"{where}: stats.turns 與實際不符")
        checker.check(stats.get("devices") == len(devices), f"{where}: stats.devices 與實際不符")

        # ---- turns ----
        seen_positions: set[tuple[str, int]] = set()
        order_keys: list[tuple[int, int]] = []
        for position, turn in enumerate(turns):
            label = f"{where} turn#{position}"
            checker.check(turn.get("index") == position, f"{label}: index 應為 {position}")
            paragraph_id = turn.get("p")
            if not checker.check(paragraph_id in paragraphs, f"{label}: 段 {paragraph_id} 不屬於本章"):
                continue
            text = paragraphs[paragraph_id]
            marker = turn.get("marker")
            offset = turn.get("offset")
            checker.check(isinstance(marker, str) and marker, f"{label}: marker 不可為空")
            if not checker.check(isinstance(offset, int) and offset >= 0, f"{label}: offset 非法"):
                continue
            end = offset + len(marker or "")
            checker.check(end <= len(text), f"{label}: offset+marker 越界（{end} > {len(text)}）")
            checker.check(text[offset:end] == marker, f"{label}: marker 切片不符：{text[offset:end]!r}")
            turn_text = turn.get("text")
            checker.check(isinstance(turn_text, str) and turn_text, f"{label}: text 不可為空")
            if isinstance(turn_text, str):
                checker.check(turn_text in text, f"{label}: text 不是原文子串")
                rel = text.find(turn_text)
                checker.check(
                    rel >= 0 and rel <= offset < rel + len(turn_text),
                    f"{label}: marker 未落在 text 片段內",
                )
            key = (paragraph_id, offset)
            checker.check(key not in seen_positions, f"{label}: 同一段同一偏移重複")
            seen_positions.add(key)
            order_keys.append((paragraph_ids.index(paragraph_id), offset))

            speaker = turn.get("speaker")
            speaker_id = turn.get("speakerId")
            addressee = turn.get("addressee")
            addressee_id = turn.get("addresseeId")
            for name, value in (
                ("speaker", speaker),
                ("speakerId", speaker_id),
                ("addressee", addressee),
                ("addresseeId", addressee_id),
            ):
                checker.check(value is None or (isinstance(value, str) and value.strip()), f"{label}: {name} 不可為空字串")
            checker.check(
                (speaker is None) == (speaker_id is None), f"{label}: speaker 與 speakerId 必須同時為 null"
            )
            # addressee 允許保留原文裡的稱謂（如「楚王」）而 addresseeId 為 null——
            # 文字是原文可複核的，id 認不出就留空並記 unresolved，這不算猜。
            checker.check(
                addressee is not None or addressee_id is None,
                f"{label}: addressee 為 null 時 addresseeId 必須為 null",
            )
            if speaker_id is not None:
                checker.check(speaker_id in entity_ids, f"{label}: speakerId {speaker_id} 不存在")
                if speaker is not None:
                    checker.check(
                        speaker in forms_by_id.get(speaker_id, set()),
                        f"{label}: speaker「{speaker}」不是 {speaker_id} 的別名",
                    )
                    checker.check(speaker in text, f"{label}: speaker「{speaker}」不在原文")
            if addressee_id is not None:
                checker.check(addressee_id in entity_ids, f"{label}: addresseeId {addressee_id} 不存在")
                if addressee is not None:
                    checker.check(
                        addressee in forms_by_id.get(addressee_id, set()),
                        f"{label}: addressee「{addressee}」不是 {addressee_id} 的別名",
                    )
            if addressee is not None and addressee_id is None:
                checker.check(addressee in text, f"{label}: 未回填的 addressee 文字不在原文")
            if turn.get("speaker") is None:
                unresolved_seen.append((chapter_id, position, "speaker"))
            if turn.get("addresseeId") is None:
                unresolved_seen.append((chapter_id, position, "addressee"))
            turn_total += 1

        checker.check(order_keys == sorted(order_keys), f"{where}: turns 未依段落與偏移排序")

        # ---- devices ----
        spans: dict[str, list[tuple[int, int]]] = {}
        for position, device in enumerate(devices):
            label = f"{where} device#{position}"
            kind = device.get("kind")
            checker.check(kind in declared_kinds, f"{label}: kind {kind} 未在 deviceKinds 宣告")
            paragraph_id = device.get("p")
            if not checker.check(paragraph_id in paragraphs, f"{label}: 段 {paragraph_id} 不屬於本章"):
                continue
            text = paragraphs[paragraph_id]
            offset = device.get("offset")
            length = device.get("len")
            matched = device.get("matched")
            if not checker.check(isinstance(offset, int) and offset >= 0, f"{label}: offset 非法"):
                continue
            checker.check(isinstance(length, int) and length > 0, f"{label}: len 必須為正整數")
            checker.check(
                isinstance(matched, str) and matched and len(matched) == length,
                f"{label}: matched 長度必須等於 len",
            )
            end = offset + (length or 0)
            checker.check(end <= len(text), f"{label}: offset+len 越界（{end} > {len(text)}）")
            if isinstance(matched, str):
                checker.check(text[offset:end] == matched, f"{label}: 切片不符 {text[offset:end]!r} != {matched!r}")
            spans.setdefault(paragraph_id, []).append((offset, end))
            device_total += 1

        for paragraph_id, ranges in spans.items():
            ranges.sort()
            for previous, current in zip(ranges, ranges[1:]):
                checker.check(
                    previous[1] <= current[0],
                    f"{where}: 段 {paragraph_id} 的手段標記重疊 {previous} / {current}",
                )

    # ---- unresolvedTurns ----
    unresolved = doc.get("unresolvedTurns") or []
    reported: list[tuple[str, int, str]] = []
    for item in unresolved:
        chapter_id = item.get("chapterId")
        label = f"unresolved {chapter_id}"
        if not checker.check(chapter_id in chapter_paragraphs, f"{label}: 未知章"):
            continue
        index = item.get("turnIndex")
        field = item.get("field")
        checker.check(field in ("speaker", "addressee"), f"{label}: field 非法 {field}")
        checker.check(item.get("reason") in REASONS, f"{label}: reason 未在詞表內：{item.get('reason')}")
        entry = next((e for e in entries if e.get("chapterId") == chapter_id), None)
        turns = (entry or {}).get("turns") or []
        if not checker.check(isinstance(index, int) and 0 <= index < len(turns), f"{label}: turnIndex 越界"):
            continue
        turn = turns[index]
        checker.check(turn.get("p") == item.get("p"), f"{label}: p 與該 turn 不符")
        missing = turn.get("speaker") is None if field == "speaker" else turn.get("addresseeId") is None
        checker.check(missing, f"{label}: 該 turn 的 {field} 並非未識別")
        reported.append((chapter_id, index, field))

    checker.check(len(reported) == len(set(reported)), "unresolvedTurns 有重複條目")
    checker.check(
        sorted(reported) == sorted(unresolved_seen),
        f"unresolvedTurns 與 null 欄位不一致：多 {sorted(set(reported) - set(unresolved_seen))[:5]}，"
        f"缺 {sorted(set(unresolved_seen) - set(reported))[:5]}",
    )

    print(f"章 {len(entries)} · turns {turn_total} · devices {device_total} · unresolved {len(unresolved)}")
    return checker.errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--strategy", default=str(DEFAULT_STRATEGY))
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args()

    errors = validate(pathlib.Path(args.strategy))
    if errors:
        print(f"✗ {len(errors)} 條斷言不成立：", file=sys.stderr)
        for error in errors[:25]:
            print(f"  - {error}", file=sys.stderr)
        if len(errors) > 25:
            print(f"  …（另 {len(errors) - 25} 條）", file=sys.stderr)
        return 1
    if not args.quiet:
        print("✓ 全部斷言通過")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
