#!/usr/bin/env bash
# Fetches the film and paper data that scripts/film/build.py reads, at the
# commits named here, into the folder given.
#
#   scripts/film/fetch.sh <dir>
#   python3 -I scripts/film/build.py <dir> > crates/mono-tone/src/film/data.rs
#
# SpektraLab's profiles are CC BY-SA 4.0 (Hanze Qiu); the KG3 heat filter's
# transmittance is spektrafilm's (Andrea Volpato), GPL-3.0. Needs gh.
set -euo pipefail

dir=${1:?usage: fetch.sh <dir>}
spektralab=e6fdcdb79c9b1e114e38f2ab12a505152d6e632f
spektrafilm=6cd00c8d
mkdir -p "$dir/profiles"

get() { gh api "repos/$1/contents/$2?ref=$3" --jq .content | base64 -d > "$4"; }

base=engine/resources_product
get JamesQiu2005/SpektraLab "$base/paper_grades.json" $spektralab "$dir/paper_grades.json"
get JamesQiu2005/SpektraLab "$base/NOTICE.md" $spektralab "$dir/NOTICE.md"
for s in kodak_tri_x_400 kodak_tmax_100 kodak_tmax_400 kodak_tmax_p3200 kodak_plus_x_125 \
    kodak_double_x_5222 ilford_hp5_plus_400 ilford_fp4_plus_125 ilford_pan_f_plus_50 \
    ilford_delta_100 ilford_delta_400 ilford_delta_3200 ilford_sfx_200 ilford_xp2_super_400 \
    ilford_ortho_plus_80 fujifilm_neopan_acros_100_ii fujifilm_neopan_400 fujifilm_neopan_1600 \
    ilford_multigrade_iv_rc; do
    get JamesQiu2005/SpektraLab "$base/profiles/$s.json" $spektralab "$dir/profiles/$s.json"
done
get andreavolpato/spektrafilm src/spektrafilm/data/filters/heat_absorbing/schott/KG3.csv \
    $spektrafilm "$dir/KG3.csv"
# The spectra of the colors on spektrafilm's chromaticity grid (Johannes
# Hanika's spectral upsampling, made by vkdt's BSD-2 tool), for a color
# photo seen through a film. Taken through the blob, it is too large for
# the contents API.
sha=$(gh api "repos/andreavolpato/spektrafilm/contents/src/spektrafilm/data/luts/spectral_upsampling?ref=$spektrafilm" \
    --jq '.[] | select(.name == "irradiance_xy_tc.npy") | .sha')
gh api "repos/andreavolpato/spektrafilm/git/blobs/$sha" --jq .content | base64 -d > "$dir/irradiance_xy_tc.npy"
echo $spektralab > "$dir/SOURCE_SHA"
