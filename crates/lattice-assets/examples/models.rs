//! Reads the game's model definitions from a jar and prints what it found.
//! `cargo run -p lattice-assets --example models -- path/to/client.jar [out.json]`

fn main() {
    let jar = std::env::args().nth(1).expect("path to the client jar");
    let started = std::time::Instant::now();
    match lattice_assets::geometry::read_models(std::path::Path::new(&jar)) {
        Ok(models) => {
            println!("{} models in {:.2} s", models.len(), started.elapsed().as_secs_f32());
            let mut names: Vec<_> = models.keys().cloned().collect();
            names.sort();
            for n in names.iter().take(12) {
                let m = &models[n];
                println!("  {n}: {} x {}, {} children", m.tex_size[0], m.tex_size[1], m.root.children.len());
            }
            if let Some(out) = std::env::args().nth(2) {
                fn part(p: &lattice_assets::geometry::Part) -> serde_json::Value {
                    serde_json::json!({
                        "offset": p.offset, "rotation": p.rotation, "scale": p.scale,
                        "cubes": p.cubes.iter().map(|c| serde_json::json!({"tex": c.tex, "origin": c.origin, "size": c.size, "grow": c.grow, "mirror": c.mirror, "tex_scale": c.tex_scale, "faces": c.faces})).collect::<Vec<_>>(),
                        "children": p.children.iter().map(|(k, v)| (k.clone(), part(v))).collect::<serde_json::Map<_, _>>(),
                    })
                }
                let all: serde_json::Map<String, serde_json::Value> = models.iter().map(|(k, m)| (k.clone(), serde_json::json!({"tex_size": m.tex_size, "root": part(&m.root)}))).collect();
                std::fs::write(out, serde_json::to_vec(&all).unwrap()).unwrap();
            }
        }
        Err(e) => {
            println!("failed: {e}");
            std::process::exit(1);
        }
    }
}
