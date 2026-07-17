#!/usr/bin/env sh
#-*-mode: Shell-script; coding: utf-8;-*-
# SPDX-License-Identifier: BlueOak-1.0.0
# Description: Record binary artifact sizes to .build-meta/sizes/history.json,
# write a step summary, and regenerate history.md.
#
# Usage:
#   ci-record-sizes <wasm-file> <win-file> <mac-file>
#
# Tested with: $ nix run .#ci-record-sizes -- artifacts/mitchty-wasm/mitchty_bg.wasm artifacts/mitchty-windows-x86_64/mitchty-windows-x86_64.exe artifacts/mitchty-darwin-aarch64/mitchty-darwin-aarch64
set -eux

WASM_FILE=${1:?'usage: ci-record-sizes <wasm-file> <win-file> <mac-file>'}
shift
WIN_FILE=${1:?'usage: ci-record-sizes <wasm-file> <win-file> <mac-file>'}
shift
MAC_FILE=${1:?'usage: ci-record-sizes <wasm-file> <win-file> <mac-file>'}

HISTORY=".build-meta/sizes/history.json"
HISTORY_MD=".build-meta/sizes/history.md"

# Prefer CI-provided commit info, otherwise fall back to local git for testing purposes.
# This hasn't yet been battle tested so probably wrong. Wanted something to start with.
if [ -n "${GITHUB_SHA:-}" ]; then
  SHORT_SHA=$(printf '%.7s' "$GITHUB_SHA")
  FULL_SHA=$GITHUB_SHA
else
  SHORT_SHA=$(git rev-parse --short HEAD)
  FULL_SHA=$(git rev-parse HEAD)
fi
GIT_REF=${GITHUB_REF:-$(git symbolic-ref HEAD 2> /dev/null || printf 'refs/heads/unknown')}
RUN_ID=${GITHUB_RUN_ID:-0}

version=$(grep -A5 '\[workspace.package\]' Cargo.toml \
  | grep '^version' | sed 's/version = "\(.*\)"/\1/')

DETAIL_DIR=".build-meta/versions/${version}-${SHORT_SHA}"
mkdir -p .build-meta/sizes "$DETAIL_DIR"

# Need to know what the last version-commit was to calc size deltas.
printf '%s\n' "$DETAIL_DIR" > .build-meta/.last-detail-dir

wasm_bytes=$(wc -c < "$WASM_FILE" | tr -d ' ')
win_bytes=$(wc -c < "$WIN_FILE" | tr -d ' ')
mac_bytes=$(wc -c < "$MAC_FILE" | tr -d ' ')

hr() { numfmt --to=iec-i --suffix=B "$1"; }

# Size deltas vs the most recent saved history entry.
if [ -f "$HISTORY" ]; then
  prev_wasm=$(jq '.[-1].builds["mitchty-wasm-bg"].total_bytes        // 0' "$HISTORY")
  prev_win=$(jq '.[-1].builds["mitchty-windows-x86_64"].total_bytes // 0' "$HISTORY")
  prev_mac=$(jq '.[-1].builds["mitchty-darwin-aarch64"].total_bytes // 0' "$HISTORY")
else
  prev_wasm=0
  prev_win=0
  prev_mac=0
fi

delta() {
  _d_curr=$1
  _d_prev=$2
  if [ "$_d_prev" -eq 0 ]; then
    printf 'n/a'
    return
  fi
  _d=$((_d_curr - _d_prev))
  if [ "$_d" -gt 0 ]; then
    printf '+%s' "$(hr "$_d")"
  elif [ "$_d" -lt 0 ]; then
    _d_abs=$((-_d))
    printf -- '-%s' "$(hr "$_d_abs")"
  else
    printf '±0'
  fi
}

wasm_delta=$(delta "$wasm_bytes" "$prev_wasm")
win_delta=$(delta "$win_bytes" "$prev_win")
mac_delta=$(delta "$mac_bytes" "$prev_mac")

# Build and append the JSON entry for the build details.
json_entry=$(jq -n \
  --arg version "$version" \
  --arg commit "$FULL_SHA" \
  --arg short "$SHORT_SHA" \
  --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg ref "$GIT_REF" \
  --arg detail_path "$DETAIL_DIR" \
  --argjson run_id "$RUN_ID" \
  --argjson wasm "$wasm_bytes" \
  --argjson win "$win_bytes" \
  --argjson mac "$mac_bytes" \
  '{
    version:      $version,
    commit:       $commit,
    short_commit: $short,
    date:         $date,
    ref:          $ref,
    run_id:       $run_id,
    detail_path:  $detail_path,
    builds: {
      "mitchty-wasm-bg":        { total_bytes: $wasm },
      "mitchty-windows-x86_64": { total_bytes: $win  },
      "mitchty-darwin-aarch64": { total_bytes: $mac  }
    }
  }')

if [ -f "$HISTORY" ]; then
  jq ". + [$json_entry]" "$HISTORY" > .build-meta/sizes/tmp_h.json \
    && mv .build-meta/sizes/tmp_h.json "$HISTORY"
else
  printf '[%s]\n' "$json_entry" | jq '.' > "$HISTORY"
fi

