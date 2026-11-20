#!/usr/bin/env bash
# Copyright (c) 2026 Jody Scott
# SPDX-License-Identifier: Apache-2.0

# The LLM-rule gate's fixture test (#5): a clean advisor passes, and each way of reaching a state client, a token
# or a write path from the advisor's side fails. Run by `just llm-rule`, before the gate runs on src/.
set -euo pipefail

gate="$(cd "$(dirname "$0")" && pwd)/llm-rule-gate.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

# A source tree the gate passes: an advisor with a submodule, the read trait and the proposal types, and a core
# that is free to hold anything.
clean_tree() {
  local root="$1"
  mkdir -p "$root/advisor" "$root/core"
  cat > "$root/advisor/mod.rs" <<'EOF'
//! The advisor: it reads through the trait the core passes in, and proposes.
pub mod tools;

use std::sync::Arc;

use crate::advisor::tools::dispatch;
use crate::core::proposal::Proposal;
use crate::core::reads::AdvisorReads;

pub struct Advisor {
    reads: Arc<dyn AdvisorReads>,
}

impl Advisor {
    pub async fn ask(&self, input: &str) -> Vec<Proposal> {
        let value: serde_json::Value = serde_json::from_str(input).unwrap_or_default();
        tracing::debug!(?value, "asked");
        tools::dispatch(&self.reads, input).await;
        dispatch(&self.reads, input).await;
        self::tools::dispatch(&self.reads, input).await;
        Vec::<Proposal>::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::advisor::Advisor;

    #[test]
    fn builds() {
        let _ = std::mem::size_of::<Advisor>();
    }
}
EOF
  cat > "$root/advisor/tools.rs" <<'EOF'
use std::collections::HashMap;
use std::sync::Arc;

use crate::core::reads::AdvisorReads;

pub async fn dispatch(reads: &Arc<dyn AdvisorReads>, input: &str) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    seen.insert(input.to_string(), 1);
    let _ = reads.redacted_config(input).await;
    let _ = <String as std::str::FromStr>::from_str(input);
}
EOF
  cat > "$root/core/reads.rs" <<'EOF'
#[async_trait::async_trait]
pub trait AdvisorReads: Send + Sync {
    async fn redacted_config(&self, trace_id: &str) -> Result<String, String>;
}
EOF
  cat > "$root/core/proposal.rs" <<'EOF'
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum Proposal {
    Classification { rationale: String },
}

#[cfg(test)]
mod tests {
    use crate::core::proposal::Proposal;

    #[tokio::test]
    async fn parses() {
        let _: Proposal = serde_json::from_str("{}").unwrap();
    }
}
EOF
  cat > "$root/core/mod.rs" <<'EOF'
pub mod proposal;
pub mod reads;

use skipper_model::s2sm::ServiceManager;
use std::fs;
EOF
  cat > "$root/lib.rs" <<'EOF'
pub mod advisor;
pub mod core;
EOF
}

# VERBOSE=1 prints what the gate said on every case.
expect() {
  local want="$1" name="$2" root="$3" out
  if out="$("$gate" "$root" 2>&1)"; then got=pass; else got=fail; fi
  [[ -n "${VERBOSE:-}" ]] && printf "%s\n" "$out" | sed "s/^/    /"
  if [[ "$got" != "$want" ]]; then
    echo "FAIL $name: the gate said $got, want $want" >&2
    printf '%s\n' "$out" | sed 's/^/    /' >&2
    failures=$((failures + 1))
  else
    echo "ok   $name ($want)"
  fi
}

# case NAME FILE LINE: a clean tree with LINE appended to FILE (relative to the tree) must fail the gate.
case_fails() {
  local name="$1" file="$2" line="$3" root="$work/$1"
  clean_tree "$root"
  mkdir -p "$(dirname "$root/$file")"
  printf '%s\n' "$line" >> "$root/$file"
  expect fail "$name" "$root"
}

clean_tree "$work/clean"
expect pass clean "$work/clean"

# A missing advisor is not a clean one: the gate checks something or fails.
mkdir -p "$work/no-advisor/core"
expect fail no-advisor "$work/no-advisor"

