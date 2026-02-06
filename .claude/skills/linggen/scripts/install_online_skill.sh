#!/bin/bash
# Install a skill from the online registry (with confirmation).
source "$(dirname "$0")/config.sh"

QUERY="$*"
if [ -z "$QUERY" ]; then
    echo "Usage: $0 <skill name or keyword>"
    exit 1
fi

REGISTRY_URL=${LINGGEN_SKILLS_REGISTRY_URL:-"https://linggen-analytics.liangatbc.workers.dev"}
REGISTRY_LIMIT=${LINGGEN_SKILLS_REGISTRY_LIMIT:-200}
REGISTRY_API_KEY=${LINGGEN_SKILLS_REGISTRY_API_KEY:-}
REGISTRY_INSTALLER=${LINGGEN_SKILLS_REGISTRY_INSTALLER:-"linggen-cli"}
REGISTRY_INSTALLER_VERSION=${LINGGEN_SKILLS_REGISTRY_INSTALLER_VERSION:-"1.0.0"}
SKILLS_SH_URL=${LINGGEN_SKILLS_SH_URL:-"https://skills.sh/api/search"}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../../../../" && pwd)"

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

REG_MATCH_COUNT=$(echo "$REG_RESPONSE" | jq -r --arg q "$QUERY" '
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
    | length
')

SKILLS_SH_RESPONSE=""
if [ "$REG_MATCH_COUNT" -lt 10 ]; then
    SKILLS_SH_RESPONSE=$(curl -s -X GET "$SKILLS_SH_URL?q=$ENC_QUERY&limit=10" | tr -d '\000')
    if [ $? -ne 0 ]; then
        echo "Warning: Could not connect to skills.sh at $SKILLS_SH_URL (skipping GitHub fallback)"
        SKILLS_SH_RESPONSE=""
    elif ! echo "$SKILLS_SH_RESPONSE" | jq -e . >/dev/null 2>&1; then
        echo "Warning: skills.sh returned invalid JSON (skipping GitHub fallback)"
        SKILLS_SH_RESPONSE=""
    fi
fi

TMP_JSON=$(mktemp)
echo "$REG_RESPONSE" > "$TMP_JSON"
TMP_SKILLS_SH_JSON=""
if [ -n "$SKILLS_SH_RESPONSE" ]; then
    TMP_SKILLS_SH_JSON=$(mktemp)
    echo "$SKILLS_SH_RESPONSE" > "$TMP_SKILLS_SH_JSON"
fi

PY_SCRIPT=$(mktemp)
cat > "$PY_SCRIPT" <<'PY'
import json
import sys

query = sys.argv[1].lower()
path = sys.argv[2]
skills_sh_path = sys.argv[3] if len(sys.argv) > 3 else ""
target = (sys.argv[4] if len(sys.argv) > 4 else "").lower().strip()

with open(path, "r", encoding="utf-8", errors="ignore") as f:
    data = json.load(f)

skills = data.get("skills") or data.get("results") or data.get("data") or []

def match(s: str) -> bool:
    return query in (s or "").lower()

matches = [
    s for s in skills
    if match(s.get("skill", "")) or match(s.get("url", "")) or match(s.get("ref", ""))
]

skills_sh = []
if skills_sh_path:
    try:
        with open(skills_sh_path, "r", encoding="utf-8", errors="ignore") as f:
            skills_sh_data = json.load(f)
            skills_sh = skills_sh_data.get("skills") or skills_sh_data.get("results") or skills_sh_data.get("data") or []
    except Exception:
        skills_sh = []

def key(url: str, skill: str) -> str:
    return f"{(url or '').lower()}::{(skill or '').lower()}"

registry_keys = set(key(s.get("url", ""), s.get("skill", "")) for s in matches)

skills_sh_matches = []
for s in skills_sh:
    repo_url = f"https://github.com/{s.get('topSource', '')}".strip()
    if key(repo_url, s.get("id", "")) in registry_keys:
        continue
    if match(s.get("id", "")) or match(s.get("name", "")) or match(s.get("topSource", "")):
        skills_sh_matches.append(s)

selected = None
if target:
    for s in matches:
        if (s.get("skill") or "").lower() == target:
            selected = s
            break
    if not selected:
        for s in skills_sh_matches:
            if (s.get("id") or "").lower() == target or (s.get("name") or "").lower() == target:
                selected = s
                break
else:
    if len(matches) >= 1:
        selected = matches[0]
    elif len(skills_sh_matches) >= 1:
        selected = skills_sh_matches[0]

if not selected:
    print("", end="")
    sys.exit(0)

if "skill" in selected:
    print(json.dumps({
        "skill": selected.get("skill"),
        "url": selected.get("url"),
        "ref": selected.get("ref") or "main",
        "source": "linggen",
        "display_name": selected.get("skill")
    }))
else:
    repo_url = f"https://github.com/{selected.get('topSource', '')}".strip()
    print(json.dumps({
        "skill": selected.get("id"),
        "url": repo_url,
        "ref": "main",
        "source": "skillsSh",
        "display_name": selected.get("name") or selected.get("id")
    }))
PY

TARGET_NAME=${LINGGEN_SKILL_NAME:-}
SELECTED_JSON=$(python3 "$PY_SCRIPT" "$QUERY" "$TMP_JSON" "$TMP_SKILLS_SH_JSON" "$TARGET_NAME")

rm -f "$TMP_JSON" "$TMP_SKILLS_SH_JSON" "$PY_SCRIPT"

if [ -z "$SELECTED_JSON" ]; then
    echo "No selection made."
    exit 0
fi

if ! echo "$SELECTED_JSON" | jq -e . >/dev/null 2>&1; then
    echo "Error: Invalid selection data."
    exit 1
fi

SKILL_NAME=$(echo "$SELECTED_JSON" | jq -r '.skill // empty')
SKILL_URL=$(echo "$SELECTED_JSON" | jq -r '.url // empty')
SKILL_REF=$(echo "$SELECTED_JSON" | jq -r '.ref // "main"')
SKILL_SOURCE=$(echo "$SELECTED_JSON" | jq -r '.source // "linggen"')
SKILL_DISPLAY=$(echo "$SELECTED_JSON" | jq -r '.display_name // empty')

if [ -z "$SKILL_NAME" ] || [ -z "$SKILL_URL" ]; then
    echo "Error: Selected skill is missing required fields."
    exit 1
fi


URL_CLEAN=$(echo "$SKILL_URL" | sed -e 's/\.git$//' -e 's/\/$//')
REPO_PATH=$(echo "$URL_CLEAN" | sed -e 's#^https://github.com/##')
OWNER=$(echo "$REPO_PATH" | cut -d'/' -f1)
REPO=$(echo "$REPO_PATH" | cut -d'/' -f2)

if [ -z "$OWNER" ] || [ -z "$REPO" ]; then
    echo "Error: Invalid GitHub URL: $SKILL_URL"
    exit 1
fi

ZIP_URL="https://codeload.github.com/$OWNER/$REPO/zip/$SKILL_REF"
TMP_ZIP=$(mktemp)

spinner() {
    local pid="$1"
    local msg="$2"
    local spin='|/-\\'
    local i=0
    printf "%s " "$msg"
    while kill -0 "$pid" 2>/dev/null; do
        i=$(( (i + 1) % 4 ))
        printf "\b%s" "${spin:$i:1}"
        sleep 0.1
    done
    printf "\b✓\n"
}

curl -s -L -o "$TMP_ZIP" "$ZIP_URL" &
DL_PID=$!
spinner "$DL_PID" "Downloading skill archive"
wait "$DL_PID"
if [ $? -ne 0 ] || [ ! -s "$TMP_ZIP" ]; then
    echo "Error: Failed to download $ZIP_URL"
    rm -f "$TMP_ZIP"
    exit 1
fi

TARGET_DIR="$WORKSPACE_ROOT/.claude/skills/$SKILL_NAME"
rm -rf "$TARGET_DIR"
mkdir -p "$TARGET_DIR"

PY_EXTRACT=$(mktemp)
cat > "$PY_EXTRACT" <<'PY'
import os
import sys
import zipfile

zip_path = sys.argv[1]
skill_name = sys.argv[2]
target_dir = sys.argv[3]

with zipfile.ZipFile(zip_path, "r") as zf:
    entries = zf.namelist()
    skill_root = None
    for name in entries:
        lower = name.lower()
        if (lower.endswith("/skill.md") or lower.endswith("/skill.md".lower())) and f"/{skill_name}/" in lower:
            skill_root = os.path.dirname(name)
            break
        if (name.endswith("/SKILL.md") or name.endswith("/skill.md")) and f"/{skill_name}/" in name:
            skill_root = os.path.dirname(name)
            break

    if not skill_root:
        raise SystemExit(f"Could not find skill '{skill_name}' in the repository (missing SKILL.md).")

    prefix = skill_root + "/"
    for name in entries:
        if not name.startswith(prefix):
            continue
        if name.endswith("/"):
            continue
        rel = name[len(prefix):]
        if not rel or ".." in rel or rel.startswith("/"):
            continue
        dest = os.path.join(target_dir, rel)
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        with zf.open(name) as src, open(dest, "wb") as dst:
            dst.write(src.read())
PY

python3 "$PY_EXTRACT" "$TMP_ZIP" "$SKILL_NAME" "$TARGET_DIR" &
EX_PID=$!
spinner "$EX_PID" "Extracting skill files"
wait "$EX_PID"
RESULT=$?

rm -f "$TMP_ZIP" "$PY_EXTRACT"

if [ $RESULT -ne 0 ]; then
    echo "Error: Failed to extract skill. Ensure the repo contains $SKILL_NAME/SKILL.md."
    exit 1
fi

echo "✓ Skill installed to .claude/skills/$SKILL_NAME"

if [ -n "$REGISTRY_API_KEY" ]; then
    INSTALL_PAYLOAD=$(jq -n \
        --arg url "$SKILL_URL" \
        --arg skill "$SKILL_NAME" \
        --arg ref "$SKILL_REF" \
        --arg installer "$REGISTRY_INSTALLER" \
        --arg installer_version "$REGISTRY_INSTALLER_VERSION" \
        --arg timestamp "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" \
        '{url:$url, skill:$skill, ref:$ref, installer:$installer, installer_version:$installer_version, timestamp:$timestamp}')

    TMP_INSTALL=$(mktemp)
    HTTP_STATUS=$(curl -s -o "$TMP_INSTALL" -w "%{http_code}" -X POST "$REGISTRY_URL/skills/install" \
        -H "Content-Type: application/json" \
        -H "X-API-Key: $REGISTRY_API_KEY" \
        -d "$INSTALL_PAYLOAD")

    if [ "$HTTP_STATUS" -ge 200 ] && [ "$HTTP_STATUS" -lt 300 ]; then
        if jq -e . >/dev/null 2>&1 < "$TMP_INSTALL"; then
            COUNTED=$(jq -r '.counted // empty' < "$TMP_INSTALL")
            if [ "$COUNTED" = "true" ]; then
                echo "✓ Installation recorded in Linggen registry ($SKILL_SOURCE)"
            else
                echo "ℹ Installation recorded (already counted recently)"
            fi
        else
            echo "⚠ Installation recorded, but response was not JSON"
        fi
    else
        echo "⚠ Failed to record installation (HTTP $HTTP_STATUS)"
    fi

    rm -f "$TMP_INSTALL"
else
    echo "ℹ Install count not recorded (set LINGGEN_SKILLS_REGISTRY_API_KEY to enable)"
fi
