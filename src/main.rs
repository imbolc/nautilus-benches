mod feather;

use anyhow::{Result, bail};
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
        [mode, path, count] if mode == "feather-direct" || mode == "feather-staged" => {
            feather::run(mode, std::path::Path::new(path), count.parse()?)
        }
        _ => bail!(
            "usage: nautilus-bench allocator-check | feather-direct DIR ROWS | feather-staged DIR ROWS"
        ),
    }
}