# The generated model and chassis: the ServiceManager (state clients, S2S clients, the identity manager), the token
# chassis, the identity managers, the gRPC transport.
case_fails generated-model          advisor/mod.rs 'use skipper_model::s2sm::ServiceManager;'
case_fails generated-model-inline   advisor/tools.rs 'fn f(_: &skipper_model::s2sm::ServiceManager) {}'
case_fails servicemanager-chassis   advisor/mod.rs 'use servicemanager::ChannelProvider;'
case_fails token-chassis            advisor/mod.rs 'use token::Token;'
case_fails service-im               advisor/mod.rs 'use service_im::IdentityManager;'
case_fails client-im                advisor/mod.rs 'use client_im::Manager;'
case_fails tonic                    advisor/mod.rs 'use tonic::transport::Channel;'
case_fails reqwest                  advisor/tools.rs 'async fn f() { let _ = reqwest::get("https://example.com").await; }'
case_fails tokio                    advisor/tools.rs 'fn f() { tokio::spawn(async {}); }'
case_fails absolute-path            advisor/mod.rs 'fn f(_: &::skipper_model::s2sm::ServiceManager) {}'
case_fails extern-crate             advisor/mod.rs 'extern crate skipper_model as m;'
case_fails crate-alias              advisor/mod.rs 'use servicemanager as sm;'

# Names that carry a token or a write path, wherever they come from.
case_fails token-provider           advisor/tools.rs 'fn f(_: TokenProvider) {}'
case_fails state-clients            advisor/tools.rs 'fn f(_: &StateClients) {}'
case_fails service-manager-name     advisor/tools.rs 'fn f(_: &ServiceManager) {}'
case_fails s2s-accessor             advisor/tools.rs 'fn f(sm: &X) { sm.s2s(); }'

# The rest of the crate: only the read trait and the proposal types, each named in full.
case_fails core-root                advisor/mod.rs 'use crate::core;'
case_fails core-other-module        advisor/mod.rs 'use crate::core::llmproxy::LlmProxyModel;'
case_fails core-group               advisor/mod.rs 'use crate::core::{reads, llmproxy};'
case_fails crate-group              advisor/mod.rs 'use crate::{advisor, core};'
case_fails crate-glob               advisor/mod.rs 'use crate::*;'
case_fails crate-service            advisor/mod.rs 'use crate::service::SkipperHandler;'
case_fails super-escape             advisor/mod.rs 'use super::core::llmproxy;'
case_fails bare-core                advisor/tools.rs 'fn f() { core::llmproxy::make(); }'
case_fails split-across-lines       advisor/mod.rs $'use crate::core::\n    llmproxy::LlmProxyModel;'
case_fails spaced-path              advisor/mod.rs 'use crate :: core :: llmproxy :: LlmProxyModel;'

# Writes and side doors out of the process, and code pulled in from elsewhere.
case_fails std-fs                   advisor/tools.rs 'fn f() { let _ = std::fs::write("/tmp/x", "y"); }'
case_fails std-net                  advisor/tools.rs 'use std::net::TcpStream;'
case_fails std-process              advisor/tools.rs 'fn f() { let _ = std::process::Command::new("sh"); }'
case_fails std-env                  advisor/tools.rs 'fn f() { let _ = std::env::var("TOKEN"); }'
case_fails std-os                   advisor/tools.rs 'use std::os::unix::net::UnixStream;'
case_fails std-group                advisor/tools.rs 'use std::{fs, io};'
case_fails std-glob                 advisor/tools.rs 'use std::*;'
case_fails include                  advisor/tools.rs 'include!("../core/mod.rs");'
case_fails include-str              advisor/tools.rs 'const S: &str = include_str!("/var/run/secrets/token");'
case_fails path-attribute           advisor/mod.rs '#[path = "../core/llmproxy.rs"] mod proxy;'
case_fails path-attribute-elsewhere lib.rs '#[path = "core/llmproxy.rs"] pub mod advisor_tools;'

# The read trait and the proposal types are the advisor's too: they may not hand it a handle either.
case_fails reads-hands-a-handle     core/reads.rs 'pub fn sm() -> &'"'"'static skipper_model::s2sm::ServiceManager { todo!() }'
case_fails reads-reexports          core/reads.rs 'pub use crate::core::llmproxy::LlmProxyModel;'
case_fails reads-file-module         core/reads.rs 'pub mod handles;'
case_fails tokio-beyond-test        advisor/tools.rs '#[tokio::main] async fn main() {}'
case_fails proposal-hands-a-token   core/proposal.rs 'pub struct Approval { pub token: token::Token }'

if ((failures > 0)); then
  echo "llm-rule-gate-test: $failures case(s) failed" >&2
  exit 1
fi
echo "llm-rule-gate-test: all cases passed"
