#!/bin/sh
# Downloads the 12 black and white HaldCLUTs of t3mujinpack (MIT, João
# Almeida) at a pinned commit, for the bundled looks (docs/notes/742):
#
#     scripts/t3mujinpack/fetch.sh <folder>
set -eu
out="$1"
mkdir -p "$out"
base="https://raw.githubusercontent.com/t3mujinpack/t3mujinpack/0b421f3e25209ed78253d1724a29cc6255c5e7fe"
for f in "AGFA APX 100" "AGFA APX 25" "Fuji Neopan 1600" "Fuji Neopan Acros 100" "Ilford Delta 100" "Ilford Delta 3200" "Ilford Delta 400" "Ilford FP4 125" "Ilford HP5 Plus 400" "Ilford XP2" "Kodak T-Max 3200" "Kodak Tri-X 400"; do
  enc=$(printf '%s' "t3mujinpack - Black and White - $f.png" | sed 's/ /%20/g')
  curl -sSfL -o "$out/$f.png" "$base/haldcluts/$enc"
done
curl -sSfL -o "$out/LICENSE.txt" "$base/LICENSE.txt"
curl -sSfL -o "$out/README.md" "$base/README.md"
