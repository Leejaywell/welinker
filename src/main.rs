mod agent;
mod api;
mod cmd;
mod config;
mod ilink;
mod messaging;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    cmd::execute().await
}
