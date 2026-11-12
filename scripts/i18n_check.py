"""Checks locale files against crates/rf-app/locales/_source.json.

    python scripts/i18n_check.py            # every locale file
    python scripts/i18n_check.py de-DE ja-JP
    python scripts/i18n_check.py --add      # first list new t!("…") texts in _source.json

The UI's source language is English: `t!("New folder")` is a key, and _source.json lists every
key with its id (FNV-1a 32 of the English text, in hex). A locale file is
crates/rf-app/locales/<code>.json: {"<id>": "<translation>", ...}; tr.json is Turkish.

Reports missing / unknown ids, empty values, and placeholders ({n}, {n:03}, {name}, ...) that
differ from the English text. Keys marked "new" in _source.json have no translation yet in most
languages and are only counted. Also reports t!("…") texts in the code that _source.json
doesn't list (--add lists them). Exit code 1 if anything is wrong.
"""
import json
import pathlib
import re
import sys

# Windows consoles default to a legacy code page; texts here are in every script
sys.stdout.reconfigure(encoding="utf-8")

ROOT = pathlib.Path(__file__).resolve().parent.parent
LOC = ROOT / "crates" / "rf-app" / "locales"
PH = re.compile(r"\{[^{}]*\}")
# t!("…") and t!("…", name = value): the literal, as written in Rust
T_CALL = re.compile(r'(?<![A-Za-z0-9_])t!\(\s*"((?:[^"\\]|\\.)*)"')


def fnv(s):
    h = 0x811C9DC5
    for b in s.encode():
        h = ((h ^ b) * 0x01000193) & 0xFFFFFFFF
    return f"{h:08x}"


def unescape(s):
    def rep(m):
        e = m.group(1)
        if e.startswith("u{"):
            return chr(int(e[2:-1], 16))
        return {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}[e]

    return re.sub(r"\\(u\{[0-9a-fA-F]+\}|.)", rep, s)


def code_keys():
    """Every t!("…") text in crates/, in file order (not in // comments)."""
    keys = []
    for p in sorted((ROOT / "crates").rglob("*.rs")):
        for line in p.read_text(encoding="utf-8").splitlines():
            if not line.lstrip().startswith("//"):
                keys += [unescape(m.group(1)) for m in T_CALL.finditer(line)]
    return list(dict.fromkeys(keys))


def check(path, source):
    data = json.loads(path.read_text(encoding="utf-8"))
    problems, untranslated = [], 0
    ids = {e["id"] for e in source}
    for e in source:
        v = data.get(e["id"])
        if v is None:
            if e.get("new"):
                untranslated += 1
            else:
                problems.append(f"missing {e['id']} ({e['en']!r})")
        elif not isinstance(v, str) or not v.strip():
            problems.append(f"empty {e['id']} ({e['en']!r})")
        elif sorted(PH.findall(v)) != sorted(PH.findall(e["en"])):
            problems.append(f"placeholders {e['id']}: {e['en']!r} -> {v!r}")
    problems += [f"unknown id {k}" for k in data if k not in ids]
    return problems, untranslated


def main():
    args = [a for a in sys.argv[1:] if a != "--add"]
    path = LOC / "_source.json"
    source = json.loads(path.read_text(encoding="utf-8"))
    listed = {e["en"] for e in source if "key" not in e}
    missing = [k for k in code_keys() if k not in listed]
    bad = 0
    if missing and "--add" in sys.argv:
        at = max(i for i, e in enumerate(source) if "key" not in e) + 1
        source[at:at] = [{"id": fnv(k), "en": k, "new": True} for k in missing]
        path.write_text(json.dumps(source, ensure_ascii=False, indent=1) + "\n", encoding="utf-8")
        print(f"_source.json: added {len(missing)} new text(s)")
    elif missing:
        print(f"_source.json: {len(missing)} t!() text(s) not listed (run with --add):")
        for k in missing[:20]:
            print("   ", repr(k))
        bad += 1
    files = [LOC / f"{c}.json" for c in args] or sorted(p for p in LOC.glob("*.json") if not p.name.startswith("_"))
    for f in files:
        if not f.exists():
            print(f"{f.stem}: no file")
            bad += 1
            continue
        p, untranslated = check(f, source)
        status = "ok" if not p else f"{len(p)} problem(s)"
        print(f"{f.stem}: {status}" + (f", {untranslated} new text(s) untranslated" if untranslated else ""))
        for line in p[:20]:
            print("   ", line)
        bad += bool(p)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