# Write summary to GITHUB_STEP_SUMMARY in CI, or stdout locally. Hopefully this works.
emit() {
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    cat >> "$GITHUB_STEP_SUMMARY"
  else
    cat
  fi
}

# This is terrible but its ci, so... whatever future mitch good luck at figuring
# this out again.
{
  printf '## Binary Sizes\n\n'
  printf '| Binary | Size | Δ prev |\n'
  printf '|--------|------|--------|\n'
  printf "| \`mitchty_bg.wasm\`            | %s | %s |\n" "$(hr "$wasm_bytes")" "$wasm_delta"
  printf "| \`mitchty-windows-x86_64.exe\` | %s | %s |\n" "$(hr "$win_bytes")" "$win_delta"
  printf "| \`mitchty-darwin-aarch64\`     | %s | %s |\n" "$(hr "$mac_bytes")" "$mac_delta"
  printf '\n'
  if [ -n "${GITHUB_SERVER_URL:-}" ]; then
    printf "**Commit:** \`%s\` | **Version:** \`%s\` | **Detail:** [\`%s\`](%s/%s/tree/%s/%s)\n" \
      "$SHORT_SHA" "$version" "$DETAIL_DIR" \
      "$GITHUB_SERVER_URL" "$GITHUB_REPOSITORY" "$GITHUB_REF_NAME" "$DETAIL_DIR"
  else
    printf "**Commit:** \`%s\` | **Version:** \`%s\` | **Detail:** \`%s\`\n" \
      "$SHORT_SHA" "$version" "$DETAIL_DIR"
  fi
} | emit

printf 'Sizes recorded for %s-%s -> %s\n' "$version" "$SHORT_SHA" "$HISTORY"

# Regenerate history.md from history.json, with newest entry first. Delta vs the
# prior older entry is folded into each size cell.
TAB=$(printf '\t')

# Format a size+delta cell: "9.3MiB +200KiB" / "9.3MiB n/a" / "9.3MiB ±0"
# $1 = raw bytes
# $2 = raw delta bytes as integer or the string "n/a" if there is nil/null prior
fmtcell() {
  _fc_bytes=$1
  _fc_delta=$2
  _fc_hr=$(numfmt --to=iec-i --suffix=B "$_fc_bytes")
  if [ "$_fc_delta" = "n/a" ]; then
    printf '%s n/a' "$_fc_hr"
  elif [ "$_fc_delta" -gt 0 ]; then
    printf '%s +%s' "$_fc_hr" "$(numfmt --to=iec-i --suffix=B "$_fc_delta")"
  elif [ "$_fc_delta" -lt 0 ]; then
    _fc_abs=$((-_fc_delta))
    printf '%s -%s' "$_fc_hr" "$(numfmt --to=iec-i --suffix=B "$_fc_abs")"
  else
    printf '%s ±0' "$_fc_hr"
  fi
}

# I hate while IFS loops so much but whatever this is a write once kinda script.
{
  printf '# Build Size History\n\n'
  printf "Newest first. Generated from \`history.json\` by \`ci-record-sizes\`.\n\n"
  printf '| Version | Commit | Date | WASM | Windows | macOS |\n'
  printf '|---------|--------|------|------|---------|-------|\n'

  # For each entry newest first, emit the delta vs the prior entry. jq index
  # math cause I can barely remember this crap when I do it: after reverse,
  # entry at index $i has its older neighbor at original index $len - 2 - $i so
  # guard against going negative for the oldest row so it gets "n/a" rather than
  # wrapping to the last entry which is dum/wrong.
  jq -r '. as $all | length as $len |
    reverse | to_entries | .[] |
    .key as $i | .value as $c |
    (if ($len - 2 - $i) >= 0 then $all[$len - 2 - $i] else null end) as $p |
    [
      $c.version,
      $c.short_commit,
      ($c.date | split("T")[0]),
      ($c.builds["mitchty-wasm-bg"].total_bytes        | tostring),
      ($c.builds["mitchty-windows-x86_64"].total_bytes | tostring),
      ($c.builds["mitchty-darwin-aarch64"].total_bytes | tostring),
      (if $p then ($c.builds["mitchty-wasm-bg"].total_bytes        - $p.builds["mitchty-wasm-bg"].total_bytes        | tostring) else "n/a" end),
      (if $p then ($c.builds["mitchty-windows-x86_64"].total_bytes - $p.builds["mitchty-windows-x86_64"].total_bytes | tostring) else "n/a" end),
      (if $p then ($c.builds["mitchty-darwin-aarch64"].total_bytes - $p.builds["mitchty-darwin-aarch64"].total_bytes | tostring) else "n/a" end)
    ] | @tsv' "$HISTORY" \
    | while IFS="$TAB" read -r ver sha date wasm win mac wasm_d win_d mac_d; do
      wasm_cell=$(fmtcell "$wasm" "$wasm_d")
      win_cell=$(fmtcell "$win" "$win_d")
      mac_cell=$(fmtcell "$mac" "$mac_d")
      printf "| \`%s\` | \`%s\` | %s | %s | %s | %s |\n" \
        "$ver" "$sha" "$date" "$wasm_cell" "$win_cell" "$mac_cell"
    done
} > "$HISTORY_MD"

printf 'History markdown updated: %s\n' "$HISTORY_MD"
