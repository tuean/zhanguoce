#!/usr/bin/env python3
"""Build data/zhanguoce/events/chapter_years.json — evidenced 系年 for 戰國策 chapters.

Method (auditable, see docs/M3C2_NOTES.md):
  * text match: longest common substring (after punctuation stripping) between a
    chapter body and one 通鑑 event, kept only when long, rare (df<=2) and clearly
    ahead of the runner-up year;
  * commentary-date: an 注疏 sentence that both names a 周/秦 王年 AND carries a
    decisive cue (此章/當在/附載…), rejecting negation ("必非") and corrections
    ("此當作").
Every emitted year carries >=1 evidence pointing at a real event id.

Two evidence shapes, never mixed:
  * substring / substring-range / commentary-date carry "matched", a span that
    really occurs in the text it quotes;
  * name-overlap / name-window carry "names", a list of person names. They must
    NOT carry "matched": joining names into one string produces glyphs the
    chapter never contains, and showing that next to 章文「…」 invents a
    quotation that looks checkable but is false.
"""
from __future__ import annotations

import json
import re
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BOOK = ROOT / "data" / "zhanguoce"
EVENTS = BOOK / "events" / "tongjian.json"
OUT = BOOK / "events" / "chapter_years.json"

PUNCT = set("，。「」：；！？、『』（）《》…—・·○　 \n\t\r“”‘’,.;:!?()[]<>/|~\"'")
K = 6                      # seed gram length
L_MIN = 8                  # minimum common-substring length to be considered
DF_MAX = 2                 # a span appearing in more than 2 events is a formula
MARGIN = 2                 # best year must beat the runner-up year by this many chars
HIGH_L = 14                # L at or above this (with df==1) is "high"
SPAN_SLACK = 2             # years within best L-2 may join the range
RANGE_MAX = 3              # a year range wider than this is dropped back to a single year
# 通鑑 opens at 前403 and packs the whole 前455–前403 三家滅智 story into that one
# year: the year is an editorial bucket, not the date of the event. Matching it
# would put 知伯 on 前403 when he died 前453, so those events never date a chapter.
BUCKET_YEARS = {403}
# Rejections from the manual sample review (chapterId -> why). Kept in code so the
# decision is auditable next to the rule that produced the candidate.
REVIEW_REJECT = {
    "qi-3-01": "楚王死 narrates 懷王's death (前296); the 前299 event is only his detention",
    "dongzhou-18": "蘇厲爲周最謂蘇秦 puts 蘇秦 alive at 前310, but 通鑑 records his death at 前316",
    "song-wei-14": "胥靡 story belongs to 嗣君's early reign; 前283 is only the year 通鑑 collects his anecdotes",
    "han-1-17": "秦韓戰于濁澤: 濁澤之戰 is 前307; the L8 span 賂之以一名都與之 belongs to 張儀誑楚, not this chapter",
    "qi-6-08": "齊閔王之遇殺: 閔王 dies 前284; the matched 法章 passage is the 前283 aftermath, one year late",
    "zhongshan-10": "昭王既息民繕兵: only evidence is the formula 楚地方五千里持戟百萬 (df=1 by luck); no real match",
}

RULER_RE = re.compile(r"(威烈王|安王|烈王|顯王|慎靚王|愼靚王|赧王|昭襄王)([一二三四五六七八九十百]{1,4})年")
CUE_RE = re.compile(r"此章|此䇿|此策|當在|是歳|是歲|此年|此即|此當|附載|正當|按通鑑|按通鉴")
NEG_RE = re.compile(r"必非|非赧王|此當作|當作|未可|恐非")


def norm(text: str) -> str:
    return "".join(ch for ch in text if ch not in PUNCT)


def cn_num(s: str):
    digits = {"一": 1, "二": 2, "三": 3, "四": 4, "五": 5, "六": 6, "七": 7, "八": 8, "九": 9}
    total, section, num = 0, 0, 0
    for ch in s:
        if ch in digits:
            num = digits[ch]
        elif ch == "十":
            section += (num or 1) * 10
            num = 0
        elif ch == "百":
            section += (num or 1) * 100
            num = 0
        else:
            return None
    return total + section + num


def load_chapters():
    chapters = {}
    for path in sorted((BOOK / "volumes").glob("*.json")):
        doc = json.loads(path.read_text(encoding="utf-8"))
        for ch in doc["chapters"]:
            body = "".join(p["text"] for p in ch["paragraphs"])
            anns = [a for p in ch["paragraphs"] for a in p.get("annotations", [])]
            chapters[ch["id"]] = {
                "id": ch["id"],
                "title": ch["title"],
                "body": body,
                "norm": norm(body),
                "anns": anns,
            }
    return chapters


def load_events():
    doc = json.loads(EVENTS.read_text(encoding="utf-8"))
    events = doc["events"]
    for e in events:
        e["norm"] = norm(e["text"])
    return doc, events


def load_people():
    doc = json.loads((BOOK / "entities" / "people.json").read_text(encoding="utf-8"))
    return doc["entities"] if isinstance(doc, dict) and "entities" in doc else doc


