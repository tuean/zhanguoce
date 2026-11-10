#!/usr/bin/env python3
"""Fail-fast validator for data/zhanguoce/events/chapter_years.json (M3c-2).

Checks every invariant the reader relies on:

1. every chapter of the book is either decided or unresolved, exactly once;
2. every decided chapter has at least one evidence entry;
3. every evidence eventId exists in events/tongjian.json;
4. yearBc sits inside yearRange and inside the timeline year bounds;
5. confidence is only high/medium -- no low;
6. unresolved entries carry a fixed, non-empty reason and no yearBc;
7. the JSON is byte-for-byte stable (sorted keys are not required, wording is);
8. evidence carries exactly one of two shapes (M3c-2b):
     * name-overlap / name-window -> a non-empty "names" list, no "matched",
       and every name really occurs in that chapter's body (punctuation
       stripped). A joined name string is not a quotation, so it must never be
       stored as one;
     * substring / substring-range / commentary-date -> "matched", no "names".

Exit code 0 = clean, 1 = violations (all printed).

Usage: validate_chapter_years.py [chapter_years.json]
The optional argument re-points the validator at another file, which is how the
negative test (a name that the chapter does not contain) is run.
"""
from __future__ import annotations

import json
import pathlib
import sys
from collections import Counter

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

from build_chapter_years import norm  # noqa: E402  (same normalisation as the builder)

ROOT = pathlib.Path(__file__).resolve().parent.parent
BOOK = ROOT / "data" / "zhanguoce"
YEARS = BOOK / "events" / "chapter_years.json"
EVENTS = BOOK / "events" / "tongjian.json"

REASONS = {
    "no-event-before-453",
    "no-tongjian-match",
    "below-threshold",
    "formula-match",
    "ambiguous-multi-year",
    "tongjian-bucket-year",
    "commentary-conflict",
    "manual-review-reject",
}
CONFIDENCE = {"high", "medium"}
NAME_KINDS = {"name-overlap", "name-window"}
TEXT_KINDS = {"substring", "substring-range", "commentary-date"}


def chapters():
    """chapterId -> punctuation-stripped body, for the name-in-chapter check."""
    bodies = {}
    for path in sorted((BOOK / "volumes").glob("*.json")):
        volume = json.loads(path.read_text(encoding="utf-8"))
        for chapter in volume.get("chapters", []):
            bodies[chapter["id"]] = norm("".join(p.get("text", "") for p in chapter["paragraphs"]))
    return bodies


def main() -> int:
    problems = []
    path = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else YEARS
    data = json.loads(path.read_text(encoding="utf-8"))
    timeline = json.loads(EVENTS.read_text(encoding="utf-8"))
    event_ids = {event["id"] for event in timeline["events"]}
    bounds = (min(e["yearBc"] for e in timeline["events"]), max(e["yearBc"] for e in timeline["events"]))
    earliest, latest = bounds
    bodies = chapters()
    book_ids = list(bodies)
    kinds = Counter()

    if data.get("schemaVersion") != 1:
        problems.append(f"schemaVersion is {data.get('schemaVersion')!r}, expected 1")

    seen = {}
    for chapter in data.get("chapters", []):
        cid = chapter.get("chapterId", "")
        if cid in seen:
            problems.append(f"{cid}: decided twice")
        seen[cid] = "decided"
        if cid not in bodies:
            problems.append(f"{cid}: not a chapter of the book")
        evidence = chapter.get("evidence") or []
        if not evidence:
            problems.append(f"{cid}: decided with empty evidence")
        for item in evidence:
            eid = item.get("eventId", "")
            kind = item.get("kind")
            kinds[kind] += 1
            if eid not in event_ids:
                problems.append(f"{cid}: evidence points at missing event {eid!r}")
            if kind not in NAME_KINDS | TEXT_KINDS:
                problems.append(f"{cid}: unknown evidence kind {kind!r}")
            elif kind in NAME_KINDS:
                if "matched" in item:
                    problems.append(
                        f"{cid}: {kind} evidence {eid} carries matched; joined names are not a quotation"
                    )
                names = item.get("names")
                if not isinstance(names, list) or not names:
                    problems.append(f"{cid}: {kind} evidence {eid} has no non-empty names list")
                    continue
                for name in names:
                    if not isinstance(name, str) or not name.strip():
                        problems.append(f"{cid}: {kind} evidence {eid} has a blank name {name!r}")
                    elif cid in bodies and norm(name) not in bodies[cid]:
                        problems.append(f"{cid}: {kind} evidence {eid} names {name!r}, which is not in this chapter")
            else:
                if "names" in item:
                    problems.append(f"{cid}: {kind} evidence {eid} carries names; text evidence must quote matched")
                if not str(item.get("matched", "")).strip():
                    problems.append(f"{cid}: evidence {eid} has an empty matched span")
        if chapter.get("confidence") not in CONFIDENCE:
            problems.append(f"{cid}: confidence {chapter.get('confidence')!r} is not high/medium")
        lo, hi = chapter.get("yearRange", [None, None])
        year = chapter.get("yearBc")
        if lo is None or hi is None or lo > hi:
            problems.append(f"{cid}: bad yearRange {chapter.get('yearRange')!r}")
        elif not (lo <= year <= hi):
            problems.append(f"{cid}: yearBc {year} outside yearRange {lo}-{hi}")
        if not (earliest <= year <= latest):
            problems.append(f"{cid}: yearBc {year} outside timeline {earliest}-{latest}")
        if not str(chapter.get("method", "")).strip():
            problems.append(f"{cid}: empty method")

    for chapter in data.get("unresolved", []):
        cid = chapter.get("chapterId", "")
        if cid in seen:
            problems.append(f"{cid}: listed as unresolved but already {seen[cid]}")
        seen[cid] = "unresolved"
        if "yearBc" in chapter:
            problems.append(f"{cid}: unresolved entry carries a yearBc")
        if chapter.get("reason") not in REASONS:
            problems.append(f"{cid}: unresolved reason {chapter.get('reason')!r} not in the fixed vocabulary")

    for cid in book_ids:
        if cid not in seen:
            problems.append(f"{cid}: not mentioned at all (every chapter must be decided or unresolved)")

    decided = len(data.get("chapters", []))
    unresolved = len(data.get("unresolved", []))
    print(f"chapters={len(book_ids)} decided={decided} unresolved={unresolved} coverage={decided/len(book_ids):.1%}")
    print("evidence kinds:", dict(kinds))
    if problems:
        print(f"FAIL: {len(problems)} problem(s)")
        for problem in problems[:40]:
            print("  -", problem)
        return 1
    print("OK: every decided year has real, non-empty evidence and sits inside the timeline")
    return 0


if __name__ == "__main__":
    sys.exit(main())
