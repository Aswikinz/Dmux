#!/usr/bin/env bash
# T010: shared local-only URL guard for the phase0 harnesses (prove.sh, perf.sh).
# Sourced by both so the rule lives in ONE place. Defines dmux_guard_url only;
# each harness keeps its own refusal message and non-zero exit.
#
# Threat it closes (Priya's T005 re-review R1): the old glob
#   *@127.0.0.1[:/]*|*@localhost[:/]*
# only checked that the URL *contained* a local host prefix. libpq honours valid
# URL syntax that routes the connection ELSEWHERE while keeping that prefix:
#   - multi-host comma lists  ...@127.0.0.1:59999,remote:5433/bank
#     (libpq falls through to the SECOND host, which can be any remote server);
#   - query-string overrides  ...@127.0.0.1:59999/bank?host=remote&port=5432
#     (?host= / ?port= / ?service= / ?options= override the netloc entirely);
#   - an embedded '@' in credentials, which older libpq clients may parse so the
#     real host is not the one the glob saw.
# These harnesses WRITE (a throwaway phase0 schema), so a bypass could point them
# at a real system. The guard must therefore accept ONLY a strict single-host
# local URL and reject everything else BEFORE any connection attempt.
#
# Accepted form (anchored, allow-list — nothing else gets through):
#   postgres://<user>:<password>@<host>:<numeric port>/<dbname>
#     host    = 127.0.0.1 or localhost (lowercase, exactly)
#     user, password, dbname = one or more URI unreserved chars [A-Za-z0-9._~-]
#               plus '%' for percent-encoding; non-empty
#     port    = one or more digits, required
# Because the host and port are matched literally, percent-encoding cannot sneak
# a comma/remote host past them; because '?' ',' '@' '/' and every other char are
# outside the allow-list, there is no query string, no multi-host list, no second
# '@', no path after the dbname, and no whitespace/control characters.

# dmux_guard_url URL — return 0 only for the strict form above, 1 otherwise.
dmux_guard_url() {
  local u="${1:-}"
  local re='^postgres://[A-Za-z0-9._~%-]+:[A-Za-z0-9._~%-]+@(127\.0\.0\.1|localhost):[0-9]+/[A-Za-z0-9._~%-]+$'
  [[ $u =~ $re ]]
}
