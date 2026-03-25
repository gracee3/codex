use clap::Parser;
use codex_responses_bench::Args;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    codex_responses_bench::run(Args::parse()).await
}
