#!/usr/bin/env bash
# READ-ONLY: what a Registry user worked on in a date range, from LRS43 on 10.10.10.5.
#   bash tools/aumentum-inventory/registry-user-activity.sh psatdom 2026-11-10 2026-11-13
# (end date is exclusive: 2026-11-13 means "up to and including 12 November")
# SELECT statements only, READ UNCOMMITTED, nothing is written to the Registry.
# Login: REGISTRY_LRS_SQL_USER / REGISTRY_LRS_SQL_PASSWORD from the repo's .env (never printed).
# Output: lrs-export/registry/user-activity-<user>-<from>-<to>.txt (git-ignored, under lrs-export/)
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
who="${1:?usage: $0 <user> <from yyyy-mm-dd> <to yyyy-mm-dd, exclusive>}"
from="${2:-2026-11-10}"; to="${3:-2026-11-13}"
[[ "$who" =~ ^[A-Za-z0-9._@-]+$ ]] || { echo "user name may only contain letters, digits . _ @ -" >&2; exit 2; }
[[ "$from" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ && "$to" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || { echo "dates must be yyyy-mm-dd" >&2; exit 2; }
get() { { grep -E "^$1=" "$repo/.env" || true; } | head -1 | cut -d= -f2- | sed -e "s/^[\"']//" -e "s/[\"']$//"; }
user="$(get REGISTRY_LRS_SQL_USER)"; pass="$(get REGISTRY_LRS_SQL_PASSWORD)"
[ -n "$user" ] && [ -n "$pass" ] || { echo "REGISTRY_LRS_SQL_USER / REGISTRY_LRS_SQL_PASSWORD missing in .env" >&2; exit 1; }
c=cadastre-mssql-test
docker start $c >/dev/null 2>&1 || { echo "Docker container $c not found: it provides sqlcmd. Use RUN-ME-USER-ACTIVITY.cmd on the REGISTRY server instead." >&2; exit 1; }
tool="$(docker exec $c sh -c 'ls /opt/mssql-tools18/bin/sqlcmd /opt/mssql-tools/bin/sqlcmd 2>/dev/null | head -1')"
docker exec $c sh -c 'printf "openssl_conf = d\n[d]\nssl_conf = s\n[s]\nsystem_default = sd\n[sd]\nMinProtocol = TLSv1\nCipherString = DEFAULT@SECLEVEL=0\n" > /tmp/legacy-tls.cnf'
out="$here/lrs-export/registry"; mkdir -p "$out"
file="$out/user-activity-$who-$from-$to.txt"
q() { # $1 = title, $2 = query
  { echo; echo "===== $1 ====="
    docker exec -e SQLCMDPASSWORD="$pass" -e OPENSSL_CONF=/tmp/legacy-tls.cnf $c "$tool" -S 10.10.10.5,1433 -U "$user" -d LRS43 \
      -C -No -l 15 -W -s '|' -y 0 -Q "SET NOCOUNT ON; SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED; $2" 2>&1 || true
  } | tee -a "$file"
}
echo "Registry activity of '$who' from $from to $to (exclusive), run $(date)" | tee "$file"
W="'%$who%'"

q "1. Columns of lr_process_history (to know which column holds the user)" \
  "SELECT COLUMN_NAME+' '+DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA='LRSAdmin' AND TABLE_NAME='lr_process_history' ORDER BY ORDINAL_POSITION"

# Every text column of lr_process_history is searched for the user name, so the actor column name need not be known.
q "2. Workflow history rows of $who (lr_process_history)" \
  "DECLARE @w nvarchar(max) = ''; SELECT @w = @w + ' OR CONVERT(nvarchar(4000),[' + COLUMN_NAME + ']) LIKE N$W' FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA='LRSAdmin' AND TABLE_NAME='lr_process_history' AND DATA_TYPE IN ('varchar','nvarchar','char','nchar');
   DECLARE @s nvarchar(max) = N'SELECT * FROM LRSAdmin.lr_process_history WITH (NOLOCK) WHERE (task_create >= ''$from'' AND task_create < ''$to'' OR task_complete >= ''$from'' AND task_complete < ''$to'') AND (1=0' + @w + N') ORDER BY task_create, id';
   EXEC sp_executesql @s"

q "3. Distinct transactions $who touched (lr_process_history)" \
  "DECLARE @w nvarchar(max) = ''; SELECT @w = @w + ' OR CONVERT(nvarchar(4000),[' + COLUMN_NAME + ']) LIKE N$W' FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA='LRSAdmin' AND TABLE_NAME='lr_process_history' AND DATA_TYPE IN ('varchar','nvarchar','char','nchar');
   DECLARE @s nvarchar(max) = N'SELECT transaction_id, process_name, COUNT(*) AS steps, MIN(task_create) AS first_step, MAX(ISNULL(task_complete, task_create)) AS last_step FROM LRSAdmin.lr_process_history WITH (NOLOCK) WHERE (task_create >= ''$from'' AND task_create < ''$to'' OR task_complete >= ''$from'' AND task_complete < ''$to'') AND (1=0' + @w + N') GROUP BY transaction_id, process_name ORDER BY MIN(task_create)';
   EXEC sp_executesql @s"

q "4. jBPM task instances assigned to $who (JBPM_TASKINSTANCE)" \
  "SELECT t.ID_, t.NAME_, t.ACTORID_, t.CREATE_, t.START_, t.END_, t.ISCANCELLED_, t.PROCINST_, p.NAME_ AS process, pi.KEY_ AS process_key
   FROM LRSAdmin.JBPM_TASKINSTANCE t WITH (NOLOCK)
   LEFT JOIN LRSAdmin.JBPM_PROCESSINSTANCE pi WITH (NOLOCK) ON pi.ID_ = t.PROCINST_
   LEFT JOIN LRSAdmin.JBPM_PROCESSDEFINITION p WITH (NOLOCK) ON p.ID_ = pi.PROCESSDEFINITION_
   WHERE t.ACTORID_ LIKE $W AND (t.CREATE_ >= '$from' AND t.CREATE_ < '$to' OR t.START_ >= '$from' AND t.START_ < '$to' OR t.END_ >= '$from' AND t.END_ < '$to')
   ORDER BY t.CREATE_"

q "5. Other LRSAdmin tables with a user-like column and a date column (to find documents/files they edited)" \
  "SELECT u.TABLE_NAME, u.COLUMN_NAME AS user_column, d.COLUMN_NAME AS date_column
   FROM INFORMATION_SCHEMA.COLUMNS u JOIN INFORMATION_SCHEMA.COLUMNS d ON d.TABLE_SCHEMA=u.TABLE_SCHEMA AND d.TABLE_NAME=u.TABLE_NAME
   WHERE u.TABLE_SCHEMA='LRSAdmin' AND u.DATA_TYPE IN ('varchar','nvarchar') AND (u.COLUMN_NAME LIKE '%user%' OR u.COLUMN_NAME LIKE '%actor%' OR u.COLUMN_NAME LIKE '%created_by%' OR u.COLUMN_NAME LIKE '%modified_by%' OR u.COLUMN_NAME LIKE '%login%' OR u.COLUMN_NAME LIKE '%officer%')
     AND d.DATA_TYPE IN ('datetime','datetime2','date','smalldatetime') AND u.TABLE_NAME NOT LIKE 'JBPM%'
   ORDER BY u.TABLE_NAME, u.COLUMN_NAME, d.COLUMN_NAME"

q "6. Columns of the transaction tables (to turn transaction_id into a file/application number)" \
  "SELECT TABLE_NAME+'.'+COLUMN_NAME+' '+DATA_TYPE FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA='LRSAdmin' AND (TABLE_NAME LIKE 'lr_transaction%' OR TABLE_NAME LIKE 'lr_application%' OR TABLE_NAME LIKE 'lr_file%') ORDER BY TABLE_NAME, ORDINAL_POSITION"

echo; echo "Saved: $file"
