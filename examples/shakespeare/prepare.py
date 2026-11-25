#!/usr/bin/env python3
"""Download the PlayShakespeare.com modern editions into raw/, pinned to one commit, and turn them into
records.tsv and aliases.tsv.

Run with `python3 -I prepare.py`. Files already in raw/ are kept, so a rerun only fetches what's missing. The
outputs are committed, so building the index never needs the network. The texts are under the GNU Free
Documentation License 1.3 or later, stated in each file's <license>.

  records.tsv   ref, text, speaker, kind (verse, prose, lyric, rhyme or stage), one line of a work per row
  aliases.tsv   a work's name, then the other names it goes by

A ref is the work's name and act.scene.line ("Hamlet 3.1.56"). A prologue is scene 0 of its act ("Henry V
2.0.12"). An epilogue's name ends in "Epilogue". A sonnet is "Sonnet 18.1" and a poem "Venus and Adonis 5". A
stage direction takes the number of the line it follows.
"""

import sys
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

COMMIT = "9168116e0f244e08d1ff2ee1368d590e4499be6b"  # 2026-08-12, edition 5.4
URL = f"https://raw.githubusercontent.com/severdia/PlayShakespeare.com-XML/{COMMIT}/playshakespeare_editions/{{}}"
HERE = Path(__file__).resolve().parent
RAW = HERE / "raw"

WORKS = """
alls_well_that_ends_well antony_cleopatra as_you_like_it comedy_of_errors coriolanus cymbeline hamlet
henry_iv_pt1 henry_iv_pt2 henry_v henry_vi_pt1 henry_vi_pt2 henry_vi_pt3 henry_viii julius_caesar king_john
king_lear king_richard_ii lovers_complaint loves_labours_lost macbeth measure_for_measure merchant_of_venice
merry_wives_of_windsor midsummer_nights_dream much_ado_about_nothing othello passionate_pilgrim pericles
phoenix_and_turtle rape_of_lucrece richard_iii romeo_and_juliet sir_thomas_more sonnets taming_of_the_shrew
tempest timon_of_athens titus_andronicus to_the_queen troilus_and_cressida twelfth_night two_gentlemen_of_verona
two_noble_kinsmen venus_and_adonis winters_tale
""".split()


def fetch():
    RAW.mkdir(exist_ok=True)
    for work in WORKS:
        path = RAW / f"ps_{work}.xml"
        if path.exists():
            continue
        with urllib.request.urlopen(URL.format(path.name), timeout=60) as response:
            data = response.read()
        if not data.startswith(b"<?xml"):
            sys.exit(f"{path.name}: not XML")
        path.write_bytes(data)


def clean(text):
    return " ".join(text.replace("\u2019", "'").replace("\u2018", "'").split())


def field(text):
    return text.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n")


def line_text(l):
    """A line's words, leaving out any modern translation nested in it."""
    return clean("".join(l.itertext()))


class Work:
    def __init__(self, root):
        titles = {t.get("type"): clean("".join(t.itertext())) for t in root.find("titleStmt").findall("title")}
        self.name = titles["common"]
        self.aliases = [titles[k] for k in ("main", "short", "abbr") if titles.get(k) and titles[k] != self.name]
        self.rows = []

    def add(self, ref, text, speaker, kind):
        if text:
            self.rows.append((ref, text, speaker, kind))


def walk(work, node, place, speaker, last):
    """Every line and stage direction under node, in reading order. `last` holds the latest line number."""
    for child in node:
        if child.tag == "sp":
            who = child.find("speaker")
            walk(work, child, place, who.get("long") if who is not None else "", last)
        elif child.tag == "l" and child.get("variant") != "modern":
            n = child.get("n")
            if n:
                last[0] = n.split(".")[0]
            work.add(f"{place}.{last[0]}", line_text(child), speaker, child.get("form") or "verse")
        elif child.tag == "stage":
            d = child.find("dir")
            work.add(f"{place}.{last[0]}", clean("".join(d.itertext())) if d is not None else "", "", "stage")
        elif child.tag in ("lg", "recite", "song"):
            walk(work, child, place, speaker, last)


def play(root):
    work = Work(root)
    acts = root.findall("act")
    for act in acts:
        for part in act:
            if part.tag in ("scene", "prologue"):
                place = f"{work.name} {act.get('num')}.{part.get('num')}"
                walk(work, part, place, "", ["0"])
    for epilogue in root.iter("epilogue"):
        walk(work, epilogue, f"{work.name} Epilogue", "", ["0"])
    # An epilogue's ref is "<name> Epilogue.5"; make it "<name> Epilogue 5".
    work.rows = [(r.replace(" Epilogue.", " Epilogue "), *rest) for r, *rest in work.rows]
    return work


def poem(root):
    work = Work(root)
    sonnets = root.find("sonnets")
    if sonnets is not None:
        work.name, work.aliases = "Sonnet", ["Sonnets", "Sonn"]
        for sonnet in sonnets.iter("sonnet"):
            for l in sonnet.iter("l"):
                if l.get("variant") != "modern":
                    work.add(f"Sonnet {sonnet.get('num')}.{l.get('n')}", line_text(l), "", l.get("form") or "verse")
        return work
    argument = 0
    for l in root.iter("l"):
        if l.get("variant") == "modern":
            continue
        if l.get("n"):
            work.add(f"{work.name} {l.get('n')}", line_text(l), "", l.get("form") or "verse")
        else:
            argument += 1
            work.add(f"{work.name} Argument {argument}", line_text(l), "", "prose")
    return work


def main():
    fetch()
    works = []
    for name in WORKS:
        root = ET.parse(RAW / f"ps_{name}.xml").getroot()
        works.append(play(root) if root.tag == "play" else poem(root))
    with open(HERE / "records.tsv", "w", encoding="utf-8") as out:
        out.write("#ref\ttext\tspeaker\tkind\n")
        for work in works:
            for row in work.rows:
                out.write("\t".join(field(f) for f in row) + "\n")
    with open(HERE / "aliases.tsv", "w", encoding="utf-8") as out:
        out.write("# A work's name, then the other names it goes by. Written by prepare.py from each file's titles.\n")
        for work in works:
            if work.aliases:
                out.write("\t".join([work.name, *work.aliases]) + "\n")
    rows = sum(len(w.rows) for w in works)
    print(f"{len(works)} works, {rows:,} records", file=sys.stderr)


if __name__ == "__main__":
    main()
