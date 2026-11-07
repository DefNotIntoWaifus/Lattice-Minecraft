//! Reads the game's keyframe animations from a jar and prints a summary.
//! `cargo run -p lattice-assets --example animations -- path/to/client.jar`

fn main() {
    let jar = std::env::args().nth(1).expect("path to the client jar");
    let started = std::time::Instant::now();
    match lattice_assets::geometry::read_animations(std::path::Path::new(&jar)) {
        Ok(all) => {
            println!("{} animations in {:.2} s", all.len(), started.elapsed().as_secs_f32());
            let mut names: Vec<_> = all.keys().cloned().collect();
            names.sort();
            for n in names {
                let a = &all[&n];
                println!("  {n}: {} s{}, {} channels, {} keys", a.length, if a.looping { " loop" } else { "" }, a.channels.len(), a.channels.iter().map(|c| c.keys.len()).sum::<usize>());
            }
        }
        Err(e) => {
            println!("failed: {e}");
            std::process::exit(1);
        }
    }
}
