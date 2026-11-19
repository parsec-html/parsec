"""Writes src/tables.rs, the Unicode data behind IDNA and normalization.

    python3 scripts/gen-tables.py

The data is Unicode 18.0.0 (Unicode License v3), the version the pinned
WPT IDNA tests are generated from: IdnaMappingTable.txt for the UTS #46
mapping step, UnicodeData.txt for canonical decompositions, combining
classes and General_Category=Mark, DerivedNormalizationProps.txt for
composition exclusions and NFC_Quick_Check, DerivedBidiClass.txt for the
RFC 5893 Bidi rule and DerivedJoiningType.txt for the RFC 5892 ContextJ
rule. Each table is sorted by code point and looked up by binary search;
runs with the same value are merged, so a table stores where each run
starts and the value it has.
"""

import subprocess
import urllib.request

VERSION = "18.0.0"
UCD = f"https://www.unicode.org/Public/{VERSION}/ucd/"
IDNA = f"https://www.unicode.org/Public/{VERSION}/idna/"
MAX = 0x10FFFF


def fetch(url):
    return urllib.request.urlopen(url).read().decode("utf-8")


def fields(text):
    """The data lines of a UCD file, split on ';' with comments dropped."""
    for line in text.splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            yield [f.strip() for f in line.split(";")]


def span(field):
    lo, _, hi = field.partition("..")
    return int(lo, 16), int(hi or lo, 16)


def runs(values):
    """(start, value) runs over every code point of a full dict."""
    out = []
    for cp in range(MAX + 1):
        if not out or values[cp] != out[-1][1]:
            out.append((cp, values[cp]))
    return out


def merge(ranges):
    out = []
    for lo, hi in sorted(ranges):
        if out and lo <= out[-1][1] + 1:
            out[-1][1] = max(out[-1][1], hi)
        else:
            out.append([lo, hi])
    return out


# UnicodeData.txt: combining classes, canonical decompositions, marks.
ccc, decomp, marks = {}, {}, []
first = None
for f in fields(fetch(UCD + "UnicodeData.txt")):
    cp = int(f[0], 16)
    if f[1].endswith(", First>"):
        first = cp
        continue
    lo = first if f[1].endswith(", Last>") else cp
    first = None
    if f[2].startswith("M"):
        marks.append((lo, cp))
    if int(f[3]):
        for c in range(lo, cp + 1):
            ccc[c] = int(f[3])
    if f[5] and not f[5].startswith("<"):
        decomp[cp] = [int(x, 16) for x in f[5].split()]

norm_props = fetch(UCD + "DerivedNormalizationProps.txt")
exclusions, nfc_qc = set(), []
for f in fields(norm_props):
    if f[1] == "Full_Composition_Exclusion":
        lo, hi = span(f[0])
        exclusions.update(range(lo, hi + 1))
    elif f[1] == "NFC_QC":
        nfc_qc.append(span(f[0]))


def full(cp):
    return [d for c in decomp.get(cp, [cp]) for d in (full(c) if c in decomp else [c])]


full_decomp = {cp: full(cp) for cp in decomp}
compose = {
    (d[0], d[1]): cp
    for cp, d in decomp.items()
    if len(d) == 2 and cp not in exclusions
}

# IdnaMappingTable.txt. Deviation is valid here: the URL Standard asks for
# nontransitional processing, where the four deviations are kept as written.
VALID, IGNORED, DISALLOWED, MAPPED, DELTA, PAIRS = 0, 1, 2, 3, 4, 5
idna = {}
mapped_text = []
mapped_at = {}


def mapping(target):
    s = "".join(chr(c) for c in target)
    if s not in mapped_at:
        mapped_at[s] = sum(len(x.encode()) for x in mapped_text)
        mapped_text.append(s)
    offset, length = mapped_at[s], len(s.encode())
    assert offset < 1 << 21 and length < 1 << 8
    return (MAPPED << 29) | (offset << 8) | length


for f in fields(fetch(IDNA + "IdnaMappingTable.txt")):
    lo, hi = span(f[0])
    status = f[1]
    for cp in range(lo, hi + 1):
        if status in ("valid", "deviation"):
            idna[cp] = VALID << 29
        elif status == "ignored":
            idna[cp] = IGNORED << 29
        elif status == "disallowed":
            idna[cp] = DISALLOWED << 29
        elif status == "mapped":
            target = [int(x, 16) for x in f[2].split()]
            if len(target) == 1:
                # A run of single code points each moved by the same amount,
                # like A-Z to a-z, is one entry.
                idna[cp] = ("delta", target[0] - cp)
            else:
                idna[cp] = mapping(target)
        else:
            raise ValueError(status)
