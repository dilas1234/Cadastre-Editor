#!/bin/bash
# Read-only export of the Aumentum Registry workflow definitions (jBPM 3) from LRS43 on 10.10.10.5.
# SELECT statements only. Nothing is written to the Registry.
# Output: tools/aumentum-inventory/lrs-export/registry/jbpm/  (git-ignored, under lrs-export/)
#   *.tsv  - one file per jBPM definition table (fields separated by TAB)
#   processdefinition/<id>-<name>-v<version>.xml  - the stored process definition files
#   history-*.tsv - counts from lr_process_history (no actor names)
# Needs: the cadastre-mssql-test container (sqlcmd), the PLAGIS LAN, and the sa password on the
# REGISTRY_LRS_SQL_USER / REGISTRY_LRS_SQL_PASSWORD lines of .env.
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT=tools/aumentum-inventory/lrs-export/registry/jbpm
mkdir -p "$OUT/processdefinition"
val() { grep -m1 -E "^$1=" .env | cut -d= -f2- || true; }
PW=$(val REGISTRY_LRS_SQL_PASSWORD); [ -n "$PW" ] || PW=$(val '# *REGISTRY_LRS_SQL_PASSWORD')
SQLUSER=$(val REGISTRY_LRS_SQL_USER); SQLUSER=${SQLUSER:-sa}
[ -n "$PW" ] || { echo "REGISTRY_LRS_SQL_PASSWORD not found in .env"; exit 1; }
echo "Reading LRS43 on 10.10.10.5 as $SQLUSER (read-only SELECTs)"
docker start cadastre-mssql-test >/dev/null
q() { # $1 = output file, $2 = query returning ONE column (a tab-joined row)
  docker exec -e SQLCMDPASSWORD="$PW" -e OPENSSL_CONF=/tmp/oldtls.cnf cadastre-mssql-test \
    /opt/mssql-tools18/bin/sqlcmd -S 10.10.10.5,1433 -No -U "$SQLUSER" -d LRS43 -y 0 -b \
    -Q "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; $2" | sed -e '/^[[:space:]]*$/d' -e '/^-*[[:space:]]*$/d' > "$1"
  echo "  $(basename "$1"): $(wc -l < "$1") rows"
}
T="+CHAR(9)+"   # tab between fields
c() { echo "ISNULL(CONVERT(nvarchar(max),$1),'')"; }   # value as text, NULL -> ''
row() { local s=""; for f in "$@"; do s="${s:+$s$T}$(c "$f")"; done; echo "$s"; }

echo "[1/3] jBPM definition tables"
q "$OUT/processdefinition.tsv" "SELECT $(row ID_ CLASS_ NAME_ DESCRIPTION_ VERSION_ STARTSTATE_) FROM LRSAdmin.JBPM_PROCESSDEFINITION ORDER BY NAME_, VERSION_"
q "$OUT/node.tsv"       "SELECT $(row ID_ CLASS_ NAME_ DESCRIPTION_ PROCESSDEFINITION_ SUPERSTATE_ SUBPROCNAME_ SUBPROCESSDEFINITION_ DECISIONEXPRESSION_ DECISIONDELEGATION ACTION_ SIGNAL_ CREATETASKS_ ENDTASKS_ NODECOLLECTIONINDEX_) FROM LRSAdmin.JBPM_NODE ORDER BY PROCESSDEFINITION_, NODECOLLECTIONINDEX_"
q "$OUT/transition.tsv" "SELECT $(row ID_ NAME_ DESCRIPTION_ PROCESSDEFINITION_ FROM_ TO_ CONDITION_ FROMINDEX_) FROM LRSAdmin.JBPM_TRANSITION ORDER BY PROCESSDEFINITION_, FROM_, FROMINDEX_"
q "$OUT/task.tsv"       "SELECT $(row ID_ NAME_ DESCRIPTION_ PROCESSDEFINITION_ ISBLOCKING_ ISSIGNALLING_ CONDITION_ DUEDATE_ PRIORITY_ ACTORIDEXPRESSION_ POOLEDACTORSEXPRESSION_ TASKNODE_ STARTSTATE_ ASSIGNMENTDELEGATION_ SWIMLANE_) FROM LRSAdmin.JBPM_TASK ORDER BY PROCESSDEFINITION_, ID_"
q "$OUT/swimlane.tsv"   "SELECT $(row ID_ NAME_ ACTORIDEXPRESSION_ POOLEDACTORSEXPRESSION_ ASSIGNMENTDELEGATION_ TASKMGMTDEFINITION_) FROM LRSAdmin.JBPM_SWIMLANE ORDER BY ID_"
q "$OUT/moduledefinition.tsv" "SELECT $(row ID_ CLASS_ NAME_ PROCESSDEFINITION_ STARTTASK_) FROM LRSAdmin.JBPM_MODULEDEFINITION ORDER BY ID_"
q "$OUT/delegation.tsv" "SELECT $(row ID_ CLASSNAME_ CONFIGURATION_ CONFIGTYPE_ PROCESSDEFINITION_) FROM LRSAdmin.JBPM_DELEGATION ORDER BY ID_"
q "$OUT/event.tsv"      "SELECT $(row ID_ EVENTTYPE_ TYPE_ GRAPHELEMENT_ PROCESSDEFINITION_ NODE_ TRANSITION_ TASK_) FROM LRSAdmin.JBPM_EVENT ORDER BY ID_"
q "$OUT/action.tsv"     "SELECT $(row ID_ class NAME_ ACTIONEXPRESSION_ REFERENCEDACTION_ ACTIONDELEGATION_ EVENT_ PROCESSDEFINITION_ TIMERNAME_ DUEDATE_ REPEAT_ TRANSITIONNAME_ EXPRESSION_ EVENTINDEX_) FROM LRSAdmin.JBPM_ACTION ORDER BY ID_"

