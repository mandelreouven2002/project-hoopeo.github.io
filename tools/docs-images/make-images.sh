#!/bin/bash
# Regenerates the documentation images of the public repository from the real binaries:
# terminal sessions of DEP CLI, DEP Engine and psql, screenshots of Hoopeo Studio, and the concept diagrams.
#
#   BIN=target/release HOOPEO_PUBLIC=public tools/docs-images/make-images.sh
#
# Linux only (Hoopeo Studio runs on a virtual display). Needs: Xvfb, ImageMagick (import), psql,
# Python 3 with Playwright and Pillow, and a Chromium for Playwright (set CHROMIUM to its path if needed).
# Writes $HOOPEO_PUBLIC/docs/images/{terminal,studio,diagrams}/*.png. The images carry no version number.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$(cd "${BIN:-target/release}" && pwd)"
export HOOPEO_PUBLIC="$(cd "${HOOPEO_PUBLIC:-public}" && pwd)"
export CLINIC="$HOOPEO_PUBLIC/examples/clinic.sql"
OUT="$HOOPEO_PUBLIC/docs/images"; RAW="$(mktemp -d)"; trap 'rm -rf "$RAW"' EXIT
mkdir -p "$OUT/terminal" "$OUT/studio" "$OUT/diagrams" "$RAW/terminal" "$RAW/studio" "$RAW/diagrams"
cd "$RAW"

# ---------------------------------------------------------------- terminal sessions
for s in first-steps access-paths transaction indexes; do
  { echo "\$ dep-cli --memory --init examples/clinic.sql -f $s.sql"; "$BIN/dep-cli" --memory --init "$CLINIC" -f "$HERE/cli/$s.sql" 2>&1; } > "t-$s.txt"
done
mv t-indexes.txt t-explain.txt
"$BIN/dep-engine" --listen 127.0.0.1:55433 --data-dir ./data --init "$CLINIC" --admin-password admin > eng.log 2>&1 & EP=$!
for _ in $(seq 40); do sleep 0.25; grep -q listening eng.log && break; done
{ echo '$ dep-engine --data-dir ./data --init examples/clinic.sql --admin-password admin'; sed 's/55433/5433/g' eng.log; } > t-engine-start.txt
Q="SELECT full_name, national_id, health_fund FROM patients WHERE national_id = '123456782';"
{ echo "\$ PGPASSWORD=admin psql -h 127.0.0.1 -p 5433 -U admin -d operational \\"; echo "    -c \"$Q\""; PGPASSWORD=admin psql -h 127.0.0.1 -p 55433 -U admin -d operational -c "$Q" 2>&1; } > t-engine-psql.txt
kill $EP
for n in first-steps access-paths transaction explain; do python3 "$HERE/render_term.py" "t-$n.txt" "terminal/cli-$n.png" "DEP CLI"; done
python3 "$HERE/render_term.py" t-engine-start.txt terminal/engine-start.png "DEP Engine"
python3 "$HERE/render_term.py" t-engine-psql.txt terminal/engine-psql.png "psql"

# ---------------------------------------------------------------- Hoopeo Studio
shot() {   # shot SCENARIO NAME [dark]
  local d=":$((90 + RANDOM % 9))"; Xvfb "$d" -screen 0 1440x900x24 >/dev/null 2>&1 & local x=$!; sleep 1
  local home="$RAW/home/dana"; rm -rf "$home"; mkdir -p "$home"
  local theme=(); [ "${3:-}" = dark ] && theme=(GTK_THEME=Adwaita:dark)
  env HOME="$home" DISPLAY="$d" WEBKIT_DISABLE_COMPOSITING_MODE=1 "${theme[@]}" HOOPEO_STUDIO_SELFTEST="$(python3 "$HERE/studio_scen.py" "$1")" "$BIN/hoopeo-studio" >/dev/null 2>&1 & local a=$!
  sleep "${SECS:-15}"; DISPLAY="$d" import -window root "studio/$2.png"; kill $a $x 2>/dev/null || true; wait $a $x 2>/dev/null || true
}
for s in lookup refused function explain matrix audit newlocal connect; do shot "$s" "studio-$s"; done
shot lookup studio-lookup-dark dark; shot matrix studio-matrix-dark dark
# the two dialogs alone, for the Install page: the white dialog body inside the dimmed window, out to its border
python3 - studio <<'PY'
import sys, numpy as np
from PIL import Image
for n in ('studio-newlocal', 'studio-connect'):
    img = Image.open(f'{sys.argv[1]}/{n}.png'); im = np.asarray(img.convert('L')).astype(int)
    rows = np.where(((im > 238).sum(1) > 300))[0]; rows = rows[rows > 60]
    cols = np.where(((im[rows.min():rows.max()] > 238).sum(0) > 150))[0]
    l, r, top, bot = cols.min(), cols.max(), rows.min(), rows.max()
    col = im[(top + bot) // 2]
    while l > 0 and col[l - 1] > 150 and abs(col[l - 1] - col[l]) < 60: l -= 1
    while r < im.shape[1] - 1 and col[r + 1] > 150 and abs(col[r + 1] - col[r]) < 60: r += 1
    v = im[:, (l + r) // 2]
    while top > 60 and abs(v[top - 1] - v[top]) < 60: top -= 1
    while bot < im.shape[0] - 1 and abs(v[bot + 1] - v[bot]) < 60: bot += 1
    img.crop((l - 1, top - 1, r + 2, bot + 2)).save(f'{sys.argv[1]}/{n}-dialog.png')
PY

# ---------------------------------------------------------------- diagrams
python3 "$HERE/diagrams.py" diagrams
NARROW=1 python3 "$HERE/diagrams.py" diagrams   # the stacked versions, NAME-narrow.png

# ---------------------------------------------------------------- optimize: 256-color PNG, about 20 to 90 KB each
python3 - "$RAW" "$OUT" <<'PY'
import sys, pathlib
from PIL import Image
raw, out = map(pathlib.Path, sys.argv[1:])
for f in sorted(raw.glob('*/*.png')):
    im = Image.open(f).convert('RGBA').quantize(colors=256, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE)
    im.save(out / f.parent.name / f.name, optimize=True); print(out / f.parent.name / f.name)
PY