assert len(idna) == MAX + 1
# Latin, Greek and Cyrillic alternate capital and small letters: an even
# code point mapped to the next one, which is valid. A run of those pairs
# is one entry too.
for cp in range(0, MAX, 2):
    if idna[cp] == ("delta", 1) and idna[cp + 1] == VALID << 29:
        idna[cp] = idna[cp + 1] = PAIRS << 29

idna_runs = []
for start, v in runs(idna):
    if isinstance(v, tuple):
        delta = v[1]
        assert -(1 << 28) <= delta < 1 << 28
        v = (DELTA << 29) | (delta + (1 << 28))
    idna_runs.append((start, v))

# Bidi classes. The file's @missing lines give the defaults for unassigned
# code points, which matter little here: IDNA disallows those anyway.
BIDI = ["L", "R", "AL", "AN", "EN", "ES", "CS", "ET", "ON", "BN", "NSM", "B", "S", "WS",
        "LRE", "LRO", "RLE", "RLO", "PDF", "LRI", "RLI", "FSI", "PDI"]
LONG = {
    "Left_To_Right": "L", "Right_To_Left": "R", "Arabic_Letter": "AL",
    "Arabic_Number": "AN", "European_Number": "EN", "European_Separator": "ES",
    "Common_Separator": "CS", "European_Terminator": "ET", "Other_Neutral": "ON",
    "Boundary_Neutral": "BN", "Nonspacing_Mark": "NSM", "Paragraph_Separator": "B",
    "Segment_Separator": "S", "White_Space": "WS",
}
bidi_text = fetch(UCD + "extracted/DerivedBidiClass.txt")
bidi = {}
for line in bidi_text.splitlines():
    if line.startswith("# @missing:"):
        rng, value = [x.strip() for x in line[len("# @missing:"):].split(";")]
        lo, hi = span(rng)
        for cp in range(lo, hi + 1):
            bidi[cp] = BIDI.index(LONG[value])
for f in fields(bidi_text):
    lo, hi = span(f[0])
    for cp in range(lo, hi + 1):
        bidi[cp] = BIDI.index(f[1])
assert len(bidi) == MAX + 1

# Joining types: only L, D, R and T matter to ContextJ; the rest are U.
JOINING = ["U", "L", "D", "R", "T"]
joining = {cp: 0 for cp in range(MAX + 1)}
for f in fields(fetch(UCD + "extracted/DerivedJoiningType.txt")):
    if f[1] in JOINING:
        lo, hi = span(f[0])
        for cp in range(lo, hi + 1):
            joining[cp] = JOINING.index(f[1])

ccc_full = {cp: ccc.get(cp, 0) for cp in range(MAX + 1)}


def starts_values(run_list, value_type):
    starts = ", ".join(f"{s:#x}" for s, _ in run_list)
    values = ", ".join(f"{v:#x}" if value_type == "u32" else str(v) for _, v in run_list)
    return starts, values


def flat(ranges):
    return ", ".join(f"{lo:#x}, {hi:#x}" for lo, hi in merge(ranges))


def rust_str(s):
    return '"' + "".join(c if 0x20 <= ord(c) < 0x7F and c not in '"\\' else f"\\u{{{ord(c):x}}}" for c in s) + '"'


out = [
    "// SPDX-License-Identifier: Apache-2.0\n",
    "// Copyright 2026 James Conway\n\n",
    f"// Generated by scripts/gen-tables.py from Unicode {VERSION}. Don't edit.\n\n",
    "//! Unicode data for IDNA and normalization. A run table is two arrays:\n",
    "//! where each run of code points with the same value starts, and the\n",
    "//! value; a code point's value is its run's, found by binary search.\n",
    "//! A range table is sorted, merged ranges, flat: lo, hi, lo, hi.\n\n",
    "/// UTS #46 status runs. The top three bits are the kind: 0 valid\n",
    "/// (deviations included), 1 ignored, 2 disallowed, 3 mapped to the\n",
    "/// `IDNA_MAPPED` bytes at bits 8..29 with the length in bits 0..8,\n",
    "/// 4 mapped to the code point plus bits 0..29 minus 2^28, and 5 mapped\n",
    "/// to the next code point when even and valid when odd.\n",
]
starts, values = starts_values(idna_runs, "u32")
out.append(f"pub(crate) static IDNA_STARTS: &[u32] = &[{starts}];\n\n")
out.append(f"pub(crate) static IDNA_VALUES: &[u32] = &[{values}];\n\n")
out.append("/// The targets of multi-code-point mappings, end to end.\n")
out.append(f"pub(crate) static IDNA_MAPPED: &str = {rust_str(''.join(mapped_text))};\n\n")

