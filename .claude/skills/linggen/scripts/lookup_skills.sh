#!/bin/bash
# Search for skills in the local Linggen library and the online skills registry.
source "$(dirname "$0")/config.sh"

QUERY="$*"
if [ -z "$QUERY" ]; then
    echo "Usage: $0 <skill name or keyword>"
    exit 1
fi

REGISTRY_URL=${LINGGEN_SKILLS_REGISTRY_URL:-"https://linggen-analytics.liangatbc.workers.dev"}
REGISTRY_LIMIT=${LINGGEN_SKILLS_REGISTRY_LIMIT:-200}
SKILLS_SH_URL=${LINGGEN_SKILLS_SH_URL:-"https://skills.sh/api/search"}

echo "## Local Library Packs (skills) matching: $QUERY"
echo ""

LIB_RESPONSE=$(curl -s -X GET "$API_URL/api/library" | tr -d '\000')
if [ $? -ne 0 ]; then
    echo "Warning: Could not connect to linggen-server at $API_URL (skipping local library search)"
else
    if ! echo "$LIB_RESPONSE" | jq -e . >/dev/null 2>&1; then
        echo "Warning: Linggen server returned invalid JSON (skipping local library search)"
    else
        echo "$LIB_RESPONSE" | jq -r --arg q "$QUERY" '
        def packs:
            if type=="array" then .
            elif type=="object" and (.packs|type=="array") then .packs
            else []
            end;
        def q: ($q | ascii_downcase);
        def match(s): (s | tostring | ascii_downcase | contains(q));
        packs
        | map(select(
            ((.folder // "") | ascii_downcase | contains("skills")) or
            ((.id // "") | ascii_downcase | contains("/skills/"))
        ))
        | map(select(
            match(.name // "") or match(.id // "") or match(.description // "")
        ))
        | .[0:20]
        | if length == 0 then
            "No matching packs found."
          else
            .[] | "### \(.name)\n- ID: \(.id)\n- Folder: \(.folder // "root")\n- Description: \(.description // "No description")\n"
          end
    '
    fi
fi

echo ""
echo "## Online Skills Registry matching: $QUERY"
echo ""

ENC_QUERY=$(python3 - "$QUERY" <<'PY'
import sys, urllib.parse
print(urllib.parse.quote(sys.argv[1], safe=""))
PY
)

REG_RESPONSE=$(curl -s -X GET "$REGISTRY_URL/skills/search?q=$ENC_QUERY&limit=$REGISTRY_LIMIT" | tr -d '\000')
if [ $? -ne 0 ]; then
    echo "Error: Could not connect to skills registry at $REGISTRY_URL"
    exit 1
fi
if ! echo "$REG_RESPONSE" | jq -e . >/dev/null 2>&1; then
    echo "Error: Skills registry returned invalid JSON"
    exit 1
fi

REG_SKILLS=$(echo "$REG_RESPONSE" | jq --arg q "$QUERY" '
    def q: ($q | ascii_downcase);
    def match(s): (s | tostring | ascii_downcase | contains(q));
    def skills:
        if type=="array" then .
        elif type=="object" and (.skills|type=="array") then .skills
        elif type=="object" and (.results|type=="array") then .results
        elif type=="object" and (.data|type=="array") then .data
        else []
        end;
    skills
    | map(select(
        match(.skill // "") or match(.url // "") or match(.ref // "")
    ))
')

echo "$REG_RESPONSE" | jq -r --arg q "$QUERY" '
    def skills:
        if type=="array" then .
        elif type=="object" and (.skills|type=="array") then .skills
        elif type=="object" and (.results|type=="array") then .results
        elif type=="object" and (.data|type=="array") then .data
        else []
        end;
    def match(s): (s | tostring | ascii_downcase | contains($q | ascii_downcase));
    if (skills | type) != "array" then
        "Error: Unexpected registry response."
    else
        skills
        | map(select(
            match(.skill // "") or match(.url // "") or match(.ref // "")
        ))
        | .[0:20]
        | if length == 0 then
            "No matching online skills found."
          else
            .[] | "### \(.skill)\n- URL: \(.url)\n- Ref: \(.ref // "main")\n- Installs: \(.install_count)\n- Updated: \(.updated_at)\n"
          end
    end
'

REG_COUNT=$(echo "$REG_SKILLS" | jq 'length')

if [ "$REG_COUNT" -lt 10 ]; then
    echo ""
    echo "## GitHub Skills (via skills.sh) matching: $QUERY"
    echo ""

    SKILLS_SH_RESPONSE=$(curl -s -X GET "$SKILLS_SH_URL?q=$ENC_QUERY&limit=10" | tr -d '\000')
    if [ $? -ne 0 ]; then
        echo "Warning: Could not connect to skills.sh at $SKILLS_SH_URL"
        exit 0
    fi
    if ! echo "$SKILLS_SH_RESPONSE" | jq -e . >/dev/null 2>&1; then
        echo "Warning: skills.sh returned invalid JSON"
        exit 0
    fi

    echo "$SKILLS_SH_RESPONSE" | jq -r --argjson reg "$REG_SKILLS" '
        def key(url; skill): (url + "::" + skill) | ascii_downcase;
        def reg_keys: [ $reg[] | select(.url and .skill) | key(.url; .skill) ];
        def skills:
            if type=="array" then .
            elif type=="object" and (.skills|type=="array") then .skills
            elif type=="object" and (.results|type=="array") then .results
            elif type=="object" and (.data|type=="array") then .data
            else []
            end;
        skills
        | map(select(
            (key(("https://github.com/" + (.topSource // "")); (.id // "")) | . as $k | (reg_keys | index($k) | not))
        ))
        | .[0:20]
        | if length == 0 then
            "No additional GitHub skills found."
          else
            .[] | "### \(.name // .id)\n- URL: https://github.com/\(.topSource)\n- Skill: \(.id)\n- Installs: \(.installs)\n"
          end
    '
fi
