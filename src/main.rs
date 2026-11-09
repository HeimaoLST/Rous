mod app;
mod client;
mod command;
mod frontend;
mod input;
mod session;

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(app::run());
    runtime.shutdown_background();
    result
}