def span_index(events, k=K):
    idx = defaultdict(list)
    for i, e in enumerate(events):
        n = e["norm"]
        for j in range(len(n) - k + 1):
            idx[n[j:j + k]].append((i, j))
    return idx


def best_matches(chapter_norm, events, idx, k=K):
    best = {}
    n = chapter_norm
    for i in range(len(n) - k + 1):
        gram = n[i:i + k]
        for ei, ej in idx.get(gram, ()):
            en = events[ei]["norm"]
            a, b = i, ej
            while a > 0 and b > 0 and n[a - 1] == en[b - 1]:
                a -= 1
                b -= 1
            c, d = i + k, ej + k
            while c < len(n) and d < len(en) and n[c] == en[d]:
                c += 1
                d += 1
            L = c - a
            prev = best.get(ei)
            if prev is None or L > prev[0]:
                best[ei] = (L, n[a:c])
    return best


def person_years(events, people):
    """name -> (earliest, latest) 通鑑 year the name is attested in."""
    spans = {}
    for p in people:
        for nm in [p["name"]] + list(p.get("aliases") or []):
            if not nm or len(nm) < 2:
                continue
            ys = [e["yearBc"] for e in events if nm in e["text"]]
            if ys:
                spans[nm] = (min(ys), max(ys))
    return spans


def chronology_ok(names, year, spans, slack=2):
    """A chapter cannot be dated outside the 通鑑 lifespan of a person it names."""
    for nm in names:
        span = spans.get(nm)
        if span and not (span[0] - slack <= year <= span[1] + slack):
            return False
    return True


def chapter_persons(chapter, people):
    names = set()
    for p in people:
        for nm in [p["name"]] + list(p.get("aliases") or []):
            if nm and len(nm) >= 2 and nm in chapter["body"]:
                names.add(nm)
    return names


def analyse(chapter, events, idx, people):
    m = best_matches(chapter["norm"], events, idx)
    if not m:
        return None, None, {"reason": "no-tongjian-match", "best": 0, "second": 0}
    scored = []
    for ei, (L, span) in m.items():
        if L < K:
            continue
        df = sum(1 for e in events if span in e["norm"])
        scored.append({"ei": ei, "L": L, "span": span, "df": df})
    if not scored:
        return None, None, {"reason": "below-threshold", "best": 0, "second": 0}
    pn = chapter_persons(chapter, people)
    for c in scored:
        c["persons"] = sorted(nm for nm in pn if nm in events[c["ei"]]["text"])
        c["year"] = events[c["ei"]]["yearBc"]
    scored.sort(key=lambda c: (-c["L"], c["df"]))
    per_year = {}
    for c in scored:
        y = c["year"]
        best = per_year.get(y)
        if best is None or (c["L"] > best["L"]) or (c["L"] == best["L"] and c["df"] < best["df"]):
            per_year[y] = c
    ranked = sorted(per_year.values(), key=lambda c: (-c["L"], c["df"]))
    diag = {"best": ranked[0]["L"], "second": ranked[1]["L"] if len(ranked) > 1 else 0,
            "years": [c["year"] for c in ranked[:4]]}
    return scored, ranked, diag


def pick(ranked, diag):
    top = ranked[0]
    gap = top["L"] - diag["second"]
    if top["L"] < L_MIN:
        return None, "below-threshold"
    if top["df"] > DF_MAX:
        return None, "formula-match"
    if gap < MARGIN:
        return None, "ambiguous-multi-year"
    if top["year"] in BUCKET_YEARS:
        return None, "tongjian-bucket-year"
    years = {top["year"]}
    for c in ranked[1:]:
        if c["L"] >= top["L"] - SPAN_SLACK and c["df"] <= DF_MAX and c["year"] not in BUCKET_YEARS:
            years.add(c["year"])
    if max(years) - min(years) > RANGE_MAX:
        years = {top["year"]}
    conf = "high" if (top["L"] >= HIGH_L and top["df"] == 1) else "medium"
    return {"year": top["year"], "years": sorted(years), "conf": conf}, None


def name_window(chapter, events, spans, people, name_events):
    """Rare-name co-occurrence: >=2 names that 通鑑 attests only a few times, all
    meeting in exactly one year. Never used when the text itself matched."""
    names = chapter_persons(chapter, people)
    # Sorted, because chapter_persons returns a set: without this the emitted
    # name order (and which three names survive the [:3] cut) changes with the
    # interpreter's hash seed and the data file churns on every rebuild.
    rare = sorted(nm for nm in names if 1 <= len(name_events.get(nm, ())) <= 3)
    if len(rare) < 2:
        return None
    per_year = defaultdict(list)
    for e in events:
        hit = [nm for nm in rare if nm in e["text"]]
        if len(hit) >= 2:
            per_year[e["yearBc"]].append((e, hit))
    if len(per_year) != 1:
        return None
    year, hits = next(iter(per_year.items()))
    if year in BUCKET_YEARS or not chronology_ok(names, year, spans):
        return None
    evidence = []
    for e, hit in hits[:2]:
        evidence.append({
            "eventId": e["id"],
            "kind": "name-window",
            "names": hit[:3],
            "score": 0.55,
        })
    return {
        "chapterId": chapter["id"],
        "yearBc": year,
        "yearRange": [year, year],
        "confidence": "medium",
        "method": "name-window",
        "evidence": evidence,
        "reason": None,
    }


