#!/bin/bash
# Sourced by build-macos.sh. Keep selection separate for deterministic fixtures.
skylis_signing_identity() {
  if [[ -n "${SKYLIS_SIGNING_IDENTITY+x}" ]]; then
    [[ -n "$SKYLIS_SIGNING_IDENTITY" ]] || { echo 'SKYLIS_SIGNING_IDENTITY must not be empty.' >&2; return 1; }
    printf '%s\n' "$SKYLIS_SIGNING_IDENTITY"
    return
  fi
  local identities candidates count
  identities="$(/usr/bin/security find-identity -v -p codesigning)" || return
  candidates="$(printf '%s\n' "$identities" | sed -nE 's/^[[:space:]]*[0-9]+\) ([0-9A-Fa-f]{40}) "(Apple Development:|Developer ID Application:).*"$/\1/p')"
  count="$(printf '%s\n' "$candidates" | awk 'NF { n++ } END { print n+0 }')"
  case "$count" in
    0) echo 'No eligible signing identity; using ad-hoc signing. Privacy authorization may need re-adding after code changes.' >&2
       printf '%s\n' - ;;
    1) printf '%s\n' "$candidates" ;;
    *) echo 'Multiple signing identities: set SKYLIS_SIGNING_IDENTITY to the desired certificate SHA-1 (security find-identity -v -p codesigning), or explicitly to - for ad-hoc.' >&2
       return 1 ;;
  esac
}
