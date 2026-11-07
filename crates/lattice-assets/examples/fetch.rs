//! Downloads and verifies the client jar into the given cache directory.
fn main() {
    let dir = std::env::args().nth(1).expect("cache dir");
    let jar = lattice_assets::ensure_client_jar(std::path::Path::new(&dir), &|p| {
        if p.done % (4 << 20) < 65536 {
            eprintln!("{} / {:?}", p.done, p.total);
        }
    })
    .unwrap();
    let r = lattice_assets::Resources::open(&jar).unwrap();
    println!("blockstates: {}", r.list("blockstates/", ".json").len());
    println!("block models: {}", r.list("models/block/", ".json").len());
    println!("block textures: {}", r.list("textures/block/", ".png").len());
    println!("item definitions: {}", r.list("items/", ".json").len());
    println!("translations: {}", r.translations("en_us").len());
}
