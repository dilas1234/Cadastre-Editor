#!/usr/bin/env bash
# Seed a small demo fabric near Jos (UTM 32N, around E 488350 N 1100130) into a DEV server:
# control beacons, one posted block of parcels, and one open job with draft parcels.
# Usage: tools/seed-demo.sh [base_url]   (default http://127.0.0.1:8080; CADASTRE_AUTH=disabled)
set -euo pipefail
B="${1:-http://127.0.0.1:8080}"
post() { curl -sf -X POST "$B$1" -H 'content-type: application/json' -d "$2"; }
jid() { python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])'; }
sq() { # sq E N W H -> GeoJSON polygon (UTM 32N)
  python3 - "$@" <<'EOF'
import sys,json
e,n,w,h=map(float,sys.argv[1:5])
print(json.dumps({"type":"Polygon","coordinates":[[[e,n],[e+w,n],[e+w,n+h],[e,n+h],[e,n]]]}))
EOF
}
stamp=$(date +%s)

echo "Control beacons"
i=0
for xy in "488120 1099930" "488600 1099925" "488610 1100340" "488110 1100350" "488355 1100135"; do
  set -- $xy; i=$((i+1))
  post /api/v1/beacons "{\"number\":\"PLC-$stamp-$i\",\"easting\":$1,\"northing\":$2,\"is_control\":true}" >/dev/null || echo "  (beacon $i skipped)"
done

echo "Posted block (2 rows x 6 plots, 30 m x 40 m)"
JOB=$(post /api/v1/jobs "{\"reference\":\"DEMO-$stamp-A\",\"job_type\":\"capture\",\"title\":\"Rayfield layout block A\"}" | jid)
for r in 0 1; do for c in 0 1 2 3 4 5; do
  post "/api/v1/jobs/$JOB/parcels" "{\"geometry\":$(sq $((488200 + c*32)) $((1100060 + r*44)) 30 40),\"stated_area\":1200}" >/dev/null
done; done
for a in submit start_review approve; do post "/api/v1/jobs/$JOB/transition" "{\"action\":\"$a\"}" >/dev/null; done
post "/api/v1/jobs/$JOB/post" '' | python3 -c 'import sys,json;d=json.load(sys.stdin);print("  posted",d["posted"],"parcels:",d["parcel_numbers"][0],"…")'

echo "Open job with draft parcels"
JOB2=$(post /api/v1/jobs "{\"reference\":\"DEMO-$stamp-B\",\"job_type\":\"subdivision\",\"title\":\"Block B subdivision\"}" | jid)
post "/api/v1/jobs/$JOB2/parcels" "{\"geometry\":$(sq 488400 1100060 45 40),\"stated_area\":1800}" >/dev/null
post "/api/v1/jobs/$JOB2/parcels" "{\"geometry\":$(sq 488447 1100060 45 40),\"stated_area\":1800}" >/dev/null
echo "  job DEMO-$stamp-B (draft) with 2 draft parcels"
echo "Done. Open $B/editor"
