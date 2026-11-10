#!/usr/bin/env python3
"""Build data/zhanguoce/entities/{people,places}.json from the curated roster.

Matching is exact substring matching over the traditional base text: every
form (name, nameSimp, each alias and its simplified twin) is searched as a
literal string.  No fuzzy matching, no normalisation, no inference.

Precision guard for one-character place names: a bare 秦 / 齊 / 楚 / 趙 / 魏 /
韓 / 燕 / 宋 / 衞 / 巴 / 蜀 / 莒 ... is only counted when it does not sit
inside a longer name that is already an entity of this layer (蘇秦, 秦昭王,
衞鞅, 齊貌辨 ...).  Missing a mention is acceptable; attaching a state name to
a person's surname is not.
"""
from __future__ import annotations

import json
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import entities_lib as lib
import entities_seed as seed


def alias_forms(aliases: list[str]) -> list[str]:
    """Alias list with the simplified twin of every form, in stable order."""
    out: list[str] = []
    for alias in aliases:
        for form in (alias, lib.to_simp(alias)):
            if form not in out:
                out.append(form)
    return out


def scan(forms_by_id: dict[str, list[str]], paragraphs: list[tuple[str, str]]):
    """All multi-character matches, and the per-paragraph span mask they form."""
    hits: dict[str, list[tuple[int, int, int, str]]] = defaultdict(list)
    masked: dict[int, list[tuple[int, int]]] = defaultdict(list)
    for ent_id, forms in forms_by_id.items():
        for form in forms:
            if len(form) < 2:
                continue
            for idx, (_pid, text) in enumerate(paragraphs):
                if form not in text:
                    continue
                start = 0
                while True:
                    pos = text.find(form, start)
                    if pos < 0:
                        break
                    hits[ent_id].append((idx, pos, len(form), form))
                    masked[idx].append((pos, pos + len(form)))
                    start = pos + 1
    return hits, masked


def build(entity_defs: list[dict], kind: str, paragraphs: list[tuple[str, str]],
          masked: dict[int, list[tuple[int, int]]]):
    forms_by_id: dict[str, list[str]] = {}
    for ent in entity_defs:
        forms_by_id[ent["id"]] = lib.expanded_forms(ent["name"], ent["aliases"])

    multichar: dict[str, list[tuple[int, int, int, str]]] = defaultdict(list)
    own_hits, _own_mask = scan(forms_by_id, paragraphs)
    multichar.update(own_hits)

    occurrences: dict[str, list[tuple[int, int, int, str]]] = defaultdict(list)
    for ent in entity_defs:
        occurrences[ent["id"]] = list(multichar.get(ent["id"], []))
        for form in forms_by_id[ent["id"]]:
            if len(form) != 1:
                continue
            for idx, (_pid, text) in enumerate(paragraphs):
                start = 0
                while True:
                    pos = text.find(form, start)
                    if pos < 0:
                        break
                    start = pos + 1
                    end = pos + 1
                    if any(s < end and pos < e for s, e in masked.get(idx, ())):
                        continue
                    occurrences[ent["id"]].append((idx, pos, 1, form))

    out = []
    empty = []
    for ent in entity_defs:
        # one hit per (paragraph, offset): keep the longest form there, so
        # 函谷關 wins over its own alias 函谷 at the same position.
        best: dict[tuple[int, int], tuple[int, int, int, str]] = {}
        for idx, pos, length, form in occurrences[ent["id"]]:
            key = (idx, pos)
            prev = best.get(key)
            if prev is None or length > prev[2]:
                best[key] = (idx, pos, length, form)
        rows = sorted(best.values(), key=lambda r: (r[0], r[1], r[2], r[3]))
        if not rows:
            empty.append(ent["name"])
        out.append({
            "id": ent["id"],
            "kind": kind,
            "name": ent["name"],
            "nameSimp": ent["nameSimp"],
            "aliases": alias_forms(ent["aliases"]),
            "tags": ent["tags"],
            "summary": ent["summary"],
            "occurrences": [
                {"p": paragraphs[idx][0], "o": pos, "len": length, "form": form}
                for idx, pos, length, form in rows
            ],
        })
    return out, empty, forms_by_id


def main() -> int:
    paragraphs = lib.load_paragraphs()
    # the single-character guard must see every entity of the layer, people
    # included, otherwise 蘇秦 would also count as the state of 秦.
    all_forms = {
        ent["id"]: lib.expanded_forms(ent["name"], ent["aliases"])
        for ent in seed.PERSONS + seed.PLACES
    }
    _hits, mask = scan(all_forms, paragraphs)
    people, empty_people, forms_people = build(seed.PERSONS, "person", paragraphs, mask)
    places, empty_places, forms_places = build(seed.PLACES, "place", paragraphs, mask)

    if empty_people or empty_places:
        print("ERROR: entities without any occurrence:")
        for name in empty_people + empty_places:
            print("  -", name)
        return 1

    lib.ENTITIES.mkdir(parents=True, exist_ok=True)
    for kind, entities in (("people", people), ("places", places)):
        doc = {
            "schemaVersion": 1,
            "bookId": "zhanguoce",
            "kind": "person" if kind == "people" else "place",
            "entities": entities,
        }
        target = lib.ENTITIES / f"{kind}.json"
        target.write_text(
            json.dumps(doc, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )

    def report(label, entities, forms_map):
        total = sum(len(e["occurrences"]) for e in entities)
        covered = {o["p"] for e in entities for o in e["occurrences"]}
        zero_forms = [
            (e["name"], f)
            for e in entities
            for f in forms_map[e["id"]]
            if not any(o["form"] == f for o in e["occurrences"])
        ]
        print(f"{label}: entities={len(entities)} occurrences={total} "
              f"paragraphs_covered={len(covered)}")
        if zero_forms:
            print(f"  forms with 0 hits ({len(zero_forms)}): "
                  + ", ".join(f"{n}:{f}" for n, f in zero_forms))
        return covered

    cov_p = report("person", people, forms_people)
    cov_l = report("place", places, forms_places)
    print(f"total_paragraphs={len(paragraphs)} "
          f"union_covered={len(cov_p | cov_l)} "
          f"person_only={len(cov_p)} place_only={len(cov_l)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
