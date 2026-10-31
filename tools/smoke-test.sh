#!/usr/bin/env bash
# End-to-end smoke test of a running cadastre-server (CADASTRE_AUTH=disabled dev mode).
# Walks one job through the whole Phase 1 workflow and checks each step.
# Usage: tools/smoke-test.sh [base_url]   (default http://127.0.0.1:8080)
set -uo pipefail
B="${1:-http://127.0.0.1:8080}"
OUT="${SMOKE_OUT:-$(mktemp -d)}"; mkdir -p "$OUT"
pass=0; fail=0
ok()   { echo "  PASS  $1"; pass=$((pass+1)); }
bad()  { echo "  FAIL  $1"; echo "        $2" | head -c 600; echo; fail=$((fail+1)); }
# req METHOD PATH [JSON] -> sets $code and $body
req() {
  local data=(); [ $# -ge 3 ] && data=(-H 'content-type: application/json' -d "$3")
  body=$(curl -s -o "$OUT/body" -w '%{http_code}' -X "$1" "$B$2" "${data[@]}"); code=$body; body=$(cat "$OUT/body")
}
expect() { # expect CODE "label"
  if [ "$code" = "$1" ]; then ok "$2 ($code)"; else bad "$2: expected $1, got $code" "$body"; fi
}
jget() { python3 -c "import sys,json; d=json.load(sys.stdin); print(eval('d'+sys.argv[1]))" "$1" <<<"$body"; }

REF="SMOKE-$(date +%s)"
echo "cadastre-server smoke test against $B  (job $REF)"

echo "1. Probes"
req GET /health;  expect 200 "liveness /health"
req GET /ready;   expect 200 "readiness /ready (DB reachable)"
req GET /api-docs/openapi.json; expect 200 "OpenAPI document"

echo "2. Live survey check (100 m square traverse, stated 10,000 m2)"
req POST /api/v1/survey/check '{"start":{"x":500000,"y":1000000},"legs":[
  {"type":"line","bearing":0,"distance":100},{"type":"line","bearing":90,"distance":100},
  {"type":"line","bearing":180,"distance":100},{"type":"line","bearing":270,"distance":100}],
  "stated_area":10000}'
expect 200 "POST /survey/check"
echo "        $body" | head -c 300; echo
grep -q '"closes_on_start":true' <<<"$body" && ok "traverse closes" || bad "traverse should close" "$body"

echo "3. Job lifecycle"
req POST /api/v1/jobs "{\"reference\":\"$REF\",\"job_type\":\"new_parcel\",\"title\":\"Smoke test\"}"
expect 201 "create job"
JOB=$(jget "['id']" 2>/dev/null)
[ -n "$JOB" ] || { echo "no job id; aborting"; exit 1; }

req POST "/api/v1/jobs/$JOB/parcels" '{"geometry":{"type":"Polygon","coordinates":[[[500000,1000000],[500100,1000000],[500100,1000100],[500000,1000100],[500000,1000000]]]},"stated_area":10000}'
expect 201 "capture parcel under job"

req POST /api/v1/sessions "{\"job_id\":\"$JOB\"}"
expect 201 "open edit session"
SES=$(jget "['id']" 2>/dev/null)
req POST "/api/v1/sessions/$SES/locks" "{\"resource_type\":\"job\",\"resource_id\":\"$JOB\"}"
expect 201 "lock the job"
req POST /api/v1/sessions "{\"job_id\":\"$JOB\"}"; SES2=$(jget "['id']" 2>/dev/null)
req POST "/api/v1/sessions/$SES2/locks" "{\"resource_type\":\"job\",\"resource_id\":\"$JOB\"}"
expect 409 "second session is refused the same lock"
req POST "/api/v1/sessions/$SES/release" ; [ "$code" -lt 300 ] && ok "release session ($code)" || bad "release session" "$body"

req POST "/api/v1/jobs/$JOB/post"
[ "$code" -ge 400 ] && ok "posting a draft job is refused ($code)" || bad "draft job must not post" "$body"

for a in submit start_review approve; do
  req POST "/api/v1/jobs/$JOB/transition" "{\"action\":\"$a\"}"; expect 200 "transition: $a"
done
req POST "/api/v1/jobs/$JOB/transition" '{"action":"submit"}'
[ "$code" -ge 400 ] && ok "invalid transition from approved refused ($code)" || bad "approved job accepted submit" "$body"

req POST "/api/v1/jobs/$JOB/post"; expect 200 "post approved job"
echo "        $body"
grep -Eq '"PL[0-9]{12}"' <<<"$body" && ok "UPIN allocated in PL+12-digit format" || bad "no PL UPIN" "$body"
UPIN=$(grep -Eo 'PL[0-9]{12}' <<<"$body" | head -1)
[ -n "$UPIN" ] || UPIN="__no_upin__"   # never let an empty pattern match everything

req GET "/api/v1/parcels/search?number=$UPIN"; expect 200 "search parcel by UPIN"
grep -q "$UPIN" <<<"$body" && ok "search finds $UPIN" || bad "search missing $UPIN" "$body"

req GET "/api/v1/jobs/$JOB/events"; expect 200 "job event history"

echo "4. Outputs"
curl -s -o "$OUT/report.pdf" -w '%{http_code}' "$B/api/v1/jobs/$JOB/report.pdf" | grep -q 200 \
  && head -c 8 "$OUT/report.pdf" | grep -q '%PDF-1.4' && ok "job report PDF ($OUT/report.pdf)" || bad "job report PDF" "$(head -c 200 "$OUT/report.pdf")"
curl -s -o "$OUT/siteplan.pdf" -w '%{http_code}' "$B/api/v1/jobs/$JOB/siteplan.pdf" | grep -q 200 \
  && head -c 8 "$OUT/siteplan.pdf" | grep -q '%PDF-1.4' && ok "site plan PDF ($OUT/siteplan.pdf)" || bad "site plan PDF" "$(head -c 200 "$OUT/siteplan.pdf")"

echo "5. Audit chain"
req GET /api/v1/audit/verify; expect 200 "audit chain verify"
echo "        $body" | head -c 300; echo

echo
echo "RESULT: $pass passed, $fail failed   (artifacts in $OUT)"
[ "$fail" -eq 0 ]
