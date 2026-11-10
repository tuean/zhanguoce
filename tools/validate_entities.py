#!/usr/bin/env python3
"""Validate data/zhanguoce/entities/{people,places}.json against docs/ENTITIES.md.

Every hard rule from the contract is a named check with its own PASS/FAIL and
an item count.  The script re-reads the corpus and re-runs the substring scan
itself (the builder is imported only for nothing at all) so a stale or
hand-edited occurrence list cannot slip through.
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import entities_lib as lib

ID_RE = re.compile(r"^(person|place)-[a-z0-9]+(?:-[a-z0-9]+)*$")
TOP_KEYS = {"schemaVersion", "bookId", "kind", "entities"}
ENT_KEYS = {"id", "kind", "name", "nameSimp", "aliases", "tags", "summary", "occurrences"}
OCC_KEYS = {"p", "o", "len", "form"}

results: list[tuple[str, bool, str]] = []


def check(label: str, ok: bool, detail: str = "") -> bool:
    results.append((label, ok, detail))
    print(f"[{'PASS' if ok else 'FAIL'}] {label}{(' -- ' + detail) if detail else ''}")
    return ok


def main() -> int:
    paragraphs = lib.load_paragraphs()
    order = {pid: i for i, (pid, _text) in enumerate(paragraphs)}
    texts = dict(paragraphs)
    titles = {t for t in lib.load_chapter_titles()}

    raw: dict[str, bytes] = {}
    docs: dict[str, dict] = {}
    for kind, fname in (("person", "people.json"), ("place", "places.json")):
        path = lib.ENTITIES / fname
        if not check(f"{fname} exists", path.is_file(), str(path)):
            return 1
        raw[kind] = path.read_bytes()
        docs[kind] = json.loads(raw[kind].decode("utf-8"))

    # ---- 1. file-level schema -------------------------------------------
    ok = True
    for kind, doc in docs.items():
        ok &= check(f"{kind}: top-level keys are exactly schemaVersion/bookId/kind/entities",
                    set(doc) == TOP_KEYS, str(sorted(doc)))
        ok &= check(f"{kind}: schemaVersion == 1 and bookId == zhanguoce",
                    doc.get("schemaVersion") == 1 and doc.get("bookId") == "zhanguoce")
        ok &= check(f"{kind}: kind field == {kind}", doc.get("kind") == kind, str(doc.get("kind")))
        ok &= check(f"{kind}: entities is a non-empty list",
                    isinstance(doc.get("entities"), list) and bool(doc["entities"]),
                    str(len(doc.get("entities") or [])))
    if not ok:
        return 1

    people = docs["person"]["entities"]
    places = docs["place"]["entities"]
    everything = people + places

    # ---- 2. entity-level schema -----------------------------------------
    bad_keys = [e.get("id") for e in everything if set(e) != ENT_KEYS]
    check("entity objects carry exactly the 8 contract fields", not bad_keys, str(bad_keys[:3]))

    bad_kind = [e.get("id") for e in everything if e.get("kind") != docs["person"]["kind"]
                and e.get("kind") != docs["place"]["kind"]]
    check("every entity kind is person|place", not bad_kind, str(bad_kind[:3]))

    bad_prefix = [e["id"] for e in everything
                  if not ID_RE.match(str(e.get("id", "")))
                  or not str(e["id"]).startswith(e["kind"] + "-")]
    check("id is <kind>-<slug> and matches the id pattern", not bad_prefix, str(bad_prefix[:3]))

    ids = [e["id"] for e in everything]
    dup = sorted({i for i in ids if ids.count(i) > 1})
    check("id unique across people.json + places.json", not dup, str(dup[:5]))

    # ---- 3. traditional / simplified spelling ---------------------------
    bad_simp = []
    for e in everything:
        simp = e.get("nameSimp")
        if not isinstance(simp, str) or not simp:
            bad_simp.append((e["id"], "empty"))
            continue
        if simp != lib.to_simp(e["name"]):
            bad_simp.append((e["id"], f"{simp} != to_simp({e['name']})"))
        elif any(ch in lib.TRAD_ONLY for ch in simp):
            bad_simp.append((e["id"], f"traditional char left in nameSimp: {simp}"))
        elif len(simp) != len(e["name"]):
            bad_simp.append((e["id"], "length drift"))
    check("nameSimp is the simplified twin of name (no traditional char left)",
          not bad_simp, str(bad_simp[:5]))

    bad_alias = []
    for e in everything:
        aliases = e.get("aliases")
        if not isinstance(aliases, list):
            bad_alias.append((e["id"], "not a list"))
            continue
        if len(set(aliases)) != len(aliases):
            bad_alias.append((e["id"], "duplicate alias"))
        if e["name"] in aliases:
            bad_alias.append((e["id"], "name repeated as alias"))
        for a in aliases:
            if not isinstance(a, str) or not a.strip():
                bad_alias.append((e["id"], "empty alias"))
            elif a != lib.to_simp(a) and lib.to_simp(a) not in aliases:
                bad_alias.append((e["id"], f"simplified twin missing for {a}"))
    check("aliases are non-empty, unique, and closed under 繁->简", not bad_alias, str(bad_alias[:5]))

    # ---- 4. summary ------------------------------------------------------
    bad_summary = []
    for e in everything:
        s = e.get("summary")
        if s is None:
            continue
        if not isinstance(s, str) or not s.strip():
            bad_summary.append((e["id"], "empty summary"))
            continue
        for quoted in re.findall(r"《([^《》]+)》", s):
            if quoted not in titles:
                bad_summary.append((e["id"], f"unknown chapter title: {quoted}"))
    check("summary is null or a sentence whose 《...》 quotes are real chapter titles",
          not bad_summary, str(bad_summary[:5]))

    # ---- 5. occurrences point at real text -------------------------------
    for label, ents in (("person", people), ("place", places)):
        missing_pid, bad_offset, oob = [], [], []
        for e in ents:
            for occ in e.get("occurrences") or []:
                p, o, ln, form = occ.get("p"), occ.get("o"), occ.get("len"), occ.get("form")
                if p not in texts:
                    missing_pid.append((e["id"], p))
                    continue
                if not isinstance(o, int) or not isinstance(ln, int) or o < 0 or ln < 1:
                    bad_offset.append((e["id"], p, o, ln))
                    continue
                if o + ln > len(texts[p]):
                    oob.append((e["id"], p, o, ln))
                    continue
                if texts[p][o:o + ln] != form:
                    bad_offset.append((e["id"], p, o, ln, texts[p][o:o + ln], form))
        check(f"{label}: every occurrence p is a real paragraph id",
              not missing_pid, str(missing_pid[:3]))
        check(f"{label}: offsets are integers within the paragraph",
              not oob and not bad_offset, str((oob + bad_offset)[:3]))
        check(f"{label}: 偏移可复核 text[o:o+len] == form (逐条切片比对)",
              not bad_offset, str(bad_offset[:3]))

    # ---- 6. form must be a declared name/alias ---------------------------
    bad_form, bad_len = [], []
    for e in everything:
        declared = set(lib.expanded_forms(e["name"], e["aliases"]))
        for occ in e["occurrences"]:
            if occ["form"] not in declared:
                bad_form.append((e["id"], occ["form"]))
            if occ["len"] != len(occ["form"]):
                bad_len.append((e["id"], occ["form"], occ["len"]))
    check("every form is one of the entity's name/nameSimp/aliases", not bad_form, str(bad_form[:3]))
    check("len equals the character length of form", not bad_len, str(bad_len[:3]))

    # ---- 7. no duplicate / misordered occurrences ------------------------
    dup_occ, unsorted = [], []
    for e in everything:
        keys = [(occ["p"], occ["o"]) for occ in e["occurrences"]]
        if len(set(keys)) != len(keys):
            dup_occ.append(e["id"])
        seq = [(order[occ["p"]], occ["o"]) for occ in e["occurrences"] if occ["p"] in order]
        if seq != sorted(seq):
            unsorted.append(e["id"])
    check("no duplicate (paragraph, offset) inside an entity", not dup_occ, str(dup_occ[:3]))
    check("occurrences follow book order then ascending offset", not unsorted, str(unsorted[:3]))

    # ---- 8. coverage requirements ----------------------------------------
    check("person count >= 120", len(people) >= 120, f"{len(people)}")
    check("place count >= 60", len(places) >= 60, f"{len(places)}")
    empty = [e["id"] for e in everything if not e["occurrences"]]
    check("no dangling entity: every entity has >= 1 occurrence", not empty, str(empty[:5]))

    # ---- 9. single-character place guard --------------------------------
    spans: dict[str, list[tuple[int, int, int]]] = defaultdict(list)  # pid -> (o,end,idx)
    for idx, e in enumerate(everything):
        for occ in e["occurrences"]:
            if occ["len"] >= 2:
                spans[occ["p"]].append((occ["o"], occ["o"] + occ["len"], idx))
    leak = []
    for e in places:
        for occ in e["occurrences"]:
            if occ["len"] != 1:
                continue
            for s, end, idx in spans.get(occ["p"], ()):
                if everything[idx]["id"] != e["id"] and s < occ["o"] + 1 and occ["o"] < end:
                    leak.append((e["id"], occ["p"], occ["o"], everything[idx]["id"]))
    check("bare state/place characters never sit inside another entity's name",
          not leak, str(leak[:5]))

    # ---- 10. completeness: independent rescan ---------------------------
    forms = {e["id"]: lib.expanded_forms(e["name"], e["aliases"]) for e in everything}
    mask: dict[str, list[tuple[int, int]]] = defaultdict(list)
    found: dict[str, set[tuple[str, int, int, str]]] = defaultdict(set)
    for e in everything:
        for form in forms[e["id"]]:
            if len(form) < 2:
                continue
            for pid, text in paragraphs:
                pos = text.find(form)
                while pos >= 0:
                    found[e["id"]].add((pid, pos, len(form), form))
                    mask[pid].append((pos, pos + len(form)))
                    pos = text.find(form, pos + 1)
    for e in everything:
        for form in forms[e["id"]]:
            if len(form) != 1:
                continue
            for pid, text in paragraphs:
                pos = text.find(form)
                while pos >= 0:
                    if not any(s < pos + 1 and pos < t for s, t in mask.get(pid, ())):
                        found[e["id"]].add((pid, pos, 1, form))
                    pos = text.find(form, pos + 1)
    diff = []
    for e in everything:
        have = {(o["p"], o["o"], o["len"], o["form"]) for o in e["occurrences"]}
        # a position may legitimately be reported with the longest form only
        have_pos = {(p, o) for p, o, _l, _f in have}
        want_pos = {(p, o) for p, o, _l, _f in found[e["id"]]}
        if have_pos != want_pos:
            diff.append((e["id"], len(want_pos - have_pos), len(have_pos - want_pos)))
    check("occurrence set equals an independent full-corpus rescan",
          not diff, str(diff[:5]))

    # ---- 11. byte formatting --------------------------------------------
    fmt = []
    for kind, data in raw.items():
        name = "people.json" if kind == "person" else "places.json"
        if data.startswith(b"\xef\xbb\xbf"):
            fmt.append((name, "BOM"))
        if b"\r" in data:
            fmt.append((name, "CR"))
        if not data.endswith(b"\n") or data.endswith(b"\n\n"):
            fmt.append((name, "trailing newline"))
        reencoded = (json.dumps(docs[kind], ensure_ascii=False, indent=2) + "\n").encode("utf-8")
        if reencoded != data:
            fmt.append((name, "not canonical 2-space json"))
    check("files are UTF-8 without BOM, LF only, 2-space canonical json",
          not fmt, str(fmt[:3]))

    # ---- 12. idempotency ------------------------------------------------
    before = {k: v for k, v in raw.items()}
    runs = []
    for _ in range(2):
        subprocess.run([sys.executable, str(lib.ROOT / "tools" / "build_entities.py")],
                       check=True, capture_output=True, cwd=lib.ROOT)
        runs.append({
            "person": (lib.ENTITIES / "people.json").read_bytes(),
            "place": (lib.ENTITIES / "places.json").read_bytes(),
        })
    check("rebuild is byte-identical (idempotent, twice)",
          runs[0] == before and runs[1] == before,
          "differs from committed bytes" if not (runs[0] == before and runs[1] == before) else "")

    # ---- summary ---------------------------------------------------------
    occ_total = sum(len(e["occurrences"]) for e in everything)
    cov_p = {o["p"] for e in people for o in e["occurrences"]}
    cov_l = {o["p"] for e in places for o in e["occurrences"]}
    cov = cov_p | cov_l
    print()
    print(f"persons={len(people)} places={len(places)} occurrences={occ_total}")
    print(f"coverage: {len(cov)}/{len(paragraphs)} paragraphs = {100 * len(cov) / len(paragraphs):.2f}%")
    print(f"  person-only={len(cov_p)} place-only={len(cov_l)} "
          f"both={len(cov) - len(cov_p ^ cov_l)}")
    passed = sum(1 for _l, ok, _d in results if ok)
    print(f"checks: {passed} passed, {len(results) - passed} failed, {len(results)} total")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