echo "[2/3] stored processdefinition.xml files"
q "$OUT/bytearray.tsv" "SELECT $(row a.ID_ a.NAME_ m.PROCESSDEFINITION_) FROM LRSAdmin.JBPM_BYTEARRAY a LEFT JOIN LRSAdmin.JBPM_MODULEDEFINITION m ON m.ID_ = a.FILEDEFINITION_ ORDER BY a.ID_"
q "$OUT/byteblock.hex" "SELECT CONVERT(varchar(20),b.PROCESSFILE_)+CHAR(9)+CONVERT(varchar(10),b.INDEX_)+CHAR(9)+CONVERT(varchar(max),b.BYTES_,2) FROM LRSAdmin.JBPM_BYTEBLOCK b JOIN LRSAdmin.JBPM_BYTEARRAY a ON a.ID_ = b.PROCESSFILE_ WHERE a.NAME_ LIKE '%.xml' ORDER BY b.PROCESSFILE_, b.INDEX_"
python3 - "$OUT" <<'EOF'
import sys, os, collections, re
out = sys.argv[1]
names = {}
for line in open(f"{out}/bytearray.tsv", encoding="utf-8", errors="replace"):
    f = line.rstrip("\n").split("\t")
    if len(f) >= 3: names[f[0].strip()] = (f[1].strip(), f[2].strip())
defs = {}
for line in open(f"{out}/processdefinition.tsv", encoding="utf-8", errors="replace"):
    f = line.rstrip("\n").split("\t")
    if len(f) >= 5: defs[f[0].strip()] = (f[2].strip(), f[4].strip())
blocks = collections.defaultdict(list)
for line in open(f"{out}/byteblock.hex"):
    f = line.strip().split("\t")
    if len(f) == 3: blocks[f[0]].append((int(f[1]), bytes.fromhex(f[2])))
n = 0
for fid, parts in blocks.items():
    fname, pd = names.get(fid, ("file", ""))
    pname, ver = defs.get(pd, ("process" + pd, "?"))
    safe = re.sub(r"[^A-Za-z0-9._-]+", "_", f"{pd}-{pname}-v{ver}-{fname}")
    open(f"{out}/processdefinition/{safe}", "wb").write(b"".join(b for _, b in sorted(parts)))
    n += 1
print(f"  wrote {n} definition files")
EOF

echo "[3/3] lr_process_history counts (no actor names)"
q "$OUT/history-task-transition.tsv" "SELECT $(row process_name task_name task_description task_transition 'COUNT(*)' 'AVG(CAST(duration AS float))' 'MIN(task_create)' 'MAX(task_create)') FROM LRSAdmin.lr_process_history WITH (NOLOCK) GROUP BY process_name, task_name, task_description, task_transition ORDER BY process_name, task_name"
q "$OUT/history-next-task.tsv" "WITH h AS (SELECT process_name, task_name, task_transition, LEAD(task_name) OVER (PARTITION BY transaction_id, process_id ORDER BY task_create, id) AS next_task FROM LRSAdmin.lr_process_history WITH (NOLOCK)) SELECT $(row process_name task_name task_transition next_task 'COUNT(*)') FROM h GROUP BY process_name, task_name, task_transition, next_task ORDER BY process_name, task_name, COUNT(*) DESC"
q "$OUT/history-open-tasks.tsv" "SELECT $(row process_name task_name 'COUNT(*)') FROM LRSAdmin.lr_process_history WITH (NOLOCK) WHERE task_complete IS NULL GROUP BY process_name, task_name ORDER BY process_name, COUNT(*) DESC"
echo "done -> $OUT"
