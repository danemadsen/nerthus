//! Lists the compute devices Nerthus finds, as Options › Compute would
//! show them; `--benchmark` also scores each one.
//!
//! `cargo run -p nerthus-compute --example devices [-- --benchmark]`

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let (devices, problems) = nerthus_compute::available_devices();
    for device in &devices {
        let flags = match (device.is_fallback_device, device.requires_driver_update) {
            (true, _) => " [fallback]",
            (_, true) => " [cannot compute]",
            _ => "",
        };
        println!("{} ({}) via {}{flags}", device.device_name, device.device_type.human_name(), device.backend_name);
        println!("    {}", device.id);
    }
    if !problems.is_empty() {
        println!("problems:\n{problems}");
    }

    let benchmark = std::env::args().any(|argument| argument == "--benchmark");
    if benchmark {
        println!("benchmarking...");
        let scores = nerthus_compute::scan(|line| println!("    {line}"));
        if let Some(best) = nerthus_compute::best_device(&scores) {
            println!("fastest: {best}");
        }
    }
}
