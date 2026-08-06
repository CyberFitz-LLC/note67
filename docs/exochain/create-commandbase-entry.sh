#!/usr/bin/env bash
# Create the CommandBase entry for Note67 ExoChain meeting receipts.
#
# Needs a bearer token for os.commandbase.ai. Run:
#   CB_TOKEN=<token> ./create-commandbase-entry.sh
# Optionally override the company:
#   CB_TOKEN=<token> CB_COMPANY_ID=<uuid> ./create-commandbase-entry.sh
set -euo pipefail

BASE="${CB_BASE:-https://os.commandbase.ai}"
: "${CB_TOKEN:?set CB_TOKEN to a CommandBase bearer token}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

auth=(-H "Authorization: Bearer $CB_TOKEN" -H "Content-Type: application/json")

# Resolve the company if one was not given.
if [ -z "${CB_COMPANY_ID:-}" ]; then
  echo "Resolving company..."
  companies="$(curl -fsS "${auth[@]}" "$BASE/api/companies")"
  echo "$companies" | python3 -c "
import sys,json
d=json.load(sys.stdin)
rows=d if isinstance(d,list) else d.get('data') or d.get('companies') or []
for c in rows: print(' ', c.get('id'), c.get('name'))
"
  CB_COMPANY_ID="$(echo "$companies" | python3 -c "
import sys,json
d=json.load(sys.stdin)
rows=d if isinstance(d,list) else d.get('data') or d.get('companies') or []
print(rows[0]['id'] if len(rows)==1 else '')
")"
  [ -n "$CB_COMPANY_ID" ] || { echo "More than one company; re-run with CB_COMPANY_ID=<uuid>"; exit 1; }
fi

echo "Creating entry in company $CB_COMPANY_ID..."
curl -fsS -X POST "${auth[@]}" \
  --data-binary "@$HERE/commandbase-entry.json" \
  "$BASE/api/companies/$CB_COMPANY_ID/issues" \
| python3 -c "
import sys,json
d=json.load(sys.stdin)
i=d.get('issue') or d
print('Created:', i.get('identifier') or i.get('id'), '-', i.get('title'))
"
