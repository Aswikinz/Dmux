#!/usr/bin/env bash
# edge_bank: check that Dmux gives the verdict the guide promises for every case.
#
#   testdata/edge_bank/check.sh            # quick: the 3 million row table is only counted (about a minute)
#   EDGE_FULL=1 testdata/edge_bank/check.sh   # full: reads every row of it too (several minutes)
#
# It runs the Dmux command line against the edge_bank databases (build them first
# with build.sh) and compares each case in cases.tsv with the results file.
#
# This is an owner tool, not a CI test. It exits 0 and says "skipped" when the
# databases are absent, and it refuses to run when CI is set unless
# DMUX_EDGE_CHECK=1 is also set. It only reads the databases.
set -euo pipefail
unset PGHOSTADDR PGSERVICE PGSERVICEFILE PGHOST PGPORT PGDATABASE PGUSER PGPASSWORD
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"

if [[ -n "${CI:-}" && "${DMUX_EDGE_CHECK:-}" != "1" ]]; then
  echo "edge_bank check: skipped (CI is set; set DMUX_EDGE_CHECK=1 to run it)"
  exit 0
fi
command -v python3 >/dev/null 2>&1 || { echo "edge_bank check: python3 is required" >&2; exit 1; }

export EDGE_READER_PASSWORD="${EDGE_READER_PASSWORD:-edge_reader_only}"
OLD_URL="postgres://edge_reader:$EDGE_READER_PASSWORD@127.0.0.1:${EDGE_OLD_PORT:-5433}/edge_bank"
NEW_URL="postgres://edge_reader:$EDGE_READER_PASSWORD@127.0.0.1:${EDGE_NEW_PORT:-5434}/edge_bank"
for u in "$OLD_URL" "$NEW_URL"; do
  if ! psql -X -q -A -t -c "SELECT 1" "$u" >/dev/null 2>&1; then
    echo "edge_bank check: skipped (cannot reach edge_bank as edge_reader; run testdata/edge_bank/build.sh first)"
    exit 0
  fi
done

DEF="$HERE/run-quick.toml"
[[ "${EDGE_FULL:-}" = "1" ]] && DEF="$HERE/run.toml"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "edge_bank check: running $(basename "$DEF") (this takes a while)"
set +e
(cd "$ROOT" && cargo run -q -p dmux-cli -- run "$DEF" --out "$TMP/results.json")
code=$?
set -e
# Exit 1 (FAIL) and 2 (warnings) are expected: the playground is full of differences.
if [[ ! -s "$TMP/results.json" ]]; then
  echo "edge_bank check: FAIL, no results file (dmux exit code $code)" >&2
  exit 1
fi

python3 -I - "$TMP/results.json" "$HERE/cases.tsv" <<'PY'
import json, sys
res = json.load(open(sys.argv[1]))
tables = res["tables"]
findings = {f["object"]: f["verdict"] for f in res["structure_findings"]}
ok = bad = 0
for line in open(sys.argv[2]):
    if not line.strip() or line.startswith("#"):
        continue
    kind, obj, want = line.rstrip("\n").split("\t")
    if kind == "table":
        got = tables.get(obj, {}).get("verdict")
    else:
        got = findings.get(obj)
    if got == want:
        ok += 1
        print(f"  ok    {kind:8} {obj}: {want}")
    else:
        bad += 1
        print(f"  FAIL  {kind:8} {obj}: expected {want}, got {got}")
print(f"{ok} passed, {bad} failed (overall verdict {res['verdict']})")
sys.exit(1 if bad else 0)
PY
