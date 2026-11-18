#!/usr/bin/env python3
"""edge_bank: check that every report tells the truth, in every leave-out choice.

    python3 testdata/edge_bank/check_reports.py <dmux binary> <results.json> <out dir>

Owner tool (not a CI test). It classifies every table straight from the results
file (data difference, not mapped, structure, other, pass), renders the HTML and
Excel reports for all 16 combinations of --leave-out, and checks that:
  - the verdict and coverage line never change;
  - every data difference and every not-mapped table is always present;
  - nothing is shown as a data difference unless rows, values, counts or totals
    differ (no false alarm);
  - exactly the chosen parts are left out, and page 1 names them;
  - the tiles and the workbook agree with the results file.
It exits 1 and lists each problem when any check fails.
"""
import json, re, subprocess, sys, itertools, zipfile, html as htmllib, os
DMUX, RES, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
r = json.load(open(RES))
fails = []
def bad(msg): fails.append(msg)

# --- independent classification from the raw results fields ---------------
def kind(path, t):
    if t['verdict'] == 'PASS': return None
    if path in r.get('unmapped', {}): return 'not_mapped'
    if t['verdict'] in ('TOOL_FAULT', 'CHANGED_DURING_CHECK'): return 'other'
    pos = lambda k: (t.get(k) or 0) > 0
    counts = t.get('old_rows') is not None and t.get('new_rows') is not None and t['old_rows'] != t['new_rows']
    totals = bool(r.get('details', {}).get(path, {}).get('control_totals'))
    if any(pos(k) for k in ('missing_rows','extra_rows','changed_rows','differing_buckets','missing_buckets','extra_buckets')) or counts or totals:
        return 'data'
    if any(f['verdict'] != 'PASS' and (f['object'] == path or f['object'].startswith(path + '.')) for f in r['structure_findings']):
        return 'structure'
    return 'other'

K = {p: kind(p, t) for p, t in r['tables'].items()}
data = {p for p, k in K.items() if k == 'data'}
nm = {p for p, k in K.items() if k == 'not_mapped'}
struct = {p for p, k in K.items() if k == 'structure'}
other = {p for p, k in K.items() if k == 'other'}
passing = {p for p, k in K.items() if k is None}
not_checked = r['in_scope'] - (r['fully_checked'] + r['counts_only'] + r['structure_only'] + r['ignored'])
print(f"tables {len(K)}: data {len(data)}, not_mapped {len(nm)}, structure {len(struct)}, other {len(other)}, pass {len(passing)}; not checked {not_checked}")

verdict_word = r['verdict'].replace('_', ' ')
for combo in itertools.product([0, 1], repeat=4):
    st, ot, wa, pa = combo
    parts = [n for n, on in zip(('structure', 'other', 'warnings', 'passing'), combo) if on]
    tag = ','.join(parts) or 'nothing'
    html_p, xlsx_p = f"{OUT}/r-{tag}.html", f"{OUT}/r-{tag}.xlsx"
    args = [DMUX, 'report', '--results', RES, '--html', html_p, '--xlsx', xlsx_p]
    if parts: args += ['--leave-out', ','.join(parts)]
    subprocess.run(args, check=True, capture_output=True)
    h = open(html_p).read()

    # expected kept tables (data and not-mapped always)
    def kept(p):
        k = K[p]; t = r['tables'][p]
        if k in ('data', 'not_mapped'): return True
        if k is None: return not pa
        if k == 'structure' and st: return False
        if k == 'other' and ot: return False
        if wa and t['verdict'] == 'WARNING': return False
        return True
    exp_kept = {p for p in K if kept(p)}

    # 1. verdict and coverage never change
    if f'<div class="word">{verdict_word}' not in h: bad(f"[{tag}] verdict word missing")
    if htmllib.escape(r['coverage_line']) not in h: bad(f"[{tag}] coverage line differs")
    # 2. data-difference section has exactly the data tables
    sec = h[h.index('id="data-differences"'):h.index('id="not-mapped"')]
    shown_data = set(re.findall(r'data-kind="data"[^>]*>.*?<a href="#t-[^"]*">([^<]+)</a>', sec))
    if shown_data != data: bad(f"[{tag}] data section {sorted(shown_data)} != expected {sorted(data)}")
    # 3. not-mapped section has exactly the unmapped tables
    secnm = h[h.index('id="not-mapped"'):h.index('<h2>Other failures</h2>')]
    shown_nm = set(re.findall(r'<strong>([^<]+)</strong>', secnm))
    if shown_nm != nm: bad(f"[{tag}] not-mapped {sorted(shown_nm)} != {sorted(nm)}")
    # 4. other-failures section: exactly the kept structure/other tables
    seco = h[h.index('<h2>Other failures</h2>'):h.index('</section>', h.index('<h2>Other failures</h2>'))]
    shown_o = set(re.findall(r'<a href="#t-[^"]*">([^<]+)</a>', seco))
    exp_o = {p for p in exp_kept if K[p] in ('structure', 'other')}
    if shown_o != exp_o: bad(f"[{tag}] other failures {sorted(shown_o ^ exp_o)} wrong")
    # 5. per-table list: exactly the kept, compared tables
    rows = set(re.findall(r'<tbody class="t f" id="t-([^"]+)"', h))
    exp_rows = {p for p in exp_kept if K[p] != 'not_mapped'}
    if rows != exp_rows: bad(f"[{tag}] table list differs: {sorted(rows ^ exp_rows)}")
    # 6. left-out note present exactly when something was left out
    dropped = set(K) - exp_kept
    left_note = 'Left out of this report when it was made' in h
    findings_dropped = st or (wa and any(f['verdict'] == 'WARNING' for f in r['structure_findings']))
    warns_dropped = wa and bool(r['warnings'])
    if left_note != bool(dropped or findings_dropped or warns_dropped): bad(f"[{tag}] left-out note {left_note} but dropped={len(dropped)}")
    # 7. tiles
    tiles = dict((lab, int(n)) for n, lab in re.findall(r'<div class="tile[^"]*"><strong>(\d+)</strong><span>([^<]+)</span>', h))
    if tiles.get('Data differences') != len(data): bad(f"[{tag}] data tile {tiles.get('Data differences')} != {len(data)}")
    if tiles.get('Other failures') != len(struct | other): bad(f"[{tag}] other tile {tiles.get('Other failures')}")
    if (tiles.get('Not checked', 0)) != not_checked: bad(f"[{tag}] not-checked tile {tiles.get('Not checked')} != {not_checked}")
    if tiles.get('In scope') != r['in_scope']: bad(f"[{tag}] in-scope tile")
    # 8. xlsx kinds
    z = zipfile.ZipFile(xlsx_p)
    strings = re.findall(r'<t[^>]*>([^<]*)</t>', z.read('xl/sharedStrings.xml').decode())
    label = {'data': 'Data differences', 'structure': 'Structure differences', 'other': 'Other reasons', 'not_mapped': 'Not mapped'}
    for p in exp_kept:
        if p not in strings: bad(f"[{tag}] {p} missing from workbook")
    for k in set(K[p] for p in exp_kept if K[p]):
        if label[k] not in strings: bad(f"[{tag}] kind label {label[k]} missing from workbook")

print(f"{16} report variants checked; {len(fails)} problems")
for f in fails[:40]: print("  -", f)
print("\nPer-table classification (for review):")
for p in sorted(K, key=lambda p: (str(K[p]), p)):
    if K[p]: print(f"  {K[p]:<10} {r['tables'][p]['verdict']:<16} {p}: {'; '.join(r['tables'][p]['reasons'])[:120]}")
sys.exit(1 if fails else 0)
