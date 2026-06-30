#!/usr/bin/env bash
# Proves Ayame's "designed to crash" thesis: heavy ops (sort/group) run in
# disposable child processes, so a worker crash — even an uncatchable SIGABRT —
# returns an error but leaves the engine and the on-screen viewport fully alive.
#
# Uses the AYAME_WORKER_CRASH hook so the crash is deterministic (no kill races).
#   AYAME_WORKER_CRASH = panic | abort | hang | exit<N>   (honored by op workers)
set -u

if [ -z "${B:-}" ]; then
  TD=$(cargo metadata --format-version=1 --no-deps 2>/dev/null | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
  B="${TD:-target}/release/ayame"
fi
SP=$(mktemp -d)
F="$SP/data.csv"
P1=8810
P2=8811
export AYAME_CACHE_DIR="$SP/cache"

pass=0; fail=0
check(){ if [ "$1" = "$2" ]; then echo "  PASS: $3 ($1)"; pass=$((pass+1)); else echo "  FAIL: $3 (got '$1' want '$2')"; fail=$((fail+1)); fi; }
code(){ curl -s -o /dev/null -w "%{http_code}" "$1"; }
wait_up(){ curl -s --retry 30 --retry-connrefused --retry-delay 1 "$1" >/dev/null 2>&1; }

"$B" gen "$F" --lines 100000 --quiet

echo "== 1) healthy engine: ops succeed in child workers =="
"$B" serve "$F" --port $P1 >/dev/null 2>&1 &
S1=$!
wait_up "http://127.0.0.1:$P1/api/stat"
check "$(code "http://127.0.0.1:$P1/api/stat")" 200 "/api/stat"
check "$(code "http://127.0.0.1:$P1/api/sort?k=5&numeric=true")" 200 "/api/sort"
check "$(code "http://127.0.0.1:$P1/api/group?k=4")" 200 "/api/group"
echo "  group by col4 (status):"
curl -s "http://127.0.0.1:$P1/api/group?k=4"; echo
kill $S1 2>/dev/null; wait $S1 2>/dev/null

echo "== 2) workers crash with SIGABRT — engine must survive =="
AYAME_WORKER_CRASH=abort "$B" serve "$F" --port $P2 >/dev/null 2>&1 &
S2=$!
wait_up "http://127.0.0.1:$P2/api/stat"
check "$(code "http://127.0.0.1:$P2/api/sort?k=5&numeric=true")" 502 "sort worker SIGABRT -> 502"
check "$(code "http://127.0.0.1:$P2/api/stat")"                 200 "*** engine ALIVE after worker crash ***"
check "$(code "http://127.0.0.1:$P2/api/lines?start=0&count=5")" 200 "*** viewport ALIVE after worker crash ***"
check "$(code "http://127.0.0.1:$P2/api/group?k=4")"            502 "group worker SIGABRT -> 502"
check "$(code "http://127.0.0.1:$P2/api/stat")"                 200 "engine STILL alive after 2nd crash"
kill $S2 2>/dev/null; wait $S2 2>/dev/null

echo ""
echo "RESULT: $pass passed, $fail failed"
rm -rf "$SP"
[ "$fail" -eq 0 ]
