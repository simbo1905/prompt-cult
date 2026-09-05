//! Binary entry point for the reference Prompt Cult Mistral proxy.

use clap::Parser;
use prompt_cult_mistral_proxy::Args;
use prompt_cult_mistral_proxy::run_main;

/// Pre-main process hardening (upstream lineage; see docs/hardening.md).
/// Must run before any threads exist.
#[ctor::ctor]
fn pre_main() {
    prompt_cult_proxy_core::process_hardening::pre_main_hardening();
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    run_main(args)
}
