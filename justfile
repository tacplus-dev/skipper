# Copyright (c) 2026 Jody Scott
# SPDX-License-Identifier: Apache-2.0

# `just gates`: the checks every change to main passes, as the build pipeline runs them in the build image
# tacplus.lock pins.

# The compiler: the build image's tacplus (the release tacplus.lock pins); TACPLUS_BIN overrides it.
TACPLUS := env_var_or_default("TACPLUS_BIN", "tacplus")

default: gates

# Generate target/generated-sources (not committed) with the compiler. Generated code is never committed or
# edited by hand, so generation must leave every tracked file as it found it: the registry lock included,
# which `tacplus registry --proto-dir proto` syncs, in a commit of its own.
generate:
  #!/usr/bin/env bash
  set -euo pipefail
  if [[ -n "$(git ls-files target)" ]]; then
    echo "generate: target/ is generated and must not be committed" >&2; exit 1
  fi
  before="$(git diff HEAD | sha256sum)"
  {{TACPLUS}} build
  if [[ "$(git diff HEAD | sha256sum)" != "$before" ]]; then
    echo "generate: tacplus build changed tracked files:" >&2; git status --short >&2; exit 1
  fi

build: generate
  cargo build --locked

test: build
  cargo test --locked

# The gates every change passes before it lands.
gates: test

clean:
  rm -rf target