bidi_runs = runs(bidi)
starts, values = starts_values(bidi_runs, "u8")
out.append("/// Bidi_Class runs, as indices into `crate::idna::Bidi`.\n")
out.append(f"pub(crate) static BIDI_STARTS: &[u32] = &[{starts}];\n\n")
out.append(f"pub(crate) static BIDI_VALUES: &[u8] = &[{values}];\n\n")

join_runs = runs(joining)
starts, values = starts_values(join_runs, "u8")
out.append("/// Joining_Type runs: 0 U (and C), 1 L, 2 D, 3 R, 4 T.\n")
out.append(f"pub(crate) static JOINING_STARTS: &[u32] = &[{starts}];\n\n")
out.append(f"pub(crate) static JOINING_VALUES: &[u8] = &[{values}];\n\n")

ccc_runs = runs(ccc_full)
starts, values = starts_values(ccc_runs, "u8")
out.append("/// Canonical_Combining_Class runs.\n")
out.append(f"pub(crate) static CCC_STARTS: &[u32] = &[{starts}];\n\n")
out.append(f"pub(crate) static CCC_VALUES: &[u8] = &[{values}];\n\n")

out.append("/// General_Category=Mark, as ranges.\n")
out.append(f"pub(crate) static MARKS: &[u32] = &[{flat(marks)}];\n\n")

out.append("/// NFC_Quick_Check No or Maybe, as ranges: a string with none of\n")
out.append("/// these and its marks in canonical order is already NFC.\n")
out.append(f"pub(crate) static NFC_QC_NO_OR_MAYBE: &[u32] = &[{flat(nfc_qc)}];\n\n")

keys = sorted(full_decomp)
decomp_text, decomp_info, at = [], [], 0
for cp in keys:
    s = "".join(chr(c) for c in full_decomp[cp])
    decomp_text.append(s)
    n = len(s.encode())
    decomp_info.append((at << 8) | n)
    at += n
out.append("/// Code points with a canonical decomposition (Hangul syllables\n")
out.append("/// aside, which are algorithmic), and where each one's full\n")
out.append("/// decomposition sits in `DECOMP_TEXT`: offset << 8 | length.\n")
out.append(f"pub(crate) static DECOMP_KEYS: &[u32] = &[{', '.join(f'{k:#x}' for k in keys)}];\n\n")
out.append(f"pub(crate) static DECOMP_INFO: &[u32] = &[{', '.join(f'{v:#x}' for v in decomp_info)}];\n\n")
out.append(f"pub(crate) static DECOMP_TEXT: &str = {rust_str(''.join(decomp_text))};\n\n")

pairs = sorted(compose)
out.append("/// Primary composites: (first << 21 | second) and what they compose to.\n")
out.append(
    f"pub(crate) static COMPOSE_KEYS: &[u64] = &[{', '.join(f'{(a << 21) | b:#x}' for a, b in pairs)}];\n\n"
)
out.append(
    f"pub(crate) static COMPOSE_VALUES: &[u32] = &[{', '.join(f'{compose[p]:#x}' for p in pairs)}];\n"
)

with open("src/tables.rs", "w") as f:
    f.write("".join(out))
# The tables are plain number arrays, which rustfmt packs into full lines.
subprocess.run(["rustfmt", "--edition", "2024", "src/tables.rs"], check=True)
print(
    f"idna {len(idna_runs)} runs, {len(''.join(mapped_text).encode())} mapped bytes; "
    f"bidi {len(bidi_runs)}; joining {len(join_runs)}; ccc {len(ccc_runs)}; "
    f"decompositions {len(keys)}; compositions {len(pairs)}"
)
