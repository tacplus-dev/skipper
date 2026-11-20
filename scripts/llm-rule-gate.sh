#!/usr/bin/env bash
# Copyright (c) 2026 Jody Scott
# SPDX-License-Identifier: Apache-2.0

# The LLM-rule gate (#5; tacplus-dev/roadmap decisions/skipper-and-admiral-2026-11-01.md, 2026-11-19): an LLM may
# propose; it never holds a token or a write path. The advisor's side of skipper is src/advisor/ and the two core
# modules it may name, src/core/reads.rs (the read trait the core implements and passes in) and src/core/proposal.rs
# (the typed proposals it returns). This gate fails the build when any of them could reach a state client, a token,
# an S2S client or any other write path:
#
# - a path may start only at std, serde, serde_json, async_trait, tracing, self, crate or a module the advisor
#   declares itself: never at the generated model (skipper_model), a chassis crate (servicemanager, token, service_im,
#   client_im, ...), tonic, tokio or any crate added later, which a change to this list must argue for;
# - a crate path may only be crate::advisor, crate::core::reads or crate::core::proposal, each named in full;
# - no super (it climbs out of the advisor), no absolute ::path, no extern crate, no #[path] anywhere in the crate,
#   no include!, no std::fs, std::net, std::process, std::env or std::os, and no std::{...} or std::* that could
#   hide one;
# - no name that carries a token or a client (ServiceManager, TokenProvider, StateClients, GrpcClients,
#   IdentityManager) and no call of the ServiceManager's client accessors (.s2s(), .grpc(), .state_clients(),
#   .token_provider());
# - reads.rs and proposal.rs re-export nothing and declare no module in another file (an inline `mod tests {...}`
#   is read with them).
#
# It reads the source text, comments included (a comment that needs one of these names can say it in words), with
# the whitespace around `::` and every line break collapsed, so a path split across lines is still one path.
#
# Usage: llm-rule-gate.sh [SRC_DIR]  (default: src). Its fixture test is scripts/llm-rule-gate-test.sh.
set -euo pipefail

src="${1:-src}"
failures=0

fail() {
  echo "llm-rule-gate: $1: $2" >&2
  failures=$((failures + 1))
}

# The file as one line: every run of whitespace a single space, and none around `::`. The one attribute dropped is
# #[tokio::test], which only runs a test on a runtime of its own.
flatten() {
  tr '\n\t\r' '   ' < "$1" | sed -E 's/ +/ /g; s/ ?:: ?/::/g; s/#\[ ?tokio::test ?\]//g'
}

# check FILE LOCAL_MODULES...: the rules above, on one file of the advisor's side.
check() {
  local file="$1"; shift
  local allowed=" std serde serde_json async_trait tracing self crate $* "
  local text root path
  text="$(flatten "$file")"

  local -a denied=(
    '\bsuper\b @@ super (it climbs out of the advisor: name crate::advisor::... instead)'
    '\bextern crate\b @@ extern crate'
    '#!?\[ ?path\b @@ a #[path] attribute'
    '\binclude(_str|_bytes)? ?! @@ include!'
    '\bstd::(fs|net|process|env|os)\b @@ std::fs, std::net, std::process, std::env or std::os'
    '\bstd::[{*] @@ a std::{...} group or std::* glob'
    '(^|[^A-Za-z0-9_:>])::[A-Za-z_{*] @@ an absolute ::path'
    '\b(ServiceManager|TokenProvider|TokenProviderFor|StateClients|GrpcClients|IdentityManager)\b @@ a token- or client-carrying name'
    '\.(s2s|grpc|state_clients|token_provider) ?\( @@ a ServiceManager client accessor'
  )
  local entry pattern what found
  for entry in "${denied[@]}"; do
    pattern="${entry%% @@ *}"
    what="${entry#* @@ }"
    if found="$(grep -oE "$pattern" <<< "$text" | head -1)" && [[ -n "$found" ]]; then
      fail "$file" "$what: '${found}'"
    fi
  done

  # Every path's first segment, where it starts a path (not a segment after another `::`).
  while read -r root; do
    [[ -z "$root" ]] && continue
    if [[ "$allowed" != *" $root "* ]]; then
      fail "$file" "a path from '$root::', which the advisor may not name"
    fi
  done < <({ grep -oE '(^|[^A-Za-z0-9_:])[a-z_][a-z0-9_]*::' <<< "$text" | sed -E 's/^[^a-z_]//; s/::$//'
             grep -oE '(^|[;{}]|\]|\bpub(\([a-z ]+\))?) ?use [a-z_][a-z0-9_]*\b' <<< "$text" | sed -E 's/.*use //'; } | sort -u)

  # Every crate path, in full.
  while read -r path; do
    [[ -z "$path" ]] && continue
    if [[ ! "$path" =~ ^crate::advisor(::.*)?$ && ! "$path" =~ ^crate::core::(reads|proposal)(::.*)?$ ]]; then
      fail "$file" "'$path': the advisor may name only crate::advisor, crate::core::reads and crate::core::proposal, in full"
    fi
  done < <(grep -oE '\bcrate(::[A-Za-z0-9_]+)*(::[{*])?' <<< "$text" | grep -F '::' | sort -u)
}

if [[ ! -d "$src/advisor" ]] || [[ -z "$(find "$src/advisor" -name '*.rs' -print -quit)" ]]; then
  echo "llm-rule-gate: $src/advisor has no Rust source: there is no advisor to check" >&2
  exit 1
fi

# The modules the advisor declares itself, which its paths may start at.
mapfile -t local_mods < <(cat $(find "$src/advisor" -name '*.rs' | sort) \
  | grep -oE '(^|[^A-Za-z0-9_])mod +[a-z_][a-z0-9_]*' | sed -E 's/.*mod +//' | sort -u)

while read -r file; do
  check "$file" "${local_mods[@]}"
done < <(find "$src/advisor" -name '*.rs' | sort)

for file in "$src/core/reads.rs" "$src/core/proposal.rs"; do
  if [[ -e "$file" ]]; then
    check "$file"
    if found="$(flatten "$file" | grep -oE '\bpub(\([a-z ]+\))? use\b|\bmod [a-z_]+ ?;' | head -1)" && [[ -n "$found" ]]; then
      fail "$file" "'$found': the advisor's core modules re-export nothing and declare no module in another file"
    fi
  fi
done
for dir in "$src/core/reads" "$src/core/proposal"; do
  [[ -d "$dir" ]] && fail "$dir" "the advisor's core modules are single files, so this gate reads all of them"
done

# A #[path] anywhere could map another file into the advisor's module, or the advisor's into another.
while read -r file; do
  if flatten "$file" | grep -qE '#!?\[ ?path\b'; then
    fail "$file" "a #[path] attribute"
  fi
done < <(find "$src" -name '*.rs' -not -path "$src/advisor/*" | sort)

if ((failures > 0)); then
  echo "llm-rule-gate: $failures finding(s): the advisor may read and propose, never hold a token or a write path" >&2
  exit 1
fi
echo "llm-rule-gate: $src/advisor names no state client, token or write path"
