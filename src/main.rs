mod fixture;
mod subscriptions;

use anyhow::{Context, Result, bail};
use nautilus_bench::CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [mode] if mode == "allocator-check" => {
            nautilus_bench::check_allocator()?;
            println!("Allocator counters verified");
            Ok(())
        }
        [mode] if mode == "serve" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(fixture::serve()),
        [mode, count] if mode == "subscriptions" => {
            let count = count.parse::<usize>().context("instrument count")?;
            nautilus_common::live::get_runtime()
                .block_on(tokio::task::LocalSet::new().run_until(subscriptions::run(count)))
        }
        _ => bail!("usage: nautilus-bench allocator-check | serve | subscriptions INSTRUMENTS"),
    }
}