def commentary_dates(chapter, year_lookup):
    found = []
    for a in chapter["anns"]:
        text = a["text"]
        for m in RULER_RE.finditer(text):
            window = text[max(0, m.start() - 30):m.end() + 40]
            if not CUE_RE.search(window) or NEG_RE.search(window):
                continue
            ruler, num = m.group(1), cn_num(m.group(2))
            if num is None:
                continue
            key = (ruler, num)
            if key in year_lookup:
                found.append((ruler, num, year_lookup[key], window.replace("\n", "")))
    return found


def main():
    chapters = load_chapters()
    doc, events = load_events()
    people = load_people()
    idx = span_index(events)
    spans = person_years(events, people)
    name_events = defaultdict(list)
    for nm in spans:
        name_events[nm] = [e for e in events if nm in e["text"]]
    year_lookup = {(y["ruler"], y["rulerYear"]): y["yearBc"] for y in doc["years"]}

    decided, unresolved = [], []
    reasons = Counter()
    for cid, ch in chapters.items():
        if cid in REVIEW_REJECT:
            reasons["manual-review-reject"] += 1
            unresolved.append({"chapterId": cid, "reason": "manual-review-reject",
                               "note": REVIEW_REJECT[cid]})
            continue
        scored, ranked, diag = analyse(ch, events, idx, people)
        if scored is None:
            window = name_window(ch, events, spans, people, name_events)
            if window:
                decided.append(window)
                continue
            reasons[diag["reason"]] += 1
            unresolved.append({"chapterId": cid, "reason": diag["reason"]})
            continue
        choice, why = pick(ranked, diag)
        comm = commentary_dates(ch, year_lookup)
        if choice is None:
            window = name_window(ch, events, spans, people, name_events)
            if window:
                decided.append(window)
                continue
            if comm and why == "below-threshold":
                why = "below-threshold-commentary-only"
            reasons[why] += 1
            unresolved.append({"chapterId": cid, "reason": why,
                               "note": "bestL=%d secondL=%d" % (diag["best"], diag["second"])})
            continue

        top = ranked[0]
        ev_year = choice["year"]
        conflict = [c for c in comm if abs(c[2] - ev_year) > 2]
        if conflict:
            reasons["commentary-conflict"] += 1
            unresolved.append({"chapterId": cid, "reason": "commentary-conflict",
                               "note": "text=%s commentary=%s" % (ev_year, conflict[0][2])})
            continue

        evidence = [{
            "eventId": events[top["ei"]]["id"],
            "kind": "substring",
            "matched": top["span"],
            "score": round(top["L"] / 30.0, 2),
            "length": top["L"],
            "df": top["df"],
        }]
        for c in ranked[1:]:
            if c["year"] in choice["years"] and c["ei"] != top["ei"] and len(evidence) < 3:
                evidence.append({
                    "eventId": events[c["ei"]]["id"],
                    "kind": "substring-range",
                    "matched": c["span"],
                    "score": round(c["L"] / 30.0, 2),
                    "length": c["L"],
                    "df": c["df"],
                })
        if top["persons"]:
            evidence.append({
                "eventId": events[top["ei"]]["id"],
                "kind": "name-overlap",
                "names": top["persons"][:3],
                "score": 0.5,
            })
        agree = [c for c in comm if c[2] == ev_year]
        for ruler, num, y, quote in agree[:1]:
            evidence.append({
                "eventId": events[top["ei"]]["id"],
                "kind": "commentary-date",
                "matched": "%s%s年 · %s" % (ruler, num, quote[:60]),
                "score": 1.0,
            })
        if agree:
            choice["conf"] = "high"

        years = choice["years"] or [ev_year]
        decided.append({
            "chapterId": cid,
            "yearBc": ev_year,
            "yearRange": [min(years), max(years)],
            "confidence": choice["conf"],
            "method": "event-text-match",
            "evidence": evidence,
            "reason": None,
        })

    out = {
        "schemaVersion": 1,
        "bookId": "zhanguoce",
        "source": "通鑑戰國段事件表 + 戰國策注疏系年",
        "generatedBy": "tools/build_chapter_years.py",
        "thresholds": {"L_MIN": L_MIN, "DF_MAX": DF_MAX, "MARGIN": MARGIN, "HIGH_L": HIGH_L},
        "chapters": decided,
        "unresolved": unresolved,
    }
    OUT.write_text(json.dumps(out, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
    total = len(chapters)
    print("chapters=%d decided=%d unresolved=%d coverage=%.1f%%" % (
        total, len(decided), len(unresolved), 100.0 * len(decided) / total))
    print("confidence:", dict(Counter(c["confidence"] for c in decided)))
    print("reasons:", dict(reasons))


if __name__ == "__main__":
    main()
